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
#[path = "viewport_controls.rs"]
mod controls;
#[path = "viewport_face.rs"]
mod face;
#[path = "viewport_inspection.rs"]
mod inspection;
#[path = "viewport_knife.rs"]
mod knife;
#[path = "viewport_profile.rs"]
mod profile;
#[path = "viewport_transport.rs"]
mod transport;
pub use artwork::{ArtworkEvent, ArtworkInteraction};
pub use profile::{ProfileAnchor, ProfileAnchorEvent};

/// Pages fingerprinted per frame while a new scene settles.
const HASH_PAGES_PER_FRAME: usize = 4;
/// Wall-clock seconds the whole program takes when the transport is set to
/// **Fit**. The speed buttons scale real machine time instead: 1x plays a
/// program at the feeds it will actually run at.
const FIT_SECONDS: f64 = 20.;
/// Motions one displayed frame may apply locally. Beyond this — a stall, a very
/// high time scale, a job far larger than the frame — the display asks the
/// compute process for one exact seek instead of replaying them itself.
const LOCAL_ADVANCE_LIMIT: usize = 4096;
/// Machine-warning rows the transport lists at once. The count above the list
/// always reports the whole set, so a long list never hides behind the window.
const WARNING_ROWS: usize = 6;
/// Everything the overlay geometry depends on; a change rebuilds it.
type OverlaySignature = (
    u64,
    Option<u32>,
    (usize, u64),
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
    /// payload or the display's own raster once the animation advances locally.
    range: std::ops::Range<usize>,
    local: Option<Arc<Vec<u8>>>,
    cell_versions: Arc<Vec<u32>>,
    stats: sim::Stats,
    prefix: usize,
    /// The display's own playback clock: the field between seeks, the program
    /// time table, and the position inside the current move. `None` when the
    /// scene carries no timed motion stream, in which case playback falls back
    /// to stepping whole motions exactly as before.
    clock: Option<crate::sim_clock::LocalClock>,
    /// Bumped whenever the displayed cell bytes change, so derived geometry —
    /// the walls — is rebuilt in the frame the floor moves. The motion prefix
    /// alone cannot serve that purpose: the animation advances inside a motion.
    raster_revision: u64,
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
    #[serde(default)]
    pub inspection_tab: usize,
    #[serde(default)]
    pub section_y: bool,
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
            inspection_tab: 0,
            section_y: false,
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
            || self.inspection_tab > 2
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
    /// The cell-bytes generation the walls were derived from. The motion prefix
    /// cannot serve: the animation advances *inside* one motion, so the floor
    /// moves while the prefix stands still, and walls keyed on it would be left
    /// standing until the move ended.
    raster: u64,
    threshold: u32,
    /// `None` for the ordinary step walls, `Some(along_x)` for a section.
    section: Option<bool>,
    /// Deepest removed fraction in this state: the depth ramp's own span.
    deepest: f32,
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
    /// Fraction of that prefix's in-flight move to apply once the response
    /// lands: a scrub to a program time inside one long move needs the exact
    /// prefix from the compute process first, then the partial window locally.
    requested_fraction: f64,
    /// Position carried by the submitted seek, separate from a newer scrub
    /// queued while that response is in flight.
    inflight_stock: Option<(usize, f64)>,
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
    /// Modeled blade headings per motion — `(start, end)` in degrees — so the
    /// knife on screen turns the way the plan says it turns. Empty for a job
    /// with no knife stage, which publishes none.
    knife_headings: Arc<Vec<Option<(f64, f64)>>>,
    /// The machine's holder body, resolved from the catalogue once per scene.
    /// Empty when the machine states no holder, or names one that is not in the
    /// catalogue — in which case nothing is drawn rather than something invented.
    holder_body: Arc<Vec<cam_core::post::HolderSegment>>,
    /// Machine warnings the worker published with this execution: display
    /// estimates over its own raster, never a machining verdict.
    warnings: Arc<Vec<crate::sim_checks::Warning>>,
    /// The cell size those warnings were computed on, named in the panel so a
    /// coarse estimate never reads as an exact one (D10).
    warning_cell_mm: f64,
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
    /// Playback time scale. 1x is real machine time: the animation advances the
    /// program by its own feeds, so a pass and a plunge take the time the
    /// machine will take, in the ratio the plan commands.
    playback_speed: f64,
    /// Play the whole program in [`FIT_SECONDS`] instead of at `playback_speed`.
    playback_fit: bool,
    transport_details: bool,
    /// Sub-motion progress of the fallback clock, used when a plan's timing
    /// cannot be derived (a linear feed with no rate): playback then steps whole
    /// motions exactly as it did before the clock existed.
    playback_progress: f64,
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
    /// The cutter tip the overlay was last built with, in scene coordinates:
    /// published so a review can see that the marker it looks for is drawn
    /// where the plan says the cutter is, and not only that it was drawn.
    marker_tip: Option<[f32; 3]>,
    /// The same tip in plan millimetres, when the frame drew the cutter
    /// part-way inside a move. The clock advances after the overlay is built,
    /// so a review that reads the live clock compares this against the drawn
    /// tip rather than across a frame boundary.
    marker_plan_tip: Option<[f64; 3]>,
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
            requested_fraction: 0.,
            inflight_stock: None,
            requested_preset: None,
            status: "Open a job to begin.".into(),
            scene: None,
            camera: Camera::default(),
            stock_style: StockStyle::default(),
            stages: Arc::new(Vec::new()),
            tool_ids: Arc::new(Vec::new()),
            knife_headings: Arc::new(Vec::new()),
            holder_body: Arc::new(Vec::new()),
            warnings: Arc::new(Vec::new()),
            warning_cell_mm: 0.,
            palette: None,
            palette_revision: 0,
            wall_cache: None,
            wall_revision: 0,
            wall_report: None,
            playhead: 0,
            playing: false,
            playback_speed: 1.,
            playback_fit: false,
            transport_details: false,
            playback_progress: 0.,
            stage: 0,
            stage_window: 0,
            timeline_rows: 0,
            picker: None,
            picker_build_ms: 0.,
            selection: None,
            pick_tolerance_px: 8.,
            overlay_lines: Arc::new(Vec::new()),
            overlay_triangles: Arc::new(Vec::new()),
            marker_tip: None,
            marker_plan_tip: None,
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
            inspection_tab: self.inspection.tab,
            section_y: self.inspection.axis == inspection::SectionAxis::Y,
            // Persist the user's chosen resolution, not just what happens to be
            // displayed: a saved context that has not generated yet must keep
            // the choice it was made with.
            preset: self.desired_preset(),
        }
    }
    pub fn restore_settings(&mut self, settings: &ViewSettings) {
        self.inspection.point = settings.inspection_xy;
        self.inspection.tab = settings.inspection_tab;
        self.inspection.axis = if settings.section_y {
            inspection::SectionAxis::Y
        } else {
            inspection::SectionAxis::X
        };
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
        self.inflight_stock = None;
        self.requested_fraction = 0.;
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
        // Blade headings the plan modeled, per motion: the drawn knife turns
        // with them, and a milling job publishes none.
        self.knife_headings = Arc::new({
            let mut headings = vec![None; scene.motion_count()];
            if let Some(entries) = scene.meta.report["gui2"]["knifeMotions"].as_array() {
                for (position, entry) in entries.iter().enumerate() {
                    let index = entry["index"].as_u64().map_or(position, |i| i as usize);
                    let Some(ends) = entry["heading"].as_array() else {
                        continue;
                    };
                    let (Some(start), Some(end)) =
                        (ends[0].as_f64(), ends.get(1).and_then(|v| v.as_f64()))
                    else {
                        continue;
                    };
                    if let Some(slot) = headings.get_mut(index) {
                        *slot = Some((start, end));
                    }
                }
            }
            headings
        });
        // The machine's holder, resolved once: the catalogue lives in core, so
        // its segment numbers are not duplicated here and not copied per job.
        self.holder_body = Arc::new(
            scene
                .meta
                .sim
                .as_ref()
                .and_then(|sim| sim.holder.as_ref())
                .and_then(|holder| holder.body())
                .unwrap_or_default(),
        );
        // Machine warnings travel with the execution: computed once, over the
        // raster the plan names, and never recomputed per frame.
        self.warnings = Arc::new(
            scene.meta.report["gui2"]["warnings"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.warning_cell_mm = scene.meta.report["gui2"]["warningCellMm"]
            .as_f64()
            .unwrap_or(0.);
        self.palette = None;
        if self.stage > self.groups.len() {
            self.stage = 0;
        }
        self.playhead = scene.motion_count();
        self.stock_prefix = scene.motion_count();
        self.requested_prefix = None;
        self.requested_stock = None;
        self.inflight_stock = None;
        self.requested_fraction = 0.;
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
        let clock = self.seed_clock(scene, &meta);
        Some(StockView {
            cell_versions: versions[versions.len() - 1].clone(),
            meta,
            identity,
            range: cells,
            local: None,
            stats,
            prefix,
            clock,
            raster_revision: 0,
            last_transfer_bytes: 0,
            last_replayed: 0,
        })
    }

    /// Seed the display's playback clock from the frames that travelled with the
    /// scene. Two states are enough to run and rewind a program — the pristine
    /// stock at prefix 0 and the state the display opens on — because the whole
    /// ladder is already resident as payload bytes and a seek that needs another
    /// checkpoint asks the compute process for it.
    fn seed_clock(
        &self,
        scene: &Scene,
        meta: &crate::stock_preview::PreviewMeta,
    ) -> Option<crate::sim_clock::LocalClock> {
        let input = scene.sim_input().ok().flatten()?;
        let field_at = |index: usize| {
            let frame = meta.frames.get(index)?;
            let cells = scene.stock_cells(index)?;
            sim::Field::from_packed(
                meta.stock,
                &input.tools,
                meta.cell_mm,
                cells,
                &frame.versions,
                &frame.allocated,
                frame.stats.clone(),
            )
            .ok()
        };
        let pristine = field_at(0)?;
        let last = meta.frames.len().checked_sub(1)?;
        let field = field_at(last)?;
        let prefix = meta.frames.get(last)?.prefix;
        // The program's own numbers time the moves. A machine that states no
        // rapid rate is still played; the transport says the total rests on a
        // stated assumption. A plan that cannot be timed at all (a linear feed
        // with no rate) leaves the fallback clock in place.
        let time = sim::TimeTable::build(
            &input.motions,
            scene
                .meta
                .sim
                .as_ref()
                .and_then(|sim| sim.rapid_rate_mm_min),
        )
        .ok()?;
        Some(crate::sim_clock::LocalClock::seed(
            field.clone(),
            pristine.clone(),
            vec![(prefix, field)],
            input.motions,
            time,
            meta.retained_bytes,
        ))
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
            self.view_toolbar(ui);
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
            let (walls, wall_revision) = self.stock_walls(rect);
            let (style_uniform, palette, palette_revision) = self.stock_style_inputs(rect);
            if self.gpu_unavailable {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Injected renderer failure\nDraft and prior result retained",
                    egui::FontId::proportional(18.),
                    crate::ui_theme::ERROR,
                );
                let recover = ui.put(
                    egui::Rect::from_center_size(
                        rect.center() + egui::vec2(0., 58.),
                        egui::vec2(200., 28.),
                    ),
                    egui::Button::new("Restore viewport"),
                );
                crate::app::observe_control("Restore viewport", recover.rect);
                if recover.clicked() {
                    self.rebuild_renderer_resources();
                }
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
                            // An empty span hides the paths without touching the
                            // plan; the two kinds are filtered in the shader.
                            visible: if self.stock_style.show_cutting
                                || self.stock_style.show_travel
                            {
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
                            camera: self.scene_camera(rect),
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
                                show_edges: self.stock_style.show_edges,
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
            let readout = ui.painter().layout(
                format!("{} motions{}", self.motion_count(), pick),
                egui::FontId::monospace(11.),
                crate::ui_theme::MUTED,
                (rect.width() - 24.).max(1.),
            );
            let origin = rect.left_bottom() + egui::vec2(12., -12. - readout.size().y);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(origin, readout.size()).expand(3.),
                2.,
                crate::ui_theme::SURFACE,
            );
            ui.painter().galley(origin, readout, crate::ui_theme::MUTED);
        });
    }

    /// Shared production viewport and cumulative stock transport.
    /// Pause presentation while retaining the exact playhead and result.
    pub fn pause(&mut self) {
        self.playing = false;
    }
    pub fn show(&mut self, ctx: &egui::Context, simulate: bool) {
        self.fingerprint_pages();
        if simulate {
            let panel =
                egui::TopBottomPanel::bottom("gui2-playback").show(ctx, |ui| self.transport(ui));
            crate::app::observe_control("Simulation transport", panel.response.rect);
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
            // Time-based advance: the clock advances the program by the seconds
            // the transport asks for, so the animation is independent of the
            // frame rate. A stalled frame is clamped rather than jumping the
            // program forward, and a step the display cannot replay locally
            // becomes one exact seek instead of a long frame.
            let dt = (ctx.input(|i| i.stable_dt) as f64).clamp(0., 0.1);
            let scale = if self.playback_fit {
                self.fit_scale()
            } else {
                self.playback_speed
            };
            self.play(dt * scale);
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
        // The clock runs on a raster, so a new resolution means a new field: the
        // response's frame reseeds it exactly, while the motion stream and the
        // program-time table stay as they are.
        let mut clock = self.stock.as_mut().and_then(|stock| stock.clock.take());
        if clock.is_some() {
            // A preset response carries cells, not a motion stream, so the tool
            // geometry comes from the scene that is already displayed.
            let tools = self
                .scene
                .as_ref()
                .and_then(|scene| scene.meta.sim.as_ref())
                .map(|sim| sim.tools.clone())
                .unwrap_or_default();
            let seeded = (!tools.is_empty())
                .then(|| {
                    sim::Field::from_packed(
                        preview.stock,
                        &tools,
                        preview.cell_mm,
                        cells,
                        &frame.versions,
                        &frame.allocated,
                        frame.stats.clone(),
                    )
                    .ok()
                })
                .flatten();
            match seeded {
                Some(field) => {
                    if let Some(active) = clock.as_mut() {
                        active.reseed(field.clone(), vec![(prefix, field)], preview.retained_bytes);
                    }
                }
                // A frame that cannot be rebuilt is not a field to animate from.
                None => clock = None,
            }
        }
        self.stock = Some(StockView {
            meta: preview,
            identity,
            range: section.offset..section.offset + section.len,
            local: Some(Arc::new(cells.to_vec())),
            cell_versions,
            stats,
            prefix,
            clock,
            // A new raster at a new resolution: no derived geometry survives it.
            raster_revision: self
                .stock
                .as_ref()
                .map_or(1, |previous| previous.raster_revision.wrapping_add(1)),
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
        let prefix = self.requested_stock.take()?;
        self.inflight_stock = Some((prefix, self.requested_fraction));
        Some(prefix)
    }
    /// Whether a stock seek is still waiting for the application to submit it.
    pub fn stock_request_pending(&self) -> bool {
        self.requested_stock.is_some()
    }
    pub fn accept_stock(&mut self, meta: SceneMeta, payload: Vec<u8>) -> Result<(), String> {
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
            .ok_or("Invalid stock payload")?
            .to_vec();
        // Rebuild the exact field the response describes, so the display's clock
        // can keep running from the state the compute process just restored. The
        // response carries cells, not a motion stream, so the tool geometry comes
        // from the scene that is already displayed.
        let tools = self
            .scene
            .as_ref()
            .and_then(|scene| scene.meta.sim.as_ref())
            .map(|sim| sim.tools.clone())
            .unwrap_or_default();
        let seed = (!tools.is_empty())
            .then(|| {
                sim::Field::from_packed(
                    preview.stock,
                    &tools,
                    preview.cell_mm,
                    &cells,
                    &frame.versions,
                    &frame.allocated,
                    frame.stats.clone(),
                )
                .ok()
            })
            .flatten();
        let prefix = frame.prefix;
        let versions = frame.versions.clone();
        let stats = frame.stats.clone();
        let key = preview.identity();
        let retained_bytes = preview.retained_bytes;
        let fraction = self
            .inflight_stock
            .filter(|(target, _)| *target == prefix)
            .map_or(self.requested_fraction, |(_, fraction)| fraction);
        let stock = self.stock.as_mut().ok_or("No displayed stock")?;
        stock.local = Some(Arc::new(cells));
        stock.cell_versions = Arc::new(versions);
        stock.stats = stats;
        stock.prefix = prefix;
        // The response names its own simulation key. If it differs (a rebuild
        // landed between requests) the renderer must re-upload every tile
        // instead of mixing cells from two resolutions.
        stock.identity = key;
        stock.last_transfer_bytes = payload.len();
        stock.last_replayed = meta.report["gui2"]["replayed"].as_u64().unwrap_or(0) as usize;
        // A transported state is new bytes: the walls derived from the previous
        // state must not survive it.
        stock.raster_revision = stock.raster_revision.wrapping_add(1);
        if let (Some(clock), Some(field)) = (stock.clock.as_mut(), seed) {
            clock.reseed(field.clone(), vec![(prefix, field)], retained_bytes);
        } else if stock.clock.is_some() {
            // The response could not be rebuilt as a field (a different
            // resolution, say): the display keeps drawing the transported cells
            // and drops the clock rather than animating from a wrong state.
            stock.clock = None;
        }
        self.stock_prefix = prefix;
        self.playhead = prefix;
        self.requested_prefix = self.requested_stock;
        self.stock_loading = false;
        // A scrub to a time inside a move: the exact prefix is here now, so the
        // partial window is applied locally instead of asking again.
        if fraction > 0. {
            let advance = match self.stock.as_mut().and_then(|stock| stock.clock.as_mut()) {
                Some(clock) => clock.move_to_position(prefix, fraction, LOCAL_ADVANCE_LIMIT),
                None => return Ok(()),
            };
            if let Some((dirty, _)) = self.resolve_advance(advance) {
                self.apply_local_advance(&dirty);
            }
        }
        Ok(())
    }
    /// A stock request finished, successfully or not. The application calls this
    /// when the command it submitted for a seek completes, so a refused or
    /// failed seek cannot leave "requested" pointing at a position the display
    /// is no longer going to reach.
    pub fn finish_stock_request(&mut self) {
        self.inflight_stock = None;
        // A seek that failed or was refused must not leave a fraction behind for
        // the next response to apply to a position nobody asked for.
        if self.requested_stock.is_none() {
            self.requested_prefix = None;
            self.requested_fraction = 0.;
        }
    }
    /// The worker execution was discarded; neither the submitted seek nor a
    /// queued successor can complete against that execution anymore.
    pub fn cancel_stock_requests(&mut self) {
        self.requested_stock = None;
        self.finish_stock_request();
        self.stock_loading = false;
        self.playing = false;
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
            // The walls derived from the displayed cells. Their revision has to
            // move with the raster, not with the motion index: the animation
            // advances inside one motion, and wall geometry keyed on the prefix
            // would stand still until the move ended.
            "walls": {
                "revision": self.wall_cache.as_ref().map_or(0, |cache| cache.revision),
                "instances": self.wall_cache.as_ref().map_or(0, |cache| cache.walls.len()),
                "dropped": self.wall_report.map_or(0, |(dropped, _)| dropped),
                "thresholdMm": self.wall_report.map_or(0., |(_, threshold)| threshold),
            },
            "simulation": self.simulation_probe(),
            // Machine warnings, as the panel shows them: a count, the raster
            // they rest on, and the first few entries for the review harness.
            "warnings": {
                "count": self.warnings.len(),
                "cellMm": self.warning_cell_mm,
                "entries": self
                    .warnings
                    .iter()
                    .take(WARNING_ROWS)
                    .map(|warning| serde_json::json!({
                        "kind": match warning.kind {
                            crate::sim_checks::WarningKind::AssemblyBelowSurface => "assemblyBelowSurface",
                            crate::sim_checks::WarningKind::RapidThroughMaterial => "rapidThroughMaterial",
                        },
                        "motion": warning.motion,
                        "depthMm": warning.depth_mm,
                        "maxDepthMm": warning.max_depth_mm,
                        "tool": warning.tool,
                    }))
                    .collect::<Vec<_>>(),
            },
        })
    }

    /// What the playback clock is showing: the position inside a move, the
    /// program time, and whether the machine's own rapid rate was available.
    /// The browser harness reads this instead of guessing from the status line.
    fn simulation_probe(&self) -> serde_json::Value {
        let Some(clock) = self.stock.as_ref().and_then(|stock| stock.clock.as_ref()) else {
            return serde_json::json!({
                "timed": false, "playing": self.playing,
                "fastForward": self.playback_speed,
                "fit": self.playback_fit,
            });
        };
        let (prefix, fraction) = clock.position();
        let (tip, feed) = clock
            .tip()
            .map(|(tip, motion)| (Some(tip), motion.feed_mm_min))
            .unwrap_or((None, None));
        serde_json::json!({
            "timed": true, "playing": self.playing,
            "prefix": prefix,
            "fraction": fraction,
            "elapsedSeconds": clock.seconds(),
            "totalSeconds": clock.total_seconds(),
            "tip": tip,
            // The cutter the last drawn frame actually put on screen: the same
            // point in the coordinates the viewport draws in, and in the plan
            // millimetres it came from. The marker has to stand here, and the
            // two agreeing is what an unnormalized tip — a marker nowhere near
            // its own path — breaks.
            "sceneTip": self.marker_tip,
            "planTip": self.marker_plan_tip,
            "bounds": self
                .scene
                .as_ref()
                .map(|scene| scene.meta.bounds)
                .unwrap_or([0.; 4]),
            "feedMmMin": feed,
            "rapidRateMmMin": clock.time().rapid_rate_mm_min(),
            "rapidRateAssumed": clock.time().assumes_rapid_rate(),
            // The stream the clock walks and the plan it came from, plus the tool
            // the marker draws next to the tool the motion's own stage names.
            // Those two agreeing is the invariant an expanded stream breaks.
            "planMotions": self.motion_count(),
            "displayMotions": self
                .scene
                .as_ref()
                .and_then(|scene| scene.meta.sim.as_ref())
                .map(|sim| sim.motions),
            "markerToolId": self
                .marker_index()
                .and_then(|index| self.group_tool_at(index))
                .and_then(|tool| self.tool_ids.get(tool))
                .cloned(),
            // The tool the motion stream itself names, at the position the
            // marker draws. At the end of the program there is no move in
            // flight, and the stream's own answer is the last one that ran —
            // the same rule the marker uses, so the two stay comparable.
            "stageToolId": clock
                .tool()
                .and_then(|tool| self.tool_ids.get(tool))
                .cloned(),
            "fastForward": if self.playback_fit { self.fit_scale() } else { self.playback_speed },
            "fit": self.playback_fit,
            // What the display is drawing above the cutter, so a review can see
            // that the assembly it warns about is the one it models.
            "assembly": self.current_assembly().map(|assembly| serde_json::json!({
                "shaftDiameterMm": assembly.shaft_diameter_mm,
                "stickoutMm": assembly.stickout_mm,
            })),
            "holderSegments": self.holder_body.len(),
        })
    }

    /// How the tool the clock is in is held, when the scene states it.
    fn current_assembly(&self) -> Option<cam_core::project::ToolAssembly> {
        let sim = self.scene.as_ref()?.meta.sim.as_ref()?;
        let tool = self.stock.as_ref()?.clock.as_ref()?.tool()?;
        sim.assemblies.get(tool).copied()
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
            "showCutting": self.stock_style.show_cutting,
            "showTravel": self.stock_style.show_travel,
            "showEdges": self.stock_style.show_edges,
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
        if target != self.stock_prefix || self.requested_prefix.is_some() {
            self.requested_stock = Some(target);
            self.requested_prefix = Some(target);
            self.requested_fraction = 0.;
        }
    }

    /// Program time of the displayed position and the program's total, when the
    /// scene carries a clock. `None` means playback has to step whole motions.
    pub fn program_time(&self) -> Option<(f64, f64)> {
        let clock = self.stock.as_ref()?.clock.as_ref()?;
        Some((clock.seconds(), clock.total_seconds()))
    }

    /// Blade heading at a position inside a motion, in degrees, or `None` for a
    /// cutter that is not a knife. The blade turns the short way round between
    /// the headings the plan modeled for the move's two ends, so a corner
    /// swivel reads as the turn the machine will make.
    fn knife_heading(&self, index: usize, fraction: f32) -> Option<f32> {
        let (start, end) = (*self.knife_headings.get(index)?)?;
        let turn = (end - start + 180.).rem_euclid(360.) - 180.;
        let heading = start + turn * fraction.clamp(0., 1.) as f64;
        // Canonical degrees, so a swivel across the 0/360 seam reads as 0°
        // rather than 360°.
        Some(heading.rem_euclid(360.) as f32)
    }

    /// The motion the cutter is drawn at: the one it is inside, or the last one
    /// it finished when it sits exactly on a boundary. The marker and the probe
    /// both ask here, so what a review reads is what is drawn.
    fn marker_index(&self) -> Option<usize> {
        let (playhead, fraction) = self
            .stock
            .as_ref()
            .and_then(|stock| stock.clock.as_ref())
            .map_or((self.playhead, 0.), |clock| clock.position());
        if playhead == 0 && fraction <= 0. {
            return None;
        }
        Some(if fraction > 0. {
            playhead
        } else {
            playhead.checked_sub(1)?
        })
    }

    /// The tool the display draws at a motion: the timeline group that owns it.
    /// The stages, the spans and the motion stream share one index, so this is
    /// the plan's own stage for that move — unless a stream and its tables have
    /// drifted apart, which is what the probe's `markerToolId` exists to catch.
    fn group_tool_at(&self, index: usize) -> Option<usize> {
        self.groups
            .iter()
            .find(|group| index >= group.start && index < group.end)
            .map(|group| group.tool)
    }

    /// The motion the clock is inside, for a position that follows the curve.
    fn clock_motion(&self) -> Option<&crate::sim::Motion> {
        self.stock.as_ref()?.clock.as_ref()?.motion()
    }

    /// Time scale that plays the whole program in [`FIT_SECONDS`] of wall clock.
    fn fit_scale(&self) -> f64 {
        self.stock
            .as_ref()
            .and_then(|stock| stock.clock.as_ref())
            .map_or(1., |clock| (clock.total_seconds() / FIT_SECONDS).max(1e-6))
    }

    /// One playback frame: advance the display's clock by machine seconds.
    fn play(&mut self, program_seconds: f64) {
        let advance = match self.stock.as_mut().and_then(|stock| stock.clock.as_mut()) {
            Some(clock) => clock.advance(program_seconds, LOCAL_ADVANCE_LIMIT),
            None => {
                self.play_by_steps(program_seconds);
                return;
            }
        };
        let (dirty, finished) = match self.resolve_advance(advance) {
            Some(resolved) => resolved,
            None => return,
        };
        self.apply_local_advance(&dirty);
        if finished {
            self.playing = false;
        }
    }

    /// Fallback playback for a scene whose motion stream cannot be timed (a
    /// linear feed with no rate): step whole motions, as the display did before
    /// it had a clock. The plan's own checks already refuse such a plan; the
    /// display keeps showing it rather than refusing the whole scene.
    fn play_by_steps(&mut self, program_seconds: f64) {
        self.playback_progress +=
            (self.motion_count() as f64 / FIT_SECONDS).max(1.) * program_seconds.max(0.);
        let step = self.playback_progress.floor();
        if step >= 1. {
            self.playback_progress -= step;
            self.stock_seek((self.stock_prefix + step as usize).min(self.motion_count()));
        }
        if self.stock_prefix == self.motion_count() {
            self.playing = false;
        }
    }

    /// A local advance either moved the field or has to become one exact seek
    /// against the retained execution (the target is behind the display, or
    /// further than one frame may replay).
    fn resolve_advance(&mut self, advance: crate::sim_clock::Advance) -> Option<(Vec<u32>, bool)> {
        use crate::sim_clock::Advance;
        match advance {
            Advance::Moved { dirty, .. } => Some((dirty, false)),
            Advance::Finished { dirty, .. } => Some((dirty, true)),
            Advance::NeedsRestore { prefix, fraction } => {
                self.requested_prefix = Some(prefix);
                self.requested_stock = Some(prefix);
                self.requested_fraction = fraction;
                None
            }
        }
    }

    /// Publish what the local clock reached: patch the tiles the advance
    /// changed, and report the position the display is now showing.
    fn apply_local_advance(&mut self, dirty: &[u32]) {
        let Some(stock) = self.stock.as_mut() else {
            return;
        };
        let Some(clock) = stock.clock.as_mut() else {
            return;
        };
        if !dirty.is_empty() {
            match stock.local.as_mut() {
                Some(bytes) => {
                    let target = Arc::make_mut(bytes);
                    let _ = clock.patch(target, dirty);
                }
                // The first local frame builds the display's own raster once;
                // after that only changed tiles are copied.
                None => stock.local = Some(Arc::new(clock.field().packed_tile_bytes())),
            }
        }
        stock.cell_versions = Arc::new(clock.versions());
        stock.stats = clock.stats().clone();
        stock.prefix = clock.position().0;
        // The bytes moved, so everything derived from them (the walls) has to be
        // rebuilt in this same frame.
        if !dirty.is_empty() {
            stock.raster_revision = stock.raster_revision.wrapping_add(1);
        }
        self.stock_prefix = stock.prefix;
        self.playhead = stock.prefix;
        self.overlay_signature = None;
    }

    /// Scrub the display to a program time. Forward motion is local; a target
    /// behind the display — where material would have to be restored — becomes
    /// one exact seek whose response reseeds the clock.
    fn seek_seconds(&mut self, seconds: f64) {
        let Some((prefix, fraction)) = self
            .stock
            .as_ref()
            .and_then(|stock| stock.clock.as_ref())
            .map(|clock| clock.time().position_at(seconds))
        else {
            return;
        };
        if self.inflight_stock.is_some() {
            self.requested_stock = Some(prefix);
            self.requested_prefix = Some(prefix);
            self.requested_fraction = fraction;
            return;
        }
        self.requested_stock = None;
        self.requested_prefix = None;
        self.requested_fraction = 0.;
        let advance = match self.stock.as_mut().and_then(|stock| stock.clock.as_mut()) {
            Some(clock) => clock.move_to_position(prefix, fraction, LOCAL_ADVANCE_LIMIT),
            None => return,
        };
        if let Some((dirty, _)) = self.resolve_advance(advance) {
            self.apply_local_advance(&dirty);
        }
    }

    /// Put the display at a motion. The warnings list uses this, and it takes
    /// the same path as any other jump: local when the clock can reach it, one
    /// exact seek when it cannot.
    fn seek_motion(&mut self, motion: usize) {
        let seconds = self
            .stock
            .as_ref()
            .and_then(|stock| stock.clock.as_ref())
            .map(|clock| clock.time().seconds_at_prefix(motion));
        match seconds {
            Some(seconds) => self.seek_seconds(seconds),
            // No clock (a plan whose timing cannot be derived): fall back to the
            // motion-index seek the rest of the transport uses.
            None => self.stock_seek(motion),
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
            // The ramp spans the cut, not the stock: an explicit range in
            // millimetres if the style gives one, otherwise the deepest cut in
            // the displayed state.
            let thickness = stock.meta.stock.thickness_mm.max(1e-9);
            let span = self
                .stock_style
                .ramp_range_mm
                .map(|mm| mm as f64 / thickness)
                .or_else(|| self.wall_cache.as_ref().map(|cache| cache.deepest as f64))
                .unwrap_or(1.);
            uniform.ramp_top = 0.;
            uniform.ramp_bottom = span.clamp(1e-6, 1.) as f32;
        }
        (uniform, palette, self.palette_revision)
    }

    /// The scene pass's camera uniform, with the two path filters in its spare
    /// slots: cutting moves and travel moves are filtered in the shader by the
    /// alpha `scene.rs` writes on each motion.
    fn scene_camera(&self, rect: egui::Rect) -> [f32; 8] {
        let mut camera = self.camera(rect).uniform();
        camera[6] = f32::from(self.stock_style.show_cutting);
        camera[7] = f32::from(self.stock_style.show_travel);
        camera
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
        let raster = stock.raster_revision;
        if let Some(cache) = &self.wall_cache
            && cache.identity == identity
            && cache.raster == raster
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
        let deepest = stock_walls::max_depth(&view) as f32;
        let revision = self.wall_revision.wrapping_add(1);
        let walls = Arc::new(built.walls);
        self.wall_report = (built.dropped > 0)
            .then_some((built.dropped, built.threshold_mm))
            .or(Some((0, built.threshold_mm)));
        self.wall_cache = Some(WallCache {
            identity,
            raster,
            threshold: threshold.to_bits(),
            section,
            deepest,
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
        // The marker and the trail read the same paged motion stream the picker
        // indexes, so the picker is built on first use — by a pick, or by the
        // first animated frame.
        self.ensure_picker();
        let (playhead, fraction) = self
            .stock
            .as_ref()
            .and_then(|stock| stock.clock.as_ref())
            .map_or((self.playhead, 0.), |clock| clock.position());
        let signature = (
            self.scene_revision,
            self.selection.map(|pick| pick.motion),
            (playhead, fraction.to_bits()),
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
        // Where the cutter is now, and the part of its move it has already
        // made. Inside a move the tip interpolates along it, so the drawn path
        // and the removal agree frame by frame instead of jumping at motion
        // boundaries. Nothing here is CAM geometry.
        let picker = self.picker.as_ref();
        let (marker, trail, plan_tip) = match scene.meta.sim.as_ref().and_then(|sim| {
            let picker = picker?;
            if playhead == 0 && fraction <= 0. {
                return None;
            }
            let index = self.marker_index()?;
            let (tip, trail, plan_tip) = if fraction > 0. {
                let points = picker.endpoints(playhead as u32)?;
                let (start, end) = (points[0], points[1]);
                // Inside an arc the tip follows the curve, exactly as the
                // material sweep does; the two must not disagree. The curve is
                // in plan millimetres, so it takes the same normalization the
                // motion vertices took to become scene coordinates.
                let (tip, plan_tip) = match self.clock_motion() {
                    Some(motion) => {
                        let point = motion.point_at(fraction);
                        (
                            crate::compute::scene_point(point, scene.meta.bounds),
                            Some(point),
                        )
                    }
                    None => {
                        let lerp = |a: f32, b: f32| a + (b - a) * fraction as f32;
                        (
                            [
                                lerp(start[0], end[0]),
                                lerp(start[1], end[1]),
                                lerp(start[2], end[2]),
                            ],
                            None,
                        )
                    }
                };
                (tip, Some((start, tip)), plan_tip)
            } else {
                // On a motion boundary the tip is the end of the last move.
                let points = picker.endpoints(index as u32)?;
                (points[1], None, None)
            };
            // The cutter body follows the stage that owns this motion, and a
            // knife turns with the blade heading the plan modeled for it.
            let motion_tool = self.group_tool_at(index).unwrap_or(0);
            let tool = sim.tools.get(motion_tool).copied()?;
            let heading_deg = self.knife_heading(index, fraction as f32);
            let assembly = sim.assemblies.get(motion_tool).copied().unwrap_or_default();
            Some((
                overlay::Marker {
                    tool,
                    scale: scale as f32,
                    tip,
                    top: 0.,
                    heading_deg,
                    assembly,
                    holder: (!self.holder_body.is_empty()).then(|| self.holder_body.as_slice()),
                },
                trail,
                plan_tip,
            ))
        }) {
            Some((marker, trail, plan_tip)) => (Some(marker), trail, plan_tip),
            None => (None, None, None),
        };
        let half = self.pick_tolerance_px * 0.5 / ppp.max(1e-3) / per_unit.max(1e-6);
        let mut built = overlay::build(selection, trail, half, marker.as_slice());
        self.marker_tip = marker.as_ref().map(|marker| marker.tip);
        self.marker_plan_tip = plan_tip;
        self.knife_overlay(&mut built);
        self.profile_overlay(&mut built);
        self.face_overlay(&mut built);
        self.overlay_lines = Arc::new(built.lines);
        self.overlay_triangles = Arc::new(built.triangles);
    }

    /// Build the motion endpoint index if it is not there yet. Picking, the
    /// cutter marker and the in-flight trail all read the same paged stream, so
    /// one index serves all three and is built at most once per scene.
    fn ensure_picker(&mut self) {
        if self.picker.is_some() {
            return;
        }
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        self.picker = Picker::from_vertex_bytes(scene.motion_bytes()).ok();
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

/// A program time as a clock: `1:05:03` past an hour, `5:03` below it. The
/// transport reports machine time, so it reads like one.
fn format_program_time(seconds: f64) -> String {
    let total = seconds.max(0.).round() as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
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

    /// The cutter and its trail are drawn in the same scene space as the motion
    /// vertices. A tip left in plan millimetres is tens of scene units from the
    /// path it belongs to — the marker is nowhere on screen and its trail runs
    /// out of the viewport, which is how the report from `real_data/
    /// flower_lagging` read. Both are measured here against the move the plan
    /// itself draws, so an invented position cannot pass.
    #[test]
    fn the_cutter_is_drawn_where_its_own_move_is() {
        // The arc fit is opt-in through the job's tolerances, and an arc is the
        // move whose tip is furthest from its chord.
        let mut job: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/gui2/flower.job.json"))
                .expect("a job document");
        let budget = ["motion_tolerance_mm", "verification_tolerance_mm"]
            .iter()
            .filter_map(|key| job["tolerances"].get(*key).and_then(|v| v.as_f64()))
            .fold(f64::INFINITY, f64::min);
        job["tolerances"]["arc_fit_tolerance_mm"] = serde_json::json!(budget.min(0.005));
        let job = job.to_string();
        let (meta, payload) = crate::session::execute(
            &mut cam_service::retained::Retained::new(),
            crate::session::Command::generate(job),
        )
        .expect("the fixture generates");
        let scene = Scene {
            meta,
            payload: Arc::new(payload),
        };
        let size = (scene.meta.bounds[2] - scene.meta.bounds[0])
            .max(scene.meta.bounds[3] - scene.meta.bounds[1])
            .max(0.001);
        // Scene units per millimetre, the factor every drawn vertex carries.
        let per_mm = 1.6 / size;
        let mut view = Viewport::default();
        view.set_scene(scene);
        // Stand halfway inside a motion the plan programs as an arc: that is
        // where a tip that skipped the normalization is furthest off the move.
        let mut stood = None;
        {
            let clock = view
                .stock
                .as_mut()
                .and_then(|stock| stock.clock.as_mut())
                .expect("the fixture carries a playback clock");
            // The transport hands the clock the state the display opens on —
            // the end of the program — so walk forward from the start.
            clock.restore(0, 0.).expect("the program's own start");
            for prefix in 0..clock.motions() {
                clock.move_to_position(prefix, 0.5, LOCAL_ADVANCE_LIMIT);
                let motion = clock.motion().expect("the position names a motion").clone();
                if motion.arc.is_some() {
                    stood = Some((prefix, motion));
                    break;
                }
            }
        }
        let (prefix, motion) = stood.expect("the fixture programs an arc");
        view.build_overlay(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 900.)),
            1.,
        );
        let start = view
            .picker
            .as_ref()
            .and_then(|picker| picker.endpoints(prefix as u32))
            .expect("every motion is indexed")[0];
        let corner = |v: &Vertex| {
            (((v.position[0] - start[0]).powi(2) + (v.position[1] - start[1]).powi(2)) as f64)
                .sqrt()
        };
        // What may legitimately stand outside the part of the move already
        // made: the ribbon's own half width, and the cutter's radius around its
        // tip. Both are fractions of a millimetre in scene units; a tip in
        // millimetres is two orders of magnitude over this and fails loudly.
        let made = motion.length_mm() * per_mm;
        let trail: Vec<&Vertex> = view
            .overlay_triangles
            .iter()
            .filter(|v| v.color == overlay::TRAIL_LINE)
            .collect();
        assert_eq!(trail.len(), 6, "one ribbon for the part of the move made");
        for vertex in &trail {
            assert!(
                corner(vertex) <= made + 0.05,
                "the trail is drawn {:.3} scene units from the move it cuts, \
                 which is {made:.3} long",
                corner(vertex)
            );
        }
        let radius_mm = view
            .scene
            .as_ref()
            .and_then(|scene| scene.meta.sim.as_ref())
            .and_then(|sim| sim.tools.get(motion.tool))
            .and_then(|tool| tool.profile())
            .map_or(0., |profile| {
                profile.iter().map(|(_, radius)| *radius).fold(0., f64::max)
            });
        let body: Vec<&Vertex> = view
            .overlay_triangles
            .iter()
            .filter(|v| v.color == overlay::SELECT_FILL_ENDMILL)
            .collect();
        assert!(!body.is_empty(), "the cutter's own body is drawn");
        for vertex in &body {
            assert!(
                corner(vertex) <= made + 0.05 + radius_mm * per_mm,
                "the cutter body is drawn {:.3} scene units from its own move",
                corner(vertex)
            );
        }
    }

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

    #[test]
    fn knife_shift_click_extends_live_selection_while_the_scene_is_retained() {
        let job = include_str!("../../../fixtures/gui6/knife.job.json").to_owned();
        let scene = crate::session::execute(
            &mut cam_service::retained::Retained::new(),
            crate::session::Command::Preview { job },
        )
        .unwrap();
        let mut view = Viewport::default();
        view.load_scene(Ok(scene));
        view.set_knife_selected(true);
        view.artwork.enabled = true;
        view.camera.set_tilt(0.);
        view.camera.yaw = 0.;
        let first = view
            .knife_chains
            .iter()
            .find(|c| !c.closed)
            .unwrap()
            .reference
            .clone();
        let closed = view.knife_chains.iter().find(|c| c.closed).unwrap().clone();
        // The retained preview selected both chains. Authoring has since
        // selected just the first one without replacing that scene.
        view.artwork.selected = vec![first.clone()];
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&mut view, &ctx, vec![]);
        }
        let rect = viewport_rect();
        let a = closed.vertices[0];
        let b = closed.vertices[1];
        let point = crate::artwork_view::screen_point(
            view.camera(rect),
            view.scene.as_ref().unwrap().meta.bounds,
            rect,
            cam_core::geometry::Point::new((a[0] + b[0]) / 2., (a[1] + b[1]) / 2.),
        );
        let modifiers = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        for count in [2, 1] {
            frame_with(
                &mut view,
                &ctx,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ],
                modifiers,
            );
            frame_with(
                &mut view,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                }],
                modifiers,
            );
            let events = view.take_artwork_events();
            let Some(ArtworkEvent::KnifeSelection(selection)) = events.last() else {
                panic!("Knife click did not emit a selection");
            };
            assert_eq!(selection.len(), count);
            assert!(selection.contains(&first));
            assert_eq!(selection.contains(&closed.reference), count == 2);
            view.artwork.selected = selection.clone();
        }
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
        view.stock_style.show_cutting = false;
        view.stock_style.show_artwork = false;
        let probe = view.display_probe();
        assert_eq!(probe["style"]["appearance"], "xray");
        assert_eq!(probe["style"]["showCutting"], false);
        assert_eq!(probe["style"]["showTravel"], true);
        assert_eq!(probe["style"]["showEdges"], false);
        assert_eq!(probe["style"]["showArtwork"], false);
        assert_eq!(probe["style"]["surface"], "by_operation");
    }

    /// S2: the knife turns the way the plan says it turns, including across the
    /// 0/360 seam, so a corner swivel reads as a turn rather than a spin.
    #[test]
    fn the_knife_heading_turns_the_short_way_round() {
        let view = Viewport {
            knife_headings: Arc::new(vec![Some((350., 10.)), Some((0., 180.))]),
            ..Default::default()
        };
        // Across the seam the short way is 20 degrees, through 0.
        assert!((view.knife_heading(0, 0.).unwrap() - 350.).abs() < 1e-3);
        assert!(view.knife_heading(0, 0.5).unwrap().abs() < 1e-3);
        assert!((view.knife_heading(0, 1.).unwrap() - 10.).abs() < 1e-3);
        // A half turn has no short way; the signed remainder picks the negative
        // one, and the canonical form reports it as 270 degrees.
        assert!((view.knife_heading(1, 0.5).unwrap() - 270.).abs() < 1e-3);
        // A motion the plan modeled no heading for draws no rotation.
        assert!(view.knife_heading(2, 0.5).is_none());
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
            clock: None,
            raster_revision: 0,
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
        view.stock_style.show_cutting = false;
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
        assert!(!restored.stock_style.show_cutting);
        assert_eq!(
            restored.stock_style.overrides.get("op-a"),
            Some(&[1., 0., 0.])
        );
    }
}
