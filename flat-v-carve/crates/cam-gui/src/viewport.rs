use crate::{
    camera::{self, Camera},
    compute::{Scene, SceneMeta, Vertex},
    overlay, pages,
    pick::{self, Picker},
    render, sim,
    stock_preview::PreviewMeta,
    stock_render,
    stock_style::{self, StageIdentity, StockStyle},
    stock_walls::{self, Wall},
};
use egui::Color32;
use std::sync::Arc;
#[path = "viewport_artwork.rs"]
mod artwork;
#[path = "viewport_inspection.rs"]
mod inspection;
#[path = "viewport_knife.rs"]
mod knife;
#[path = "viewport_profile.rs"]
mod profile;
pub use artwork::{ArtworkEvent, ArtworkInteraction};
pub use profile::{ProfileAnchor, ProfileAnchorEvent};

/// Pages fingerprinted per frame while a new scene settles.
const HASH_PAGES_PER_FRAME: usize = 4;
/// Everything the overlay geometry depends on; a change rebuilds it.
type OverlaySignature = (
    u64,
    Option<u32>,
    usize,
    usize,
    i32,
    i32,
    i32,
    u64,
    Option<(u64, u64)>,
);

/// Read-only state snapshot for the browser integration probe. The probe page
/// cannot read the egui canvas, so the running application publishes the few
/// values an automated browser check needs. Nothing in the application reads it.
pub mod probe {
    #[cfg(target_arch = "wasm32")]
    use std::cell::RefCell;
    #[cfg(target_arch = "wasm32")]
    thread_local! {
        static SNAPSHOT: RefCell<String> = const { RefCell::new(String::new()) };
    }
    #[cfg(target_arch = "wasm32")]
    pub fn publish(text: String) {
        SNAPSHOT.with(|snapshot| *snapshot.borrow_mut() = text);
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn publish(_: String) {}
    #[cfg(target_arch = "wasm32")]
    pub fn snapshot() -> String {
        SNAPSHOT.with(|snapshot| snapshot.borrow().clone())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn snapshot() -> String {
        String::new()
    }
}

struct StockView {
    meta: PreviewMeta,
    identity: u64,
    /// Current displayed cells: either a transported checkpoint range in the
    /// payload or a locally re-integrated field for a seek between checkpoints.
    range: std::ops::Range<usize>,
    local: Option<Arc<Vec<u8>>>,
    cell_versions: Arc<Vec<u32>>,
    stats: sim::Stats,
    prefix: usize,
    /// Bytes the last stock response carried and the replay work it caused.
    /// Quoted by the playback bar so the display accounts for its own transfer
    /// instead of leaving the reviewer to guess.
    last_transfer_bytes: usize,
    last_replayed: usize,
}

/// Stage buttons the playback bar lays out at once. Every review build before
/// GUI9 fits inside this window, so the established timeline is unchanged; a
/// job with more stages scrolls the window instead of laying out one widget per
/// stage in the job.
const TIMELINE_WINDOW: usize = 8;

/// Frame intervals kept for the diagnostics percentiles. Long gaps are not
/// sampled: an event-driven application legitimately sleeps between frames, and
/// mixing those with real work frames would make the numbers meaningless.
const FRAME_SAMPLES: usize = 120;
const IDLE_GAP_MS: f64 = 250.;

/// One visible path range of the timeline. `index` is the stage's simulation
/// tool so the overlay can draw the right cutter glyph.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DisplayGroup {
    pub label: String,
    pub jump: String,
    pub start: usize,
    pub end: usize,
    pub tool: usize,
    pub operation: String,
    pub role: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewSettings {
    /// True when the view sits at the historical isometric tilt. Kept for the
    /// saved-view format and the review scenarios; `tilt_deg` is the
    /// authoritative elevation and is absent in views saved before the camera
    /// could tilt freely.
    pub isometric: bool,
    #[serde(default)]
    pub tilt_deg: Option<f32>,
    pub zoom: f32,
    pub yaw: f32,
    /// View-target offset in normalized scene units, as `camera::Camera` uses.
    #[serde(default)]
    pub pan: [f32; 2],
    /// Display-only appearance of the simulated stock (colour mode, ramps,
    /// x-ray, artwork/path toggles). Never part of the job.
    #[serde(default)]
    pub stock_style: StockStyle,
    pub stage: usize,
    pub stock: bool,
    pub prefix: usize,
    #[serde(default)]
    pub inspection_xy: Option<[f64; 2]>,
    /// Display resolution preset. Older saved views fall back to the preset
    /// every review before GUI9 used.
    #[serde(default)]
    pub preset: crate::stock_preview::DisplayPreset,
}
impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            isometric: false,
            tilt_deg: None,
            zoom: 1.,
            yaw: 0.,
            pan: [0., 0.],
            stock_style: StockStyle::default(),
            stage: 0,
            stock: true,
            prefix: 0,
            inspection_xy: None,
            preset: crate::stock_preview::DisplayPreset::Standard,
        }
    }
}
impl ViewSettings {
    pub fn validate(&self) -> Result<(), String> {
        let tilt = self.tilt_deg.unwrap_or(0.);
        if self
            .inspection_xy
            .is_some_and(|xy| !xy.iter().all(|v| v.is_finite()))
            || !self.zoom.is_finite()
            || !(camera::MIN_ZOOM..=camera::MAX_ZOOM).contains(&self.zoom)
            || !self.yaw.is_finite()
            || !tilt.is_finite()
            || !self.pan.iter().all(|v| v.is_finite())
            || !self.stock_style.xray_opacity.is_finite()
            || self
                .stock_style
                .wall_threshold_mm
                .is_some_and(|value| !value.is_finite() || value <= 0.)
            || !self
                .stock_style
                .ramp_top
                .iter()
                .chain(self.stock_style.ramp_bottom.iter())
                .chain(self.stock_style.plain.iter())
                .chain(self.stock_style.plain_wall.iter())
                .chain(self.stock_style.overrides.values().flatten())
                .all(|v| v.is_finite())
            || self.stage > crate::session::MAX_DISPLAY_GROUPS
            || self.prefix > crate::session::MOTION_LIMIT
        {
            return Err("Invalid saved viewport".into());
        }
        Ok(())
    }
}

/// Walls of one displayed state and the key that identifies it: the stock
/// identity, the playhead prefix and the threshold the style asked for.
struct WallCache {
    identity: u64,
    prefix: usize,
    threshold: u32,
    /// `None` for the ordinary step walls, `Some(along_x)` for a section.
    section: Option<bool>,
    walls: Arc<Vec<Wall>>,
    revision: u64,
}

pub struct Viewport {
    knife_chains: Arc<Vec<crate::knife::Chain>>,
    /// Closed catalogue contours a profile operation can select, in document
    /// order. Display/candidate data only; the document owns the selection.
    profile_contours: Arc<Vec<crate::profile::Contour>>,
    /// One timeline entry per executed stage (or operation), in plan order.
    groups: Arc<Vec<DisplayGroup>>,
    /// Whether the operation the user is editing is a drag knife. Picking
    /// behavior follows the operation, not the whole scene.
    knife_selected: bool,
    /// Whether the operation being edited is a profile: viewport selection
    /// then addresses closed contours with their advisory sides.
    profile_selected: bool,
    /// Candidate anchors of the selected profile operation (document-derived
    /// display state) and their drag gesture.
    profile_anchors: Vec<profile::ProfileAnchor>,
    profile_anchor_signature: u64,
    anchor_drag: Option<profile::AnchorDrag>,
    profile_anchor_events: Vec<profile::ProfileAnchorEvent>,
    pub artwork: ArtworkInteraction,
    pub stock_loading: bool,
    pub result_current: bool,
    inspection: inspection::Inspection,
    requested_stock: Option<usize>,
    /// Prefix a pending stock request asked for. Retained (unlike
    /// `requested_stock`, which the application consumes when it submits the
    /// command) so the display can say which position it is still showing.
    requested_prefix: Option<usize>,
    /// Display resolution the user asked for, waiting for the current command
    /// to finish before the application submits it.
    requested_preset: Option<crate::stock_preview::DisplayPreset>,
    pub status: String,
    scene: Option<Scene>,
    // Viewport and selection
    /// The orbit camera shared by the renderer, the picker and every overlay.
    /// `aspect` is refreshed from the viewport rect on each use.
    camera: Camera,
    /// Display-only stock appearance. Resolved into a uniform and a palette
    /// when the stock callback is built; never sent to the planner.
    stock_style: StockStyle,
    /// Stage identity table of the displayed plan, in the index space the
    /// field stores per cell.
    stages: Arc<Vec<StageIdentity>>,
    /// Tool ids by simulation tool index, for `ByTool` colours.
    tool_ids: Arc<Vec<String>>,
    /// Palette cache, rebuilt when the style or the stage identities change.
    palette: Option<Arc<Vec<[f32; 4]>>>,
    /// Revision the renderer keys its palette upload on.
    palette_revision: u64,
    /// Walls of the displayed field state, with the key they were built for.
    wall_cache: Option<WallCache>,
    /// Revision the renderer keys its wall upload on.
    wall_revision: u64,
    /// What the wall budget policy had to give up for the displayed state:
    /// `(dropped steps, threshold used)`. Reported, never hidden.
    wall_report: Option<(usize, f64)>,
    playhead: usize,
    playing: bool,
    stage: usize,
    /// First stage index the playback bar lays out, and how many rows it laid
    /// out last frame. The count is published as measurement evidence.
    stage_window: usize,
    timeline_rows: usize,
    picker: Option<Picker>,
    picker_build_ms: f64,
    pub selection: Option<pick::Pick>,
    pub pick_tolerance_px: f32,
    overlay_lines: Arc<Vec<Vertex>>,
    overlay_triangles: Arc<Vec<Vertex>>,
    overlay_revision: u64,
    overlay_signature: Option<OverlaySignature>,
    // Paged transport and residency
    page_hashes: Arc<Vec<Option<u64>>>,
    hash_cursor: usize,
    page_budget: u64,
    required_pages: Vec<usize>,
    pub hash_ms: f64,
    // Stock display
    stock: Option<StockView>,
    stock_prefix: usize,
    show_stock: bool,
    // Risk probes
    gpu_unavailable: bool,
    drill: render::Drill,
    gpu: bool,
    last_frame: Option<f64>,
    /// Intervals between frames that did real work, for the diagnostics view.
    frame_ms: Vec<f64>,
    frames_seen: u64,
    /// Adapter identity, so the diagnostics state which backend they measured.
    backend: String,
    /// CPU display memory the worker reported for the current execution: the
    /// live simulation field, its seeded checkpoint copies and the decoded
    /// motion stream. The plan's budget names CPU field storage, so it is
    /// counted rather than left implicit.
    worker_field_bytes: usize,
    worker_checkpoint_bytes: usize,
    worker_motion_bytes: usize,
    pub render_stats: render::SharedStats,
    pub stock_stats: stock_render::SharedStats,
    scene_revision: u64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            knife_chains: Arc::new(Vec::new()),
            profile_contours: Arc::new(Vec::new()),
            groups: Arc::new(Vec::new()),
            knife_selected: false,
            profile_selected: false,
            profile_anchors: Vec::new(),
            profile_anchor_signature: 0,
            anchor_drag: None,
            profile_anchor_events: Vec::new(),
            artwork: ArtworkInteraction::default(),
            stock_loading: false,
            result_current: false,
            inspection: Default::default(),
            requested_stock: None,
            requested_prefix: None,
            requested_preset: None,
            status: "Open a job to begin.".into(),
            scene: None,
            camera: Camera::default(),
            stock_style: StockStyle::default(),
            stages: Arc::new(Vec::new()),
            tool_ids: Arc::new(Vec::new()),
            palette: None,
            palette_revision: 0,
            wall_cache: None,
            wall_revision: 0,
            wall_report: None,
            playhead: 0,
            playing: false,
            stage: 0,
            stage_window: 0,
            timeline_rows: 0,
            picker: None,
            picker_build_ms: 0.,
            selection: None,
            pick_tolerance_px: 8.,
            overlay_lines: Arc::new(Vec::new()),
            overlay_triangles: Arc::new(Vec::new()),
            overlay_revision: 0,
            overlay_signature: None,
            page_hashes: Arc::new(Vec::new()),
            hash_cursor: 0,
            page_budget: render::DEFAULT_PAGE_BUDGET,
            required_pages: Vec::new(),
            hash_ms: 0.,
            stock: None,
            stock_prefix: 0,
            show_stock: true,
            gpu_unavailable: false,
            drill: render::Drill::None,
            gpu: false,
            last_frame: None,
            frame_ms: Vec::new(),
            frames_seen: 0,
            backend: "not initialized".into(),
            worker_field_bytes: 0,
            worker_checkpoint_bytes: 0,
            worker_motion_bytes: 0,
            render_stats: Arc::new(std::sync::Mutex::new(render::Stats::default())),
            stock_stats: Arc::new(std::sync::Mutex::new(stock_render::Stats::default())),
            scene_revision: 0,
        }
    }
}

impl Viewport {
    /// Record the CPU display memory the worker reported with a scene or stock
    /// response. A response that carries none leaves the previous numbers: an
    /// unknown value must not silently read as zero.
    fn adopt_display_memory(&mut self, value: &serde_json::Value) {
        let field = value["fieldBytes"].as_u64();
        let checkpoints = value["checkpointBytes"].as_u64();
        let motions = value["motionBytes"].as_u64();
        if let Some(field) = field {
            self.worker_field_bytes = field as usize;
        }
        if let Some(checkpoints) = checkpoints {
            self.worker_checkpoint_bytes = checkpoints as usize;
        }
        if let Some(motions) = motions {
            self.worker_motion_bytes = motions as usize;
        }
    }
    /// The saved view is the camera plus the display decisions around it. The
    /// isometric flag stays in the format for older readers and the review
    /// scenarios; the elevation itself travels in `tilt_deg`.
    pub fn settings(&self) -> ViewSettings {
        ViewSettings {
            isometric: (self.camera.tilt - camera::ISO_TILT).abs() < 1e-3,
            tilt_deg: Some(self.camera.tilt.to_degrees()),
            zoom: self.camera.zoom,
            yaw: self.camera.yaw,
            pan: self.camera.pan,
            stock_style: self.stock_style.clone(),
            stage: self.stage,
            stock: self.show_stock,
            prefix: self.stock_prefix,
            inspection_xy: self.inspection.point,
            // Persist the user's chosen resolution, not just what happens to be
            // displayed: a saved context that has not generated yet must keep
            // the choice it was made with.
            preset: self.desired_preset(),
        }
    }
    pub fn restore_settings(&mut self, settings: &ViewSettings) {
        self.inspection.point = settings.inspection_xy;
        // A view saved before free rotation carries only the isometric flag.
        let tilt = settings
            .tilt_deg
            .map(f32::to_radians)
            .unwrap_or(if settings.isometric {
                camera::ISO_TILT
            } else {
                0.
            });
        let mut camera = Camera {
            yaw: settings.yaw,
            ..Camera::default()
        };
        camera.pan = settings.pan;
        camera.set_tilt(tilt);
        camera.set_zoom(settings.zoom);
        self.camera = camera;
        self.stock_style = settings.stock_style.clone();
        self.palette = None;
        self.palette_revision = self.palette_revision.wrapping_add(1);
        self.stage = settings.stage;
        self.show_stock = settings.stock;
        self.playing = false;
        self.requested_stock = None;
        self.requested_prefix = None;
        // A restored view keeps its resolution: the stored preset is requested
        // once the application is idle, exactly like a user's own choice.
        self.requested_preset = Some(settings.preset);
        self.stage_window = 0;
    }
    /// Adopt a compute result: metadata plus its binary payload.
    pub fn load_scene(&mut self, result: Result<(SceneMeta, Vec<u8>), String>) {
        match result {
            Ok((meta, payload)) => self.set_scene(Scene {
                meta,
                payload: Arc::new(payload),
            }),
            Err(error) => self.status = error,
        }
    }

    pub fn set_scene(&mut self, scene: Scene) {
        self.adopt_display_memory(&scene.meta.report["gui2"]["displayMemory"]);
        self.knife_chains = Arc::new(
            serde_json::from_value(scene.meta.report["gui2"]["chains"].clone()).unwrap_or_default(),
        );
        self.profile_contours = Arc::new(
            serde_json::from_value(scene.meta.report["gui2"]["profileContours"].clone())
                .unwrap_or_default(),
        );
        self.groups = Arc::new(
            serde_json::from_value(scene.meta.report["gui2"]["groups"].clone()).unwrap_or_default(),
        );
        // Stage identity for the colour modes: the index the field stores per
        // cell, with the operation and tool ids a palette keys on.
        let stages: Vec<StageIdentity> = scene.meta.report["gui2"]["stages"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(StageIdentity {
                            index: entry["index"].as_u64()? as u16,
                            operation: entry["operation"].as_str()?.to_string(),
                            tool: entry["tool"].as_u64()? as usize,
                            tool_id: entry["toolId"].as_str().unwrap_or_default().to_string(),
                            role: entry["role"].as_str().unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let tool_ids = stage_tool_ids(&stages);
        if self.stages != Arc::new(stages.clone()) || self.tool_ids != Arc::new(tool_ids.clone()) {
            self.palette_revision = self.palette_revision.wrapping_add(1);
        }
        self.stages = Arc::new(stages);
        self.tool_ids = Arc::new(tool_ids);
        self.palette = None;
        if self.stage > self.groups.len() {
            self.stage = 0;
        }
        self.playhead = scene.motion_count();
        self.stock_prefix = scene.motion_count();
        self.requested_prefix = None;
        self.stage_window = 0;
        self.timeline_rows = 0;
        self.selection = None;
        self.picker = None;
        self.overlay_signature = None;
        let stock = self.build_stock(&scene);
        self.stock = stock;
        self.settle_preset_request(self.display_preset());
        self.page_hashes = Arc::new(vec![None; page_table(&scene).page_count()]);
        self.hash_cursor = 0;
        let required = self.compute_required(&scene);
        self.required_pages = required;

        self.playing = false;
        self.scene_revision += 1;
        self.scene = Some(scene);
        if let Some(prefix) = self.comparison_prefix() {
            self.stock_seek(prefix);
        }
    }

    fn build_stock(&mut self, scene: &Scene) -> Option<StockView> {
        let meta = scene.meta.stock.clone()?;
        // The stock tiles are keyed by the simulation key, not by the scene
        // payload: geometry pages and raster checkpoints have different
        // lifetimes, and a preset change must invalidate only the raster.
        let identity = meta.identity();
        let ranges: Vec<_> = scene
            .stock_sections()
            .map(|section| section.offset..section.offset + section.len)
            .collect();
        if ranges.len() != meta.frames.len() {
            self.status = "Stock checkpoint sections do not match the metadata".into();
            return None;
        }
        let versions: Vec<Arc<Vec<u32>>> = meta
            .frames
            .iter()
            .map(|frame| Arc::new(frame.versions.clone()))
            .collect();
        let cells = ranges.last()?.clone();
        let stats = meta.frames.last()?.stats.clone();
        let prefix = meta.frames.last()?.prefix;
        Some(StockView {
            cell_versions: versions[versions.len() - 1].clone(),
            meta,
            identity,
            range: cells,
            local: None,
            stats,
            prefix,
            last_transfer_bytes: 0,
            last_replayed: 0,
        })
    }

    pub fn new_viewer(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        if let Some(state) = &cc.wgpu_render_state {
            let mut renderer = state.renderer.write();
            renderer.callback_resources.insert(render::Resources::new(
                &state.device,
                state.target_format,
                app.render_stats.clone(),
            ));
            renderer
                .callback_resources
                .insert(stock_render::Resources::new(
                    &state.device,
                    state.target_format,
                    app.stock_stats.clone(),
                ));
            drop(renderer);
            app.gpu = true;
            app.backend = format!("{:?}", state.adapter.get_info());
            app.status = format!("Renderer · {:?}", state.adapter.get_info());
        }
        app
    }

    fn draw_workspace(&self, ui: &egui::Ui, rect: egui::Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let camera = self.camera(rect);
        let project = |p: [f32; 3]| {
            let xy = camera.to_points(camera.ndc(p), [rect.width(), rect.height()]);
            rect.center() + egui::vec2(xy[0], xy[1])
        };
        for i in -20..=20 {
            let n = i as f32 / 10.;
            let stroke = egui::Stroke::new(
                1.,
                if i == 0 {
                    Color32::from_rgb(207, 217, 223)
                } else {
                    Color32::from_rgb(232, 237, 240)
                },
            );
            painter.line_segment([project([n, -2., 0.]), project([n, 2., 0.])], stroke);
            painter.line_segment([project([-2., n, 0.]), project([2., n, 0.])], stroke);
        }
        if self.stock.is_none()
            && let Some(scene) = &self.scene
            && let Some(xy) = scene.meta.report["gui2"]["stockRect"].as_array()
            && xy.len() == 5
        {
            let values: Vec<f64> = xy.iter().filter_map(|n| n.as_f64()).collect();
            if values.len() != 5 {
                return;
            }
            let b = scene.meta.bounds;
            let scale = 1.6 / (b[2] - b[0]).max(b[3] - b[1]).max(0.001);
            let corners = [
                [values[0], values[1]],
                [values[0] + values[2], values[1]],
                [values[0] + values[2], values[1] + values[3]],
                [values[0], values[1] + values[3]],
            ];
            let points = |z: f64| {
                corners.map(|p| {
                    project([
                        ((p[0] - (b[0] + b[2]) / 2.) * scale) as f32,
                        ((p[1] - (b[1] + b[3]) / 2.) * scale) as f32,
                        (z * scale) as f32,
                    ])
                })
            };
            let top = points(0.);
            let bottom = points(-values[4]);
            // The side faces are only visible once the view is off the plane.
            if self.camera.tilt.sin().abs() > 1e-3 {
                for i in 0..4 {
                    let j = (i + 1) % 4;
                    painter.add(egui::Shape::convex_polygon(
                        vec![top[i], top[j], bottom[j], bottom[i]],
                        Color32::from_rgb(170, 143, 104),
                        egui::Stroke::NONE,
                    ));
                }
            }
            painter.add(egui::Shape::convex_polygon(
                top.to_vec(),
                Color32::from_rgb(215, 190, 150),
                egui::Stroke::new(1., Color32::from_rgb(175, 153, 116)),
            ));
        }
    }
    fn viewport(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Presets set the elevation only. The azimuth, the elevation
                // and the pan stay free afterwards, from the pointer or from
                // these controls.
                for (label, tilt) in [("Top", 0.), ("Isometric", camera::ISO_TILT)] {
                    let selected = (self.camera.tilt - tilt).abs() < 1e-3;
                    let response = ui.selectable_label(selected, label);
                    crate::app::observe_control(label, response.rect);
                    if response.clicked() {
                        self.camera.set_tilt(tilt);
                    }
                }
                // True elevations set both angles: the view faces a stock face
                // exactly, which the stock pass then renders as a section.
                for (label, yaw) in camera::ELEVATIONS {
                    let selected =
                        self.camera.is_elevation() && (self.camera.yaw - yaw).abs() < 1e-3;
                    let response = ui.selectable_label(selected, label);
                    crate::app::observe_control(label, response.rect);
                    if response.clicked() {
                        self.camera.yaw = yaw;
                        self.camera.set_tilt(camera::TILT_LIMIT);
                    }
                }
                let fit = ui.button("Fit");
                crate::app::observe_control("Fit", fit.rect);
                if fit.clicked() {
                    self.fit();
                }
                let mut tilt_deg = self.camera.tilt.to_degrees();
                let view = ui.add(
                    egui::Slider::new(&mut tilt_deg, -85.0..=85.0)
                        .suffix("°")
                        .text("View"),
                );
                crate::app::observe_control("View", view.rect);
                if view.changed() {
                    self.camera.set_tilt(tilt_deg.to_radians());
                }
                let mut zoom = self.camera.zoom;
                let zoom_control = ui.add(
                    egui::Slider::new(&mut zoom, camera::MIN_ZOOM..=camera::MAX_ZOOM)
                        .logarithmic(true)
                        .text("Zoom"),
                );
                crate::app::observe_control("Zoom", zoom_control.rect);
                if zoom_control.changed() {
                    self.camera.set_zoom(zoom);
                }
                if self.stock.is_some() {
                    ui.checkbox(&mut self.show_stock, "Stock preview");
                }
                crate::app::help::icon(ui, "View controls");
            });
            // Display-only stock appearance: colour mode, appearance and the
            // layer toggles. None of it reaches the job.
            ui.horizontal(|ui| {
                let mut surface = self.stock_style.surface;
                egui::ComboBox::from_id_salt("stock-color-mode")
                    .selected_text(surface.label())
                    .show_ui(ui, |ui| {
                        for mode in stock_style::ColorMode::ALL {
                            ui.selectable_value(&mut surface, mode, mode.label());
                        }
                    });
                if surface != self.stock_style.surface {
                    self.stock_style.surface = surface;
                    self.palette = None;
                }
                let mut appearance = self.stock_style.appearance;
                let solid =
                    ui.selectable_value(&mut appearance, stock_style::Appearance::Opaque, "Solid");
                crate::app::observe_control("Solid stock", solid.rect);
                let xray =
                    ui.selectable_value(&mut appearance, stock_style::Appearance::XRay, "X-ray");
                crate::app::observe_control("X-ray stock", xray.rect);
                if appearance != self.stock_style.appearance {
                    self.stock_style.appearance = appearance;
                }
                if self.stock_style.appearance == stock_style::Appearance::XRay {
                    let mut opacity = self.stock_style.xray_opacity;
                    if ui
                        .add(egui::Slider::new(&mut opacity, 0.05..=1.).text("Opacity"))
                        .changed()
                    {
                        self.stock_style.xray_opacity = opacity;
                    }
                }
                let mut walls = self.stock_style.walls;
                egui::ComboBox::from_id_salt("stock-wall-mode")
                    .selected_text(walls.label())
                    .show_ui(ui, |ui| {
                        for mode in stock_style::WallMode::ALL {
                            ui.selectable_value(&mut walls, mode, mode.label());
                        }
                    });
                if walls != self.stock_style.walls {
                    self.stock_style.walls = walls;
                }
                if ui
                    .checkbox(&mut self.stock_style.show_artwork, "Artwork")
                    .changed()
                {
                    self.overlay_signature = None;
                }
                ui.checkbox(&mut self.stock_style.show_paths, "Paths");
            });
            self.artwork_toolbar(ui);
            ui.label(
                self.scene
                    .as_ref()
                    .map(|scene| scene.meta.name.as_str())
                    .unwrap_or("Import an SVG to start your carving"),
            );
            let (rect, response) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            crate::app::observe_control("Artwork viewport", rect);
            ui.painter()
                .rect_filled(rect, 0., Color32::from_rgb(246, 248, 250));
            self.draw_workspace(ui, rect);
            self.artwork_pointer(ui, &response, rect);
            self.profile_anchor_pointer(ui, &response, rect);
            // View navigation. Active edit handles and the selectable geometry
            // kinds own their drags first (UI plan section 9.3): the artwork
            // placement gesture and the profile anchor keep the primary
            // button, so an active gesture is never stolen. Primary (or any
            // secondary) drag orbits; the middle button, or Shift with the
            // primary button, pans — and the middle button and the wheel stay
            // available for navigation in every mode.
            let edit_gesture = !self.artwork.enabled
                || self.artwork.mode == crate::artwork_view::GestureMode::Select;
            if self.anchor_drag.is_none() {
                let delta = response.drag_delta();
                let shift = ctx.input(|input| input.modifiers.shift);
                let pan = response.dragged_by(egui::PointerButton::Middle)
                    || (edit_gesture && shift && response.dragged_by(egui::PointerButton::Primary));
                let orbit = edit_gesture
                    && (response.dragged_by(egui::PointerButton::Primary)
                        || response.dragged_by(egui::PointerButton::Secondary));
                if pan {
                    self.camera
                        .pan_by([delta.x, delta.y], [rect.width(), rect.height()]);
                } else if orbit && delta != egui::Vec2::ZERO {
                    self.camera.orbit([delta.x, delta.y]);
                }
            }
            // The wheel (or a pinch / ctrl-wheel gesture) zooms about the
            // cursor, so the point under it stays put.
            if response.hovered() {
                let (scroll, pinch) =
                    ctx.input(|input| (input.raw_scroll_delta.y, input.zoom_delta()));
                let factor = if (pinch - 1.).abs() > 1e-4 {
                    pinch
                } else {
                    (scroll * 0.0025).exp()
                };
                if (factor - 1.).abs() > 1e-4 {
                    let cursor = response.hover_pos().unwrap_or_else(|| rect.center());
                    self.zoom_at(cursor, rect, factor.clamp(0.2, 5.));
                }
            }
            if !self.artwork.enabled
                && response.clicked()
                && let Some(position) = response.interact_pointer_pos()
            {
                self.pick_at(position, rect, ctx.pixels_per_point());
                if let Some(scene) = &self.scene {
                    let camera = self.camera(rect);
                    // A true elevation has no ground plane: the pointer lands on
                    // the plane the viewer faces, so the click moves the
                    // section along the axis the view shows and keeps the other
                    // coordinate where it was.
                    if let Some(point) = crate::artwork_view::elevation_setup_point(
                        camera,
                        scene.meta.bounds,
                        rect,
                        position,
                    ) {
                        let mut inspection = self.inspection.point.unwrap_or([point.x, point.y]);
                        if camera.section_along_x() {
                            inspection[0] = point.x;
                        } else {
                            inspection[1] = point.y;
                        }
                        self.inspection.point = Some(inspection);
                    } else {
                        let p = crate::artwork_view::setup_point(
                            camera,
                            scene.meta.bounds,
                            rect,
                            position,
                        );
                        self.inspection.point = Some([p.x, p.y]);
                    }
                }
            }
            self.build_overlay(rect, ctx.pixels_per_point());
            // Style inputs are resolved before the scene borrow: the palette
            // is cached and only rebuilt when the style or the plan changes.
            let (style_uniform, palette, palette_revision) = self.stock_style_inputs(rect);
            let (walls, wall_revision) = self.stock_walls(rect);
            if self.gpu_unavailable {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Injected renderer failure\nDraft and prior result retained",
                    egui::FontId::proportional(18.),
                    Color32::LIGHT_RED,
                );
            } else if self.gpu
                && let Some(scene) = &self.scene
            {
                // Artwork and paths are drawn first and the stock last. In the
                // opaque appearance that changes nothing visible (geometry
                // above the surface keeps the nearer depth it already wrote);
                // in the x-ray appearance it is what lets a path inside a cut
                // stay readable through the material.
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        render::Callback {
                            payload: scene.payload.clone(),
                            identity: scene.identity(),
                            table: page_table(scene),
                            hashes: self.page_hashes.clone(),
                            required: self.required_pages.clone(),
                            // Path display is a draw-range decision: an empty
                            // span hides the paths without touching the plan.
                            visible: if self.stock_style.show_paths {
                                self.visible_motion_range()
                            } else {
                                0..0
                            },
                            budget_bytes: self.page_budget,
                            contour_vertices: scene.meta.contour_vertices,
                            // Hidden artwork draws no range at all; the
                            // fallback below is for a scene that carries no
                            // per-item spans, not for the toggle.
                            contour_draw_ranges: if !self.stock_style.show_artwork {
                                Vec::new()
                            } else {
                                scene.meta.report["gui2"]["artworkSpans"]
                                    .as_array()
                                    .map(|spans| {
                                        spans
                                            .iter()
                                            .filter(|s| {
                                                !self
                                                    .artwork
                                                    .hidden
                                                    .contains(s[0].as_str().unwrap_or(""))
                                            })
                                            .filter_map(|s| {
                                                Some(s[1].as_u64()? as u32..s[2].as_u64()? as u32)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_else(|| {
                                        std::iter::once(0..scene.meta.contour_vertices as u32)
                                            .collect()
                                    })
                            },
                            contour_range: scene
                                .meta
                                .sections
                                .iter()
                                .find(|s| s.kind == pages::SECTION_CONTOUR)
                                .map(|s| s.offset..s.offset + s.len)
                                .unwrap_or(0..0),
                            revision: self.overlay_revision,
                            camera: self.camera(rect).uniform(),
                            lines: self.overlay_lines.clone(),
                            triangles: self.overlay_triangles.clone(),
                            drill: self.drill,
                        },
                    ));
                if self.show_stock
                    && let Some(stock) = &self.stock
                {
                    let bounds = scene.meta.bounds;
                    let size = (bounds[2] - bounds[0])
                        .max(bounds[3] - bounds[1])
                        .max(0.001);
                    let scale = 1.6 / size;
                    ui.painter()
                        .add(eframe::egui_wgpu::Callback::new_paint_callback(
                            rect,
                            stock_render::Callback {
                                payload: match stock.local {
                                    None => Some(scene.payload.clone()),
                                    Some(_) => None,
                                },
                                cells: stock.range.clone(),
                                local: stock.local.clone(),
                                cols: stock.meta.cols,
                                rows: stock.meta.rows,
                                tiles_x: stock.meta.tiles_x,
                                tiles_y: stock.meta.tiles_y,
                                versions: stock.cell_versions.clone(),
                                identity: stock.identity,
                                revision: self.scene_revision * 4096 + stock.prefix as u64,
                                camera: self.camera(rect).uniform(),
                                grid: [
                                    ((stock.meta.stock.x0 - (bounds[0] + bounds[2]) / 2.) * scale)
                                        as f32,
                                    ((stock.meta.stock.y0 - (bounds[1] + bounds[3]) / 2.) * scale)
                                        as f32,
                                    (stock.meta.cell_mm * scale) as f32,
                                    (stock.meta.stock.thickness_mm * scale) as f32,
                                    stock.meta.cols as f32,
                                    stock.meta.rows as f32,
                                    ((stock.meta.stock.x1 - stock.meta.stock.x0) * scale) as f32,
                                    ((stock.meta.stock.y1 - stock.meta.stock.y0) * scale) as f32,
                                    stock.meta.tiles_x as f32,
                                    walls.len() as f32,
                                ],
                                style: style_uniform,
                                palette: palette.clone(),
                                palette_revision,
                                walls: walls.clone(),
                                wall_revision,
                                drill: stock_drill(self.drill),
                            },
                        ));
                }
            }
            self.artwork_overlay(ui, rect);
            self.inspection_marker(ui, rect);
            let pick = self
                .selection
                .map(|pick| {
                    format!(
                        " · picked motion {} at {:.1} px",
                        pick.motion, pick.distance_pixels
                    )
                })
                .unwrap_or_default();
            ui.painter().text(
                rect.left_bottom() + egui::vec2(12., -12.),
                egui::Align2::LEFT_BOTTOM,
                format!("{} motions{}", self.motion_count(), pick),
                egui::FontId::monospace(11.),
                Color32::GRAY,
            );
        });
    }

    /// Shared production viewport and cumulative stock transport.
    pub fn show(&mut self, ctx: &egui::Context, simulate: bool) {
        self.fingerprint_pages();
        if simulate {
            egui::TopBottomPanel::bottom("gui2-playback").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let play=ui.button(if self.playing {"Pause"} else {"Play"});crate::app::observe_control(if self.playing {"Pause"} else {"Play"},play.rect);if play.clicked() { self.playing = !self.playing; if self.playing && self.stock_prefix==self.motion_count(){self.stock_seek(0);} }
                let start=ui.button("Start");crate::app::observe_control("Start",start.rect);if start.clicked() {self.playing=false;self.stock_seek(0);}
                let groups = self.groups.clone();
                let (first, last) = self.timeline_window(groups.len());
                self.timeline_rows = 0;
                if first > 0 {
                    let earlier = ui.button(format!("◀ {first} earlier stages"));
                    crate::app::observe_control("Earlier stages", earlier.rect);
                    if earlier.clicked() {
                        self.stage_window = first.saturating_sub(TIMELINE_WINDOW);
                    }
                }
                for group in &groups[first..last] {
                    let target = group.end;
                    let response = ui.button(&group.jump);
                    crate::app::observe_control(&group.jump, response.rect);
                    self.timeline_rows += 1;
                    if response.clicked() {
                        self.playing = false;
                        self.stock_seek(target);
                    }
                }
                if last < groups.len() {
                    let later = ui.button(format!("{} later stages ▶", groups.len() - last));
                    crate::app::observe_control("Later stages", later.rect);
                    if later.clicked() {
                        self.stage_window = last.min(groups.len().saturating_sub(1));
                    }
                }
                if self.knife_selected {
                    ui.label("Knife traces keep the preceding stock; pivot paths are paged.");
                }
                let selected = groups
                    .get(self.stage.wrapping_sub(1))
                    .map(|group| group.label.clone())
                    .unwrap_or_else(|| "All paths".into());
                egui::ComboBox::from_id_salt("visible-path-stage")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.stage, 0, "All paths");
                        for (index, group) in groups.iter().enumerate() {
                            ui.selectable_value(&mut self.stage, index + 1, &group.label);
                        }
                    });
                // Display resolution. It changes only the raster the simulator
                // shows; the plan, its motions and every machining value stay.
                let current = self.display_preset();
                let combo = egui::ComboBox::from_id_salt("display-resolution")
                    .selected_text(format!("Display: {}", current.label()))
                    .show_ui(ui, |ui| {
                        for preset in crate::stock_preview::DisplayPreset::ALL {
                            let response = ui.selectable_label(
                                preset == current,
                                format!("{} · {}", preset.label(), preset.summary()),
                            );
                            crate::app::observe_control(
                                &format!("Display resolution {}", preset.label()),
                                response.rect,
                            );
                            if response.clicked() {
                                self.set_display_preset(preset);
                            }
                        }
                    });
                crate::app::observe_control("Display resolution", combo.response.rect);
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width=(ui.available_width()-160.).max(80.);
                let mut prefix=self.stock_prefix;
                let slider=ui.add(egui::Slider::new(&mut prefix,0..=self.motion_count()).text("Stock motion"));crate::app::observe_control("Stock motion",slider.rect);if slider.changed(){self.playing=false;self.stock_seek(prefix);}
            });
            if let Some(stock)=&self.stock {
                ui.label(format!("Display simulation · {:.4} mm cells (reference {:.4}) · {} / {} motions · {:.2} mm³ removed",stock.meta.cell_mm,stock.meta.reference_cell_mm,stock.prefix,self.motion_count(),stock.stats.removed_volume_mm3));
                if let Some(transport)=self.scene.as_ref().map(|scene|&scene.meta.transport) {
                    let (bytes,replayed)=self.last_stock_transfer();
                    let checkpoints=stock.meta.ladder_frames.max(transport.stock_checkpoints);
                    ui.label(format!("Display transfer · scene {:.1} MiB · {} motion pages · {} checkpoints ({:.1} MiB retained) · last stock update {:.0} KiB after replaying {} motions",transport.payload_bytes as f64/1048576.,transport.motion_pages,checkpoints,stock.meta.retained_bytes as f64/1048576.,bytes as f64/1024.,replayed));
                }
                // A path page the resident budget refused is not drawn. Say so
                // instead of letting a partial scene look complete.
                if let Ok(stats) = self.render_stats.lock()
                    && stats.budget_omitted > 0
                {
                    let omitted = stats.budget_omitted;
                    let response = ui.colored_label(
                        Color32::from_rgb(164, 83, 12),
                        format!(
                            "{omitted} requested path page(s) are outside the resident budget and are not drawn; the stock field and the timeline are unaffected."
                        ),
                    );
                    crate::app::observe_control("Path pages omitted", response.rect);
                }
                if let Some(target)=self.requested_prefix {
                    ui.label(format!("Requested stock motion {target} of {} — still showing motion {}.", self.motion_count(), self.stock_prefix));
                }
                if stock.meta.dropped_stage_marks > 0 {
                    let response = ui.colored_label(Color32::from_rgb(164,83,12), format!("{} stage boundaries are beyond the display checkpoint budget; the timeline still seeks them by replaying from the nearest earlier checkpoint.", stock.meta.dropped_stage_marks));
                    crate::app::observe_control("Dropped stage boundaries", response.rect);
                }
            }
        });
        } else {
            self.playing = false;
        }
        self.viewport(ctx);
        // The recovery drill is one frame: the callbacks captured it when they
        // were built above, and painting happens after this returns.
        self.drill = render::Drill::None;
        // A bounded upload that did not finish this frame asks for the next
        // one. Nothing is dropped: the deferred pages and tiles keep their
        // fingerprints and are copied when the next frame runs.
        let loading = self
            .render_stats
            .lock()
            .is_ok_and(|stats| stats.pages_deferred > 0)
            || self
                .stock_stats
                .lock()
                .is_ok_and(|stats| stats.pending_tiles > 0);
        if loading && !self.gpu_unavailable {
            ctx.request_repaint();
        }
        let now = ctx.input(|i| i.time);
        if self.playing && !self.stock_loading && self.requested_stock.is_none() {
            let step = (self.motion_count() / 120).max(1);
            self.stock_seek((self.stock_prefix + step).min(self.motion_count()));
            if self.stock_prefix == self.motion_count() {
                self.playing = false;
            }
            ctx.request_repaint();
        }
        // Frame instrumentation: sample the interval since the previous frame,
        // ignoring the legitimate silence between event-driven frames.
        if let Some(previous) = self.last_frame {
            let interval = (now - previous) * 1000.;
            if (0.5..=IDLE_GAP_MS).contains(&interval) {
                if self.frame_ms.len() == FRAME_SAMPLES {
                    self.frame_ms.remove(0);
                }
                self.frame_ms.push(interval);
                self.frames_seen += 1;
            }
        }
        self.last_frame = Some(now);
    }
    pub fn stock_prefix(&self) -> usize {
        self.stock_prefix
    }
    /// Display preset the shown raster was derived from.
    pub fn display_preset(&self) -> crate::stock_preview::DisplayPreset {
        self.stock
            .as_ref()
            .map_or(crate::stock_preview::DisplayPreset::Standard, |stock| {
                stock.meta.preset
            })
    }
    /// The preset a new scene must be built at: the user's pending choice, or
    /// whatever the shown raster already uses.
    pub fn desired_preset(&self) -> crate::stock_preview::DisplayPreset {
        self.requested_preset
            .unwrap_or_else(|| self.display_preset())
    }
    /// The raster's simulation key. A tile or checkpoint from another key must
    /// not be shown even when the stock rectangle matches.
    pub fn simulation_key(&self) -> Option<&str> {
        Some(self.stock.as_ref()?.meta.key.as_str())
    }
    /// Ask for another display resolution. The application submits this once
    /// the current command finishes, exactly like a stock seek.
    pub fn set_display_preset(&mut self, preset: crate::stock_preview::DisplayPreset) {
        let shown = self.stock.as_ref().map(|stock| stock.meta.preset);
        if Some(preset) != shown {
            self.requested_preset = Some(preset);
        }
    }
    pub fn take_preset_request(&mut self) -> Option<crate::stock_preview::DisplayPreset> {
        self.requested_preset.take()
    }
    pub fn requested_display_preset(&self) -> Option<crate::stock_preview::DisplayPreset> {
        self.requested_preset
    }
    /// The request is satisfied once the displayed raster uses that preset; a
    /// newer request keeps waiting.
    fn settle_preset_request(&mut self, shown: crate::stock_preview::DisplayPreset) {
        if self.requested_preset == Some(shown) {
            self.requested_preset = None;
        }
    }
    /// Adopt a raster rebuilt at another display resolution for the same
    /// retained execution. The scene geometry and its pages are untouched; the
    /// stock identity changes, so every resident tile is re-uploaded.
    pub fn adopt_preset(&mut self, meta: SceneMeta, payload: Vec<u8>) -> Result<(), String> {
        self.adopt_display_memory(&meta.report["gui2"]["displayMemory"]);
        let preview = meta.stock.ok_or("Missing stock response")?;
        let frame = preview.frames.first().ok_or("Missing stock frame")?;
        let section = meta
            .sections
            .iter()
            .find(|s| s.kind == pages::SECTION_STOCK)
            .ok_or("Missing stock cells")?;
        let cells = payload
            .get(section.offset..section.offset + section.len)
            .ok_or("Invalid stock payload")?;
        let identity = preview.identity();
        let cell_versions = Arc::new(frame.versions.clone());
        let stats = frame.stats.clone();
        let prefix = frame.prefix;
        self.stock = Some(StockView {
            meta: preview,
            identity,
            range: section.offset..section.offset + section.len,
            local: Some(Arc::new(cells.to_vec())),
            cell_versions,
            stats,
            prefix,
            last_transfer_bytes: payload.len(),
            last_replayed: 0,
        });
        self.stock_prefix = prefix;
        self.playhead = prefix;
        self.requested_prefix = None;
        self.requested_stock = None;
        self.settle_preset_request(self.display_preset());
        self.stock_loading = false;
        Ok(())
    }
    /// Position a pending stock request asked for, while the display still
    /// shows [`Viewport::stock_prefix`].
    pub fn requested_stock_prefix(&self) -> Option<usize> {
        self.requested_prefix
    }
    pub fn take_stock_request(&mut self) -> Option<usize> {
        self.requested_stock.take()
    }
    /// Whether a stock seek is still waiting for the application to submit it.
    pub fn stock_request_pending(&self) -> bool {
        self.requested_stock.is_some()
    }
    pub fn accept_stock(&mut self, meta: SceneMeta, payload: Vec<u8>) -> Result<(), String> {
        self.adopt_display_memory(&meta.report["gui2"]["displayMemory"]);
        let stock = self.stock.as_mut().ok_or("No displayed stock")?;
        let preview = meta.stock.ok_or("Missing stock response")?;
        let frame = preview.frames.first().ok_or("Missing stock frame")?;
        let section = meta
            .sections
            .iter()
            .find(|s| s.kind == pages::SECTION_STOCK)
            .ok_or("Missing stock cells")?;
        let cells = payload
            .get(section.offset..section.offset + section.len)
            .ok_or("Invalid stock payload")?
            .to_vec();
        stock.local = Some(Arc::new(cells));
        stock.cell_versions = Arc::new(frame.versions.clone());
        stock.stats = frame.stats.clone();
        stock.prefix = frame.prefix;
        // The response names its own simulation key. If it differs (a rebuild
        // landed between requests) the renderer must re-upload every tile
        // instead of mixing cells from two resolutions.
        stock.identity = preview.identity();
        stock.last_transfer_bytes = payload.len();
        stock.last_replayed = meta.report["gui2"]["replayed"].as_u64().unwrap_or(0) as usize;
        self.stock_prefix = frame.prefix;
        self.playhead = frame.prefix;
        self.requested_prefix = None;
        self.stock_loading = false;
        Ok(())
    }
    /// A stock request finished, successfully or not. The application calls this
    /// when the command it submitted for a seek completes, so a refused or
    /// failed seek cannot leave "requested" pointing at a position the display
    /// is no longer going to reach.
    pub fn finish_stock_request(&mut self) {
        self.requested_prefix = None;
    }
    /// Bytes carried by the last stock response, for the display's own
    /// transfer accounting.
    pub fn last_stock_transfer(&self) -> (usize, usize) {
        self.stock.as_ref().map_or((0, 0), |stock| {
            (stock.last_transfer_bytes, stock.last_replayed)
        })
    }
    /// Stage rows the playback bar laid out on the last frame. Reported so the
    /// review can see that a large job does not lay out one row per stage.
    pub fn timeline_rows(&self) -> usize {
        self.timeline_rows
    }
    /// Read-only display state for the browser probe: which raster resolution is
    /// on screen, what the plan asked for, and the simulation key that keeps the
    /// two from being confused.
    pub fn display_probe(&self) -> serde_json::Value {
        let Some(stock) = &self.stock else {
            return serde_json::json!({
                "preset": self.desired_preset().wire(),
                "raster": null,
                "style": self.style_probe(),
            });
        };
        serde_json::json!({
            "preset": stock.meta.preset.wire(),
            "requestedPreset": self.requested_preset.map(|preset| preset.wire()),
            "cellMm": stock.meta.cell_mm,
            "referenceCellMm": stock.meta.reference_cell_mm,
            "retainedBytes": stock.meta.retained_bytes,
            "checkpoints": stock.meta.ladder_frames.max(stock.meta.frames.len()),
            "key": stock.meta.key,
            "sectionSamples": self.section_sample_count(),
            "style": self.style_probe(),
        })
    }
    /// Display-only stock appearance, as the review harness sees it.
    fn style_probe(&self) -> serde_json::Value {
        serde_json::json!({
            "surface": self.stock_style.surface.wire(),
            "walls": self.stock_style.walls.wire(),
            "appearance": self.stock_style.appearance.wire(),
            "xrayOpacity": self.stock_style.xray_opacity,
            "showStock": self.stock_style.show_stock,
            "showArtwork": self.stock_style.show_artwork,
            "showPaths": self.stock_style.show_paths,
            "paletteRevision": self.palette_revision,
            "stages": self.stages.len(),
            "wallRevision": self.wall_revision,
            "walls": self.wall_cache.as_ref().map_or(0, |cache| cache.walls.len()),
            "wallThresholdMm": self.wall_report.map(|(_, threshold)| threshold),
            "wallsDropped": self.wall_report.map(|(dropped, _)| dropped),
            // A true elevation draws the section instead of the surface; the
            // probe names which axis the section runs along.
            "section": self.wall_cache.as_ref().and_then(|cache| cache.section).map(
                |along_x| if along_x { "along_x" } else { "along_y" }
            ),
        })
    }
    /// Diagnostics drill: stop drawing through the custom renderer callbacks
    /// and say so. The document, the retained result, the playhead, the raster
    /// resolution and the camera are untouched — only the GPU-side drawing
    /// stops, which is what a lost device or a failed resource looks like from
    /// the application's side.
    pub fn inject_renderer_failure(&mut self) {
        self.gpu_unavailable = true;
        self.status = "Renderer failure injected: viewport callbacks stop drawing. The document, retained result and view are unchanged; use Rebuild renderer resources to recover.".into();
    }
    /// Rebuild every persistent GPU resource from the retained CPU data. The
    /// next frames re-copy the resident motion pages and stock tiles, so the
    /// same document, playhead and display resolution come back.
    pub fn rebuild_renderer_resources(&mut self) {
        self.gpu_unavailable = false;
        self.drill = render::Drill::Recover;
        self.status = "Rebuilding renderer resources from the retained scene and stock…".into();
    }
    /// Renderer state and load progress for the browser probe and the review.
    pub fn renderer_probe(&self) -> serde_json::Value {
        let scene = self.render_stats.lock().ok();
        let stock = self.stock_stats.lock().ok();
        // Memory categories this display is responsible for. The plan's budget
        // covers motion pages, CPU scene caches and stock/checkpoints; the
        // categories a page cannot see (WASM linear memory, GPU totals) are
        // reported as unknown by the caller, never as zero.
        let scene_bytes = self
            .scene
            .as_ref()
            .map_or(0, |scene| scene.meta.transport.payload_bytes);
        let checkpoint_bytes = self
            .stock
            .as_ref()
            .map_or(0, |stock| stock.meta.retained_bytes);
        let stock_buffer_bytes = stock.as_ref().map_or(0, |stats| stats.buffer_bytes);
        let gpu_page_bytes = scene.as_ref().map_or(0, |stats| stats.resident_bytes);
        let worker_field = self.worker_field_bytes as u64;
        let worker_checkpoints = self.worker_checkpoint_bytes as u64;
        let worker_motions = self.worker_motion_bytes as u64;
        let declared_display_bytes = scene_bytes as u64
            + checkpoint_bytes as u64
            + stock_buffer_bytes
            + gpu_page_bytes
            + worker_field
            + worker_checkpoints
            + worker_motions;
        let mut sorted = self.frame_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let percentile = |quantile: f64| {
            if sorted.is_empty() {
                return 0.;
            }
            let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
            sorted[index]
        };
        serde_json::json!({
            "build": {
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": crate::compute::PROTOCOL,
            },
            "backend": self.backend,
            "unavailable": self.gpu_unavailable,
            "gpu": self.gpu,
            "recoveries": scene.as_ref().map_or(0, |stats| stats.recoveries)
                + stock.as_ref().map_or(0, |stats| stats.recoveries),
            "injectedErrors": scene.as_ref().map_or(0, |stats| stats.injected_errors),
            "lastError": scene.as_ref().and_then(|stats| stats.last_error.clone()),
            "pagesDeferred": scene.as_ref().map_or(0, |stats| stats.pages_deferred),
            "framesLoading": scene.as_ref().map_or(0, |stats| stats.frames_loading),
            "pendingTiles": stock.as_ref().map_or(0, |stats| stats.pending_tiles),
            "stockFramesLoading": stock.as_ref().map_or(0, |stats| stats.frames_loading),
            "residentPages": scene.as_ref().map_or(0, |stats| stats.resident_pages),
            "residentBytes": scene.as_ref().map_or(0, |stats| stats.resident_bytes),
            "pagesOmittedByBudget": scene.as_ref().map_or(0, |stats| stats.budget_omitted),
            "memory": {
                "sceneBytes": scene_bytes,
                "checkpointBytes": checkpoint_bytes,
                "gpuPageBytes": gpu_page_bytes,
                "stockTileBytes": stock_buffer_bytes,
                "workerFieldBytes": worker_field,
                "workerCheckpointBytes": worker_checkpoints,
                "workerMotionBytes": worker_motions,
                "declaredDisplayBytes": declared_display_bytes,
                "displayBudgetBytes": crate::render::BROWSER_DISPLAY_BUDGET_BYTES,
                "withinBudget": declared_display_bytes <= crate::render::BROWSER_DISPLAY_BUDGET_BYTES,
            },
            "pageUploads": scene.as_ref().map_or(0, |stats| stats.page_uploads),
            "pagesSkipped": scene.as_ref().map_or(0, |stats| stats.uploads_skipped),
            "uploadBytes": scene.as_ref().map_or(0, |stats| stats.upload_bytes),
            "evictions": scene.as_ref().map_or(0, |stats| stats.evictions),
            "requiredPages": self.required_pages.len(),
            "requiredPageRange": match (self.required_pages.first(), self.required_pages.last()) {
                (Some(first), Some(last)) => serde_json::json!([first, last]),
                _ => serde_json::json!([]),
            },
            "framesSeen": self.frames_seen,
            "frameMs": {
                "samples": sorted.len(),
                "last": self.frame_ms.last().copied().unwrap_or(0.),
                "p50": percentile(0.5),
                "p95": percentile(0.95),
                "max": sorted.last().copied().unwrap_or(0.),
            },
        })
    }
    pub fn seek(&mut self, prefix: usize) {
        self.stock_seek(prefix);
    }
    pub fn motion_count(&self) -> usize {
        self.scene.as_ref().map_or(0, Scene::motion_count)
    }
    pub fn scene_bounds(&self) -> Option<[f64; 4]> {
        self.scene.as_ref().map(|s| s.meta.bounds)
    }

    /// Pages the display needs, nearest the playhead first. The renderer admits
    /// this order until the resident budget is reached.
    fn compute_required(&self, scene: &Scene) -> Vec<usize> {
        let table = page_table(scene);
        if table.motions == 0 {
            return Vec::new();
        }
        let playhead = self.playhead.min(table.motions);
        let range = visible_range(scene, &self.groups, self.stage, self.playhead);
        let (start, end) = (range.start, range.end);
        if end <= start {
            return Vec::new();
        }
        let anchor = table.page_of(playhead.saturating_sub(1));
        let first = table.page_of(start);
        let last = table.page_of(end - 1);
        let mut pages: Vec<usize> = (first..=last).collect();
        pages.sort_by_key(|page| (page.abs_diff(anchor), *page));
        pages
    }

    pub fn visible_motion_range(&self) -> std::ops::Range<usize> {
        self.scene.as_ref().map_or(0..0, |scene| {
            visible_range(scene, &self.groups, self.stage, self.playhead)
        })
    }

    /// Stage rows the playback bar lays out: every stage when the job fits the
    /// window, otherwise a window of [`TIMELINE_WINDOW`] rows that follows the
    /// selected stage. Layout cost stays bounded no matter how many operations
    /// the job has, and the established labels are unchanged inside the window.
    fn timeline_window(&self, total: usize) -> (usize, usize) {
        if total <= TIMELINE_WINDOW {
            return (0, total);
        }
        let first = self.stage_window.min(total - TIMELINE_WINDOW);
        // "All paths" selects no stage, so the window is the user's own scroll
        // position rather than a re-centre on row zero.
        if self.stage == 0 {
            return (first, first + TIMELINE_WINDOW);
        }
        let selected = (self.stage - 1).min(total - 1);
        if selected < first {
            (selected, selected + TIMELINE_WINDOW)
        } else if selected >= first + TIMELINE_WINDOW {
            let start = selected + 1 - TIMELINE_WINDOW;
            (start, start + TIMELINE_WINDOW)
        } else {
            (first, first + TIMELINE_WINDOW)
        }
    }

    fn stock_seek(&mut self, target: usize) {
        let target = target.min(self.motion_count());
        if target != self.stock_prefix {
            self.requested_stock = Some(target);
            self.requested_prefix = Some(target);
        }
    }
    /// Display picking entry point. Exposed so the interaction harness can
    /// drive a pick at a chosen scale factor without pointer simulation.
    pub fn pick_at(&mut self, cursor: egui::Pos2, rect: egui::Rect, pixels_per_point: f32) {
        let Some(scene) = &self.scene else {
            return;
        };
        if scene.motion_count() == 0 {
            return;
        }
        if self.picker.is_none() {
            let timer = crate::clock::Timer::start();
            match Picker::from_vertex_bytes(scene.motion_bytes()) {
                Ok(picker) => {
                    self.picker = Some(picker);
                    self.picker_build_ms = timer.elapsed_ms();
                }
                Err(error) => {
                    self.status = format!("Picking index unavailable: {error}");
                    return;
                }
            }
        }
        let camera = self.camera(rect);
        let rect_points = [rect.width(), rect.height()];
        let local = [cursor.x - rect.center().x, cursor.y - rect.center().y];
        let ppp = pixels_per_point.max(1e-3);
        let picker = self.picker.as_ref().unwrap();
        self.selection = picker.pick_visible(
            &camera,
            rect_points,
            local,
            self.pick_tolerance_px,
            ppp,
            self.visible_motion_range(),
        );
        self.overlay_signature = None;
        self.status = match self.selection {
            Some(pick) => format!(
                "Picked motion {} at {:.2} physical px ({:.0} px tolerance, {:.2}× DPI)",
                pick.motion, pick.distance_pixels, self.pick_tolerance_px, ppp
            ),
            None => format!(
                "No motion within {:.0} physical px at {:.2}× DPI",
                self.pick_tolerance_px, ppp
            ),
        };
    }

    fn camera(&self, rect: egui::Rect) -> Camera {
        Camera {
            aspect: (rect.width() / rect.height().max(1.)).max(0.2),
            ..self.camera
        }
    }

    /// Frame the whole scene again: the default azimuth, zoom one and no pan.
    /// The elevation the user chose is kept.
    fn fit(&mut self) {
        self.camera.yaw = 0.;
        self.camera.set_zoom(1.);
        self.camera.pan = [0., 0.];
    }

    /// The palette the stock pass reads, rebuilt only when the style or the
    /// displayed plan's stage identities change.
    fn stock_palette(&mut self) -> Arc<Vec<[f32; 4]>> {
        if let Some(palette) = &self.palette {
            return palette.clone();
        }
        let palette = Arc::new(self.stock_style.palette(&self.stages, &self.tool_ids));
        self.palette = Some(palette.clone());
        self.palette_revision = self.palette_revision.wrapping_add(1);
        palette
    }

    /// Everything the stock pass needs from the display style: the uniform
    /// (with the camera's key light), the palette and its revision.
    fn stock_style_inputs(
        &mut self,
        rect: egui::Rect,
    ) -> (stock_style::StockUniform, Arc<Vec<[f32; 4]>>, u64) {
        let palette = self.stock_palette();
        let camera = self.camera(rect);
        let (_, _, to_camera) = camera.screen_basis();
        // A true elevation has no visible surface, so the pass draws the
        // section instead of the cell quads.
        let flags = stock_style::FLAG_WALLS
            | if camera.is_elevation() {
                stock_style::FLAG_SECTION
            } else {
                0
            };
        let uniform = self.stock_style.uniform(
            stock_style::key_light(),
            [to_camera[0], to_camera[1], to_camera[2], 0.],
            flags,
        );
        // The shader needs the same step threshold the wall builder uses, to
        // tell a slope (interpolate) from a step (keep sharp).
        let mut uniform = uniform;
        if let Some(stock) = self.stock.as_ref() {
            uniform.wall_threshold = stock_walls::effective_threshold(
                stock.meta.cell_mm,
                self.stock_style.wall_threshold(stock.meta.cell_mm) as f64,
            ) as f32;
        }
        (uniform, palette, self.palette_revision)
    }

    /// The walls of the displayed field state, rebuilt when the state or the
    /// style's threshold changes and cached otherwise. The builder reads the
    /// packed field the display already holds, so no wall geometry is ever
    /// transported (plan `stock-display-plan.md` §4).
    fn stock_walls(&mut self, rect: egui::Rect) -> (Arc<Vec<Wall>>, u64) {
        let camera = self.camera(rect);
        // A true elevation draws the section: the silhouette of the material
        // across the screen, which is what makes a front view readable.
        let section = camera.is_elevation().then(|| camera.section_along_x());
        let Some(stock) = self.stock.as_ref() else {
            return (Arc::new(Vec::new()), self.wall_revision);
        };
        let threshold = self.stock_style.wall_threshold(stock.meta.cell_mm);
        let identity = stock.identity;
        let prefix = stock.prefix;
        if let Some(cache) = &self.wall_cache
            && cache.identity == identity
            && cache.prefix == prefix
            && cache.threshold == threshold.to_bits()
            && cache.section == section
        {
            return (cache.walls.clone(), cache.revision);
        }
        // The displayed cells come either from the transported payload or from
        // the locally re-integrated field; both are this process's own bytes.
        let cells: &[u8] = match (&stock.local, self.scene.as_ref()) {
            (Some(local), _) => local,
            (None, Some(scene)) => scene.payload.get(stock.range.clone()).unwrap_or(&[]),
            (None, None) => &[],
        };
        let meta = &stock.meta;
        let view = stock_walls::Grid {
            cells,
            cols: meta.cols,
            rows: meta.rows,
            tiles_x: meta.tiles_x,
            cell_mm: meta.cell_mm,
            thickness_mm: meta.stock.thickness_mm,
            width_cells: (meta.stock.x1 - meta.stock.x0) / meta.cell_mm,
            length_cells: (meta.stock.y1 - meta.stock.y0) / meta.cell_mm,
        };
        let built = match section {
            Some(along_x) => stock_walls::WallSet {
                walls: stock_walls::build_section(&view, along_x),
                threshold_mm: threshold as f64,
                dropped: 0,
            },
            None => stock_walls::build(&view, threshold as f64, stock_walls::WALL_BUDGET_INSTANCES),
        };
        let revision = self.wall_revision.wrapping_add(1);
        let walls = Arc::new(built.walls);
        self.wall_report = (built.dropped > 0)
            .then_some((built.dropped, built.threshold_mm))
            .or(Some((0, built.threshold_mm)));
        self.wall_cache = Some(WallCache {
            identity,
            prefix,
            threshold: threshold.to_bits(),
            section,
            walls: walls.clone(),
            revision,
        });
        self.wall_revision = revision;
        (walls, revision)
    }

    /// Zoom about a viewport point, keeping the scene point under it fixed.
    fn zoom_at(&mut self, cursor: egui::Pos2, rect: egui::Rect, factor: f32) {
        let mut camera = self.camera(rect);
        let ndc = camera.to_ndc(
            [cursor.x - rect.center().x, cursor.y - rect.center().y],
            [rect.width(), rect.height()],
        );
        camera.zoom_toward(ndc, factor);
        self.camera.zoom = camera.zoom;
        self.camera.pan = camera.pan;
    }

    /// Selection fill and blade marker in the same normalized scene space as
    /// the motion vertices. Nothing here is CAM geometry.
    fn build_overlay(&mut self, rect: egui::Rect, ppp: f32) {
        let signature = (
            self.scene_revision,
            self.selection.map(|pick| pick.motion),
            self.playhead,
            self.stage,
            (self.camera.zoom * 100.) as i32,
            rect.width() as i32,
            rect.height() as i32,
            self.profile_anchor_signature,
            self.anchor_drag
                .as_ref()
                .map(|drag| (drag.point[0].to_bits(), drag.point[1].to_bits())),
        );
        if self.overlay_signature == Some(signature) {
            return;
        }
        self.overlay_signature = Some(signature);
        self.overlay_revision += 1;
        let Some(scene) = &self.scene else {
            return;
        };
        let size = (scene.meta.bounds[2] - scene.meta.bounds[0])
            .max(scene.meta.bounds[3] - scene.meta.bounds[1])
            .max(0.001);
        let scale = 1.6 / size;
        // Points per scene unit, matching the viewport projection.
        let per_unit = self.camera.points_per_unit([rect.width(), rect.height()]);
        let selection = self
            .selection
            .filter(|p| self.visible_motion_range().contains(&(p.motion as usize)))
            .and_then(|pick| {
                self.picker.as_ref().and_then(|picker| {
                    picker
                        .endpoints(pick.motion)
                        .map(|points| (points[0], points[1]))
                })
            });
        let marker = scene
            .meta
            .sim
            .as_ref()
            .filter(|_| self.playhead > 0)
            .and_then(|sim| {
                let picker = self.picker.as_ref()?;
                let index = (self.playhead - 1).min(picker.motion_count().saturating_sub(1));
                let points = picker.endpoints(index as u32)?;
                // The cutter glyph follows the stage that owns this motion.
                let motion_tool = self
                    .groups
                    .iter()
                    .find(|group| index >= group.start && index < group.end)
                    .map_or(0, |group| group.tool);
                let tool = sim.tools.get(motion_tool).copied()?;
                let tip = [points[1][0], points[1][1], points[1][2]];
                Some(match tool {
                    sim::ToolSpec::Knife { .. } => return None,
                    sim::ToolSpec::Endmill { diameter } => overlay::Marker {
                        glyph: overlay::Glyph::Endmill {
                            radius: (diameter / 2.) as f32 * scale as f32,
                        },
                        tip,
                        top: 0.,
                    },
                    sim::ToolSpec::Vbit {
                        angle,
                        tip: tip_diameter,
                        diameter,
                        ..
                    } => overlay::Marker {
                        glyph: overlay::Glyph::Vbit {
                            radius: (diameter.max(tip_diameter) / 2.) as f32 * scale as f32,
                            tip_radius: (tip_diameter / 2.) as f32 * scale as f32,
                            slope: ((angle / 2.) as f32 * std::f32::consts::PI / 180.).tan(),
                        },
                        tip,
                        top: 0.,
                    },
                })
            });
        let half = self.pick_tolerance_px * 0.5 / ppp.max(1e-3) / per_unit.max(1e-6);
        let mut built = overlay::build(selection, half, marker.as_slice());
        self.knife_overlay(&mut built);
        self.profile_overlay(&mut built);
        self.overlay_lines = Arc::new(built.lines);
        self.overlay_triangles = Arc::new(built.triangles);
    }

    fn fingerprint_pages(&mut self) {
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let table = page_table(scene);
        if self.hash_cursor >= table.page_count() || self.page_hashes.len() != table.page_count() {
            return;
        }
        let timer = crate::clock::Timer::start();
        let hashes = Arc::make_mut(&mut self.page_hashes);
        for _ in 0..HASH_PAGES_PER_FRAME {
            if self.hash_cursor >= table.page_count() {
                break;
            }
            let page = self.hash_cursor;
            hashes[page] = Some(pages::page_hash(&scene.payload[table.bytes_of(page)]));
            self.hash_cursor += 1;
        }
        self.hash_ms += timer.elapsed_ms();
    }
}

/// Tool ids by simulation tool index, from the stage table, so a `ByTool`
/// palette keys on the id rather than the index.
fn stage_tool_ids(stages: &[StageIdentity]) -> Vec<String> {
    let count = stages.iter().map(|stage| stage.tool + 1).max().unwrap_or(0);
    let mut ids = vec![String::new(); count];
    for stage in stages {
        let slot = &mut ids[stage.tool];
        if slot.is_empty() {
            *slot = stage.tool_id.clone();
        }
    }
    ids
}

fn stock_drill(drill: render::Drill) -> stock_render::Drill {
    match drill {
        render::Drill::Recover => stock_render::Drill::Recover,
        _ => stock_render::Drill::None,
    }
}

fn page_table(scene: &Scene) -> pages::PageTable {
    pages::PageTable::new(
        scene.meta.motion_offset,
        scene.meta.motion_len,
        scene.meta.motions,
    )
    .unwrap_or(pages::PageTable {
        motions: 0,
        page_motions: pages::PAGE_MOTIONS,
        motion_offset: 0,
        motion_len: 0,
    })
}

/// The motion span the selected path group shows, clipped to the playhead.
/// Hiding a group's paths never rewinds the stock: the removal at the current
/// playhead is preserved because the stock field is independent of this range.
fn visible_range(
    scene: &Scene,
    groups: &[DisplayGroup],
    stage: usize,
    prefix: usize,
) -> std::ops::Range<usize> {
    let end = prefix.min(scene.motion_count());
    if stage == 0 {
        return 0..end;
    }
    let Some(group) = groups.get(stage - 1) else {
        return 0..end;
    };
    let start = group.start.min(end);
    start..group.end.clamp(start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeline_lays_out_a_bounded_window_around_the_selected_stage() {
        let mut view = Viewport::default();
        // Every established review build fits the window unchanged.
        assert_eq!(view.timeline_window(0), (0, 0));
        assert_eq!(view.timeline_window(TIMELINE_WINDOW), (0, TIMELINE_WINDOW));
        assert_eq!(
            view.timeline_window(TIMELINE_WINDOW + 1).1
                - view.timeline_window(TIMELINE_WINDOW + 1).0,
            TIMELINE_WINDOW
        );
        // Selecting a stage outside the window moves it instead of growing the
        // row count; the selected stage is always inside.
        view.stage = 500;
        let (first, last) = view.timeline_window(1_000);
        assert_eq!(last - first, TIMELINE_WINDOW);
        assert!((first..last).contains(&499));
        view.stage = 1_000;
        let (first, last) = view.timeline_window(1_000);
        assert_eq!(last - first, TIMELINE_WINDOW);
        assert!((first..last).contains(&999));
        // The window control scrolls by one window without leaving the list.
        view.stage = 0;
        view.stage_window = 1_000;
        assert_eq!(view.timeline_window(1_000), (992, 1_000));
    }

    /// Drive the real widget with synthetic pointer input, so the wiring from
    /// button to camera is covered and not only the camera math in `camera.rs`.
    fn frame(view: &mut Viewport, ctx: &egui::Context, events: Vec<egui::Event>) {
        frame_with(view, ctx, events, egui::Modifiers::default());
    }

    fn frame_with(
        view: &mut Viewport,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200., 900.),
                )),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| view.viewport(ctx),
        );
    }

    fn viewport_rect() -> egui::Rect {
        let [x0, y0, x1, y1] =
            crate::app::control_rect("Artwork viewport").expect("the viewport reports its rect");
        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    }

    fn pointer(position: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Press, move and release; returns the camera the widget settled on.
    fn drag(
        view: &mut Viewport,
        ctx: &egui::Context,
        button: egui::PointerButton,
        delta: egui::Vec2,
        shift: bool,
    ) {
        frame(view, ctx, vec![]);
        let rect = viewport_rect();
        let start = rect.center();
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        frame_with(view, ctx, vec![egui::Event::PointerMoved(start)], modifiers);
        frame_with(
            view,
            ctx,
            vec![egui::Event::PointerButton {
                pos: start,
                button,
                pressed: true,
                modifiers,
            }],
            modifiers,
        );
        for step in 1..=4 {
            let position = start + delta * (step as f32 / 4.);
            frame_with(
                view,
                ctx,
                vec![egui::Event::PointerMoved(position)],
                modifiers,
            );
        }
        frame_with(
            view,
            ctx,
            vec![egui::Event::PointerButton {
                pos: start + delta,
                button,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
    }

    fn click(view: &mut Viewport, ctx: &egui::Context, position: egui::Pos2) {
        frame(view, ctx, vec![egui::Event::PointerMoved(position)]);
        frame(
            view,
            ctx,
            vec![pointer(position, egui::PointerButton::Primary, true)],
        );
        frame(
            view,
            ctx,
            vec![pointer(position, egui::PointerButton::Primary, false)],
        );
    }

    #[test]
    fn the_pointer_orbits_pans_and_zooms_the_real_viewport() {
        let mut view = Viewport::default();
        let ctx = egui::Context::default();
        assert_eq!(view.camera, Camera::default());

        // Primary drag orbits: the history is a horizontal turn and a vertical
        // elevation, and the elevation stops at the limit.
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Primary,
            egui::vec2(60., 30.),
            false,
        );
        assert!(view.camera.yaw > 0., "{:?}", view.camera);
        assert!(view.camera.tilt > 0., "{:?}", view.camera);
        let yaw = view.camera.yaw;
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Primary,
            egui::vec2(0., 10_000.),
            false,
        );
        assert_eq!(view.camera.tilt, camera::TILT_LIMIT);
        assert_eq!(view.camera.yaw, yaw, "a pure elevation drag must not turn");
        view.fit();

        // Middle drag pans without turning, and the primary drag stays free
        // for the artwork gesture that owns it.
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Middle,
            egui::vec2(40., 0.),
            false,
        );
        assert!(view.camera.pan[0] > 0., "{:?}", view.camera);
        assert_eq!(view.camera.yaw, 0.);
        let pan = view.camera.pan;
        view.artwork.enabled = true;
        view.artwork.mode = crate::artwork_view::GestureMode::Move;
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Primary,
            egui::vec2(60., 0.),
            false,
        );
        assert_eq!(view.camera.yaw, 0., "the artwork gesture owns the drag");
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Middle,
            egui::vec2(40., 0.),
            false,
        );
        assert!(view.camera.pan[0] > pan[0], "panning stays available");
        // Shift with the primary button pans while the viewport, not the
        // artwork, owns the drag.
        view.artwork.mode = crate::artwork_view::GestureMode::Select;
        let pan = view.camera.pan;
        let yaw = view.camera.yaw;
        drag(
            &mut view,
            &ctx,
            egui::PointerButton::Primary,
            egui::vec2(40., 0.),
            true,
        );
        assert!(
            view.camera.pan[0] > pan[0] && view.camera.yaw == yaw,
            "shift-drag pans instead of orbiting: {:?}",
            view.camera
        );
        view.artwork.enabled = false;
    }

    #[test]
    fn the_wheel_zooms_toward_the_pointer_and_the_presets_set_the_elevation() {
        let mut view = Viewport::default();
        let ctx = egui::Context::default();
        frame(&mut view, &ctx, vec![]);
        let rect = viewport_rect();
        let cursor = rect.center() + egui::vec2(rect.width() * 0.3, -rect.height() * 0.2);
        let before = view
            .camera(rect)
            .ground_point(
                [cursor.x - rect.center().x, cursor.y - rect.center().y],
                [rect.width(), rect.height()],
            )
            .unwrap();
        frame(
            &mut view,
            &ctx,
            vec![
                egui::Event::PointerMoved(cursor),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0., 1.),
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
        assert!(view.camera.zoom > 1., "{:?}", view.camera);
        let after = view
            .camera(rect)
            .ground_point(
                [cursor.x - rect.center().x, cursor.y - rect.center().y],
                [rect.width(), rect.height()],
            )
            .unwrap();
        assert!(
            (before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3,
            "{before:?} -> {after:?}"
        );

        // The presets set the elevation and leave the azimuth alone.
        view.camera.yaw = 0.4;
        let isometric = crate::app::control_rect("Isometric").unwrap();
        click(
            &mut view,
            &ctx,
            egui::pos2(
                (isometric[0] + isometric[2]) / 2.,
                (isometric[1] + isometric[3]) / 2.,
            ),
        );
        assert!(
            (view.camera.tilt - camera::ISO_TILT).abs() < 1e-6,
            "{:?}",
            view.camera
        );
        let top = crate::app::control_rect("Top").unwrap();
        click(
            &mut view,
            &ctx,
            egui::pos2((top[0] + top[2]) / 2., (top[1] + top[3]) / 2.),
        );
        assert_eq!(view.camera.tilt, 0.);
        assert_eq!(view.camera.yaw, 0.4);
        assert!(!view.settings().isometric);
    }

    /// The stock pass reads one uniform and one palette; both have to follow
    /// the display style, the camera and the displayed plan, and the palette
    /// must only be rebuilt when one of those changes.
    #[test]
    fn the_stock_style_drives_the_uniform_and_the_palette() {
        let mut view = Viewport::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        view.stages = Arc::new(vec![StageIdentity {
            index: 0,
            operation: "op-a".into(),
            tool: 0,
            tool_id: "tool-a".into(),
            role: "Endmill".into(),
        }]);
        view.tool_ids = Arc::new(vec!["tool-a".into()]);

        let (uniform, palette, revision) = view.stock_style_inputs(rect);
        assert_eq!(uniform.mode, 0, "plain is the shipped default");
        assert_eq!(palette[0][3], 1., "the stage slot carries a colour");
        assert_eq!(palette[stock_style::PALETTE_STAGES][3], 1., "tool slot");
        assert_eq!(
            palette[stock_style::PALETTE_STAGES + 1][3],
            0.,
            "an unused slot stays empty rather than borrowing a colour"
        );
        // The light leans toward the viewer and carries the ambient term.
        assert!(uniform.light[3] > 0. && uniform.light[3] < 1.);
        let (_, _, to_camera) = view.camera.screen_basis();
        assert!(uniform.to_camera[0] == to_camera[0] && uniform.to_camera[1] == to_camera[1]);
        assert_eq!(uniform.wall_mode, 0);
        assert_eq!(uniform.appearance, 0);
        assert_eq!(
            uniform.flags & stock_style::FLAG_WALLS,
            stock_style::FLAG_WALLS
        );

        // A cached palette is not rebuilt, and its revision does not move.
        let (_, cached, cached_revision) = view.stock_style_inputs(rect);
        assert_eq!(cached_revision, revision);
        assert!(Arc::ptr_eq(&palette, &cached));

        // Changing the mode changes the uniform but not the palette.
        view.stock_style.surface = stock_style::ColorMode::ByOperation;
        let (uniform, same_palette, same_revision) = view.stock_style_inputs(rect);
        assert_eq!(uniform.mode, 1);
        assert!(Arc::ptr_eq(&palette, &same_palette));
        assert_eq!(same_revision, revision);

        // Changing the palette's inputs rebuilds it and moves the revision, so
        // the renderer knows to upload.
        view.palette = None;
        let (_, rebuilt, rebuilt_revision) = view.stock_style_inputs(rect);
        assert!(rebuilt_revision > revision);
        assert!(!Arc::ptr_eq(&palette, &rebuilt));

        // The appearance and the toggles reach the probe the harness reads.
        view.stock_style.appearance = stock_style::Appearance::XRay;
        view.stock_style.show_paths = false;
        view.stock_style.show_artwork = false;
        let probe = view.display_probe();
        assert_eq!(probe["style"]["appearance"], "xray");
        assert_eq!(probe["style"]["showPaths"], false);
        assert_eq!(probe["style"]["showArtwork"], false);
        assert_eq!(probe["style"]["surface"], "by_operation");
    }

    /// A stock view over a small faced field, for the section test.
    fn faced_stock(cols: usize, rows: usize, cell_mm: f64) -> StockView {
        let stock = sim::Stock {
            x0: 0.,
            y0: 0.,
            x1: cols as f64 * cell_mm,
            y1: rows as f64 * cell_mm,
            thickness_mm: 10.,
        };
        let mut cells = vec![0u8; sim::TILE * sim::TILE * 4];
        for index in 0..cols * rows {
            // Faced 4 mm of a 10 mm stock, by stage 3.
            let word = 26_214u32 | (3u32 << 16);
            cells[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        StockView {
            meta: PreviewMeta {
                stock,
                cols,
                rows,
                cell_mm,
                reference_cell_mm: cell_mm,
                tiles_x: cols.div_ceil(sim::TILE),
                tiles_y: rows.div_ceil(sim::TILE),
                retained_bytes: cells.len(),
                ladder_frames: 0,
                preset: crate::stock_preview::DisplayPreset::Standard,
                key: "0".repeat(32),
                frames: vec![],
                dropped_stage_marks: 0,
            },
            identity: 1,
            range: 0..0,
            local: Some(Arc::new(cells)),
            cell_versions: Arc::new(vec![0]),
            stats: Default::default(),
            prefix: 0,
            last_transfer_bytes: 0,
            last_replayed: 0,
        }
    }

    /// S4: a true elevation has no visible surface, so the pass draws the
    /// section instead and the probe says which way it runs.
    #[test]
    fn a_true_elevation_builds_a_section_and_sets_the_flag() {
        let mut view = Viewport {
            stock: Some(faced_stock(2, 1, 1.)),
            ..Viewport::default()
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        // Ordinary view: step walls, no section.
        let (uniform, _, _) = view.stock_style_inputs(rect);
        assert_eq!(uniform.flags & stock_style::FLAG_SECTION, 0);
        let (walls, _) = view.stock_walls(rect);
        assert!(
            walls
                .iter()
                .all(|wall| wall.identity != stock_walls::NO_CUTTER)
        );
        assert!(view.display_probe()["style"]["section"].is_null());
        // Front elevation: the section replaces the walls and the flag is set.
        view.camera.yaw = 0.;
        view.camera.set_tilt(camera::TILT_LIMIT);
        let (uniform, _, _) = view.stock_style_inputs(rect);
        assert_eq!(
            uniform.flags & stock_style::FLAG_SECTION,
            stock_style::FLAG_SECTION
        );
        let (walls, _) = view.stock_walls(rect);
        assert!(!walls.is_empty());
        assert!(
            walls.iter().all(|wall| wall.axis == 1),
            "a front view sections along X: {walls:?}"
        );
        assert!((walls[0].top - 0.4).abs() < 1e-4);
        assert_eq!(view.display_probe()["style"]["section"], "along_x");
        // A side elevation sections along Y instead.
        view.camera.yaw = camera::ELEVATIONS[2].1;
        let (walls, _) = view.stock_walls(rect);
        assert!(walls.iter().all(|wall| wall.axis == 0), "{walls:?}");
        assert_eq!(view.display_probe()["style"]["section"], "along_y");
    }

    /// S3: the whole style round-trips through the saved view, so a restored
    /// workspace keeps its colour mode, appearance and toggles.
    #[test]
    fn a_non_default_style_round_trips_through_the_saved_view() {
        let mut view = Viewport::default();
        view.stock_style.surface = stock_style::ColorMode::ByTool;
        view.stock_style.walls = stock_style::WallMode::ByDepth;
        view.stock_style.appearance = stock_style::Appearance::XRay;
        view.stock_style.xray_opacity = 0.25;
        view.stock_style.show_paths = false;
        view.stock_style
            .overrides
            .insert("op-a".into(), [1., 0., 0.]);
        let saved = view.settings();
        assert!(saved.validate().is_ok());
        let mut restored = Viewport::default();
        restored.restore_settings(&saved);
        assert_eq!(restored.settings(), saved);
        assert_eq!(restored.stock_style.surface, stock_style::ColorMode::ByTool);
        assert_eq!(restored.stock_style.walls, stock_style::WallMode::ByDepth);
        assert_eq!(restored.stock_style.xray_opacity, 0.25);
        assert!(!restored.stock_style.show_paths);
        assert_eq!(
            restored.stock_style.overrides.get("op-a"),
            Some(&[1., 0., 0.])
        );
    }
}
