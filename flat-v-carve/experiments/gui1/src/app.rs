use crate::{
    compute::{Request, Scene, SceneMeta, Vertex},
    overlay, pages,
    pick::{self, Camera, Picker},
    platform::{Event, IoValue, Port},
    recovery::{Snapshot, Tracker},
    render, sim,
    state::{Draft, FIELDS},
    stock_preview::PreviewMeta,
    stock_render,
};
use egui::{Color32, RichText};
use std::sync::Arc;

/// Replay checkpoints retained by the display process. The transported frames
/// seed this budget, so an interactive seek replays a bounded suffix.
const STOCK_CHECKPOINT_BUDGET: usize = 20 * 1024 * 1024;
/// Pages fingerprinted per frame while a new scene settles.
const HASH_PAGES_PER_FRAME: usize = 4;
/// Everything the overlay geometry depends on; a change rebuilds it.
type OverlaySignature = (u64, Option<u32>, usize, usize, i32, i32, i32);

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
    ranges: Vec<std::ops::Range<usize>>,
    versions: Vec<Arc<Vec<u32>>>,
    playback: Option<(sim::Playback, Vec<sim::Motion>)>,
    /// Current displayed cells: either a transported checkpoint range in the
    /// payload or a locally re-integrated field for a seek between checkpoints.
    range: std::ops::Range<usize>,
    local: Option<Arc<Vec<u8>>>,
    cell_versions: Arc<Vec<u32>>,
    stats: sim::Stats,
    prefix: usize,
}

impl StockView {
    fn checkpoint_for(&self, prefix: usize) -> usize {
        self.meta
            .frames
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.prefix <= prefix)
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0)
    }
}

pub struct App {
    pub draft: Draft,
    pub search: String,
    port: Port,
    generation: u64,
    active: Option<u64>,
    pub status: String,
    scene: Option<Scene>,
    // Viewport and selection
    iso: bool,
    zoom: f32,
    yaw: f32,
    playhead: usize,
    playing: bool,
    stage: usize,
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
    stock_step: usize,
    seek_ms: f64,
    show_stock: bool,
    // Risk probes
    gpu_unavailable: bool,
    drill: render::Drill,
    error_probe_baseline: u64,
    deny_save: bool,
    gpu: bool,
    pub operation_count: usize,
    pub laid_out_rows: usize,
    last_frame: Option<f64>,
    frame_ms: std::collections::VecDeque<f64>,
    pub recovery: Tracker,
    recovery_job: Option<String>,
    io_id: u64,
    io_pending: Option<(Option<egui::Id>, u64)>,
    focus_after_modal: Option<egui::Id>,
    retained_save: Option<(String, Vec<u8>)>,
    retry_save: bool,
    field_focus: std::collections::BTreeMap<(u64, u64), usize>,
    focus_field: Option<usize>,
    ime_active: bool,
    offline_status: String,
    pub render_stats: render::SharedStats,
    pub stock_stats: stock_render::SharedStats,
    pub heap: crate::alloc_probe::Counters,
    scene_revision: u64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            draft: Draft::default(),
            search: String::new(),
            port: Port::default(),
            generation: 0,
            active: None,
            status: "Experimental GUI1. Choose a reference to calculate.".into(),
            scene: None,
            iso: false,
            zoom: 1.,
            yaw: 0.,
            playhead: 0,
            playing: false,
            stage: 0,
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
            stock_step: 0,
            seek_ms: 0.,
            show_stock: false,
            gpu_unavailable: false,
            drill: render::Drill::None,
            error_probe_baseline: 0,
            deny_save: false,
            gpu: false,
            operation_count: 10,
            laid_out_rows: 0,
            last_frame: None,
            frame_ms: Default::default(),
            recovery: Tracker::default(),
            recovery_job: None,
            io_id: 0,
            io_pending: None,
            focus_after_modal: None,
            retained_save: None,
            retry_save: false,
            field_focus: Default::default(),
            focus_field: None,
            ime_active: false,
            offline_status: String::new(),
            render_stats: Arc::new(std::sync::Mutex::new(render::Stats::default())),
            stock_stats: Arc::new(std::sync::Mutex::new(stock_render::Stats::default())),
            heap: crate::alloc_probe::Counters::default(),
            scene_revision: 0,
        }
    }
}

impl App {
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
        let job = if scene.meta.job.is_empty() {
            None
        } else {
            Some(scene.meta.job.clone())
        };
        if self.recovery_job != job {
            self.recovery_job = job;
            self.recovery.changed(self.last_frame.unwrap_or(0.));
        }
        self.playhead = scene.motion_count();
        self.stock_prefix = scene.motion_count();
        self.selection = None;
        self.picker = None;
        self.overlay_signature = None;
        let stock = self.build_stock(&scene);
        self.stock = stock;
        self.page_hashes = Arc::new(vec![None; page_table(&scene).page_count()]);
        self.hash_cursor = 0;
        let required = self.compute_required(&scene);
        self.required_pages = required;
        self.stock_step = self
            .stock
            .as_ref()
            .map_or(0, |stock| stock.meta.frames.len().saturating_sub(1));
        self.show_stock = self.stock.is_some();
        self.playing = false;
        self.scene_revision += 1;
        self.scene = Some(scene);
    }

    fn build_stock(&mut self, scene: &Scene) -> Option<StockView> {
        let meta = scene.meta.stock.clone()?;
        let identity = scene.identity();
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
        let mut playback = None;
        let mut cells = ranges.last()?.clone();
        let mut stats = meta.frames.last()?.stats.clone();
        let mut prefix = meta.frames.last()?.prefix;
        if let Ok(Some(input)) = scene.sim_input() {
            let mut seed = Vec::with_capacity(meta.frames.len());
            for (index, frame) in meta.frames.iter().enumerate() {
                let field = sim::Field::from_packed(
                    meta.stock,
                    &input.tools,
                    meta.cell_mm,
                    &scene.payload[ranges[index].clone()],
                    &frame.versions,
                    &frame.allocated,
                    frame.stats.clone(),
                )
                .ok()?;
                seed.push((frame.prefix, field));
            }
            let pristine = seed[0].1.clone();
            let field = seed.last()?.1.clone();
            let checkpointed = sim::Playback::seed(field, pristine, seed, STOCK_CHECKPOINT_BUDGET);
            playback = Some((checkpointed, input.motions));
            cells = ranges[ranges.len() - 1].clone();
            stats = meta.frames[ranges.len() - 1].stats.clone();
            prefix = meta.frames[ranges.len() - 1].prefix;
        }
        Some(StockView {
            cell_versions: versions[versions.len() - 1].clone(),
            versions,
            meta,
            identity,
            ranges,
            playback,
            range: cells,
            local: None,
            stats,
            prefix,
        })
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
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
            app.status = format!("Experimental GUI1 · {:?}", state.adapter.get_info());
        }
        app.recovery.enabled = true;
        app.recovery.status = "Checking local recovery…".into();
        app.port.load_recovery(cc.egui_ctx.clone());
        app
    }

    pub fn motion_count(&self) -> usize {
        self.scene.as_ref().map_or(0, Scene::motion_count)
    }

    /// Pages the display needs, nearest the playhead first. The renderer admits
    /// this order until the resident budget is reached.
    fn compute_required(&self, scene: &Scene) -> Vec<usize> {
        let table = page_table(scene);
        if table.motions == 0 {
            return Vec::new();
        }
        let rough = scene.meta.rough_vertices / 2;
        let playhead = self.playhead.min(table.motions);
        let (mut start, mut end) = (0, playhead.max(1));
        match self.stage {
            1 => end = end.min(rough.max(1)),
            2 => start = rough.min(table.motions),
            _ => {}
        }
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

    fn begin_io(&mut self, focus: Option<egui::Id>) -> u64 {
        self.io_id += 1;
        self.io_pending = Some((focus, self.recovery.edit));
        self.io_id
    }

    fn open_dialog(&mut self, recovery: bool, focus: Option<egui::Id>, ctx: &egui::Context) {
        if self.io_pending.is_some() {
            return;
        }
        let id = self.begin_io(focus);
        self.port.open(id, recovery, ctx.clone());
    }

    fn save_bytes(
        &mut self,
        name: String,
        bytes: Vec<u8>,
        focus: Option<egui::Id>,
        ctx: &egui::Context,
    ) {
        if self.io_pending.is_some() {
            return;
        }
        self.retained_save = Some((name, bytes));
        self.retry_retained(focus, ctx);
    }

    fn retry_retained(&mut self, focus: Option<egui::Id>, ctx: &egui::Context) {
        if self.io_pending.is_some() {
            return;
        }
        if let Some((name, bytes)) = self.retained_save.clone() {
            let id = self.begin_io(focus);
            self.retry_save = true;
            self.port.save(id, name, bytes, self.deny_save, ctx.clone());
        }
    }

    fn restore_context_focus(&mut self) {
        self.focus_field = self
            .field_focus
            .get(&(self.draft.selected_source, self.draft.operation))
            .copied();
        if self.focus_field.is_some() {
            self.search.clear();
        }
    }

    fn restore_session(&mut self, snapshot: Snapshot, now: f64, ctx: &egui::Context) {
        // Restoring an empty session must also retire a scene/request opened
        // while the recovery offer was visible. No derived authority survives.
        self.generation += 1;
        self.active = None;
        self.port.cancel();
        self.scene = None;
        self.stock = None;
        self.picker = None;
        self.selection = None;
        self.show_stock = false;
        self.playing = false;
        self.playhead = 0;
        self.scene_revision += 1;
        self.draft = snapshot.draft;
        self.operation_count = self.operation_count.max(self.draft.operation as usize);
        self.recovery_job = snapshot.job.clone();
        self.recovery.changed(now);
        self.recovery.saved_edit = self.recovery.edit;
        self.recovery.status = "Session restored; derived results will be recalculated".into();
        self.status = self.recovery.status.clone();
        if let Some(json) = snapshot.job {
            self.start(Request::Open { json }, ctx);
        }
    }

    fn finish_io(&mut self, id: u64, result: Result<IoValue, String>, ctx: &egui::Context) {
        if id != self.io_id {
            return;
        }
        let Some((focus, edit)) = self.io_pending.take() else {
            return;
        };
        match result {
            Ok(IoValue::Job(json)) => self.start(Request::Open { json }, ctx),
            Ok(IoValue::Draft(draft)) => {
                if self.recovery.edit != edit {
                    self.status =
                        "Recovery result arrived after newer edits; current draft retained".into();
                } else {
                    self.draft = draft;
                    self.operation_count = self.operation_count.max(self.draft.operation as usize);
                    self.recovery.changed(ctx.input(|i| i.time));
                    self.status =
                        "Recovered raw inspector draft; no machining settings changed".into();
                }
            }
            Ok(IoValue::Saved(message)) => {
                self.retry_save = false;
                self.status = message;
            }
            Err(error) => {
                self.status = format!("{error}. Current draft and any pending save bytes retained.")
            }
        }
        // The previous modal layer remains active through this frame. Restore
        // only once egui has retired it, otherwise the field is still disabled.
        self.focus_after_modal = focus;
    }

    fn start(&mut self, request: Request, ctx: &egui::Context) {
        self.generation += 1;
        self.active = Some(self.generation);
        self.status =
            "Computing off the UI thread — previous result retained, not current for this request"
                .into();
        self.port.start(self.generation, request, ctx.clone());
    }

    fn poll(&mut self, ctx: &egui::Context) {
        // Bounded event draining. Identity gates prevent a late result replacing a newer request.
        for _ in 0..4 {
            let Some(event) = self.port.poll() else {
                break;
            };
            match event {
                Event::Notice(message) => self.status = message,
                Event::OfflineStatus(status) => self.offline_status = status,
                Event::Computed {
                    id,
                    elapsed_ms,
                    result,
                } if self.active == Some(id) => {
                    self.active = None;
                    match result {
                        Ok((meta, payload)) => {
                            let transport = &meta.transport;
                            self.status = format!(
                                "Calculation finished in {elapsed_ms:.1} ms · {} motions · {} pages · metadata {} B + payload {} B · engine status {} · output programs {}",
                                meta.motions,
                                transport.motion_pages,
                                transport.metadata_bytes,
                                transport.payload_bytes,
                                meta.report
                                    .pointer("/summary/status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("synthetic"),
                                meta.programs.len()
                            );
                            if let Some(check) = meta
                                .report
                                .pointer("/export/status")
                                .and_then(|v| v.as_str())
                            {
                                self.status = format!(
                                    "Output check: {check}; {} checked programs. Reference settings were not changed.",
                                    meta.programs.len()
                                );
                            }
                            if let Some(error) = meta
                                .report
                                .get("stockPreviewError")
                                .and_then(|v| v.as_str())
                            {
                                self.status
                                    .push_str(&format!(" · Stock preview unavailable: {error}"));
                            }
                            self.load_scene(Ok((meta, payload)));
                        }
                        Err(e) => self.status = e,
                    }
                }
                Event::Cancelled { id, stop_ms }
                    if id + 1 == self.generation && self.active.is_none() =>
                {
                    self.status = if cfg!(target_arch = "wasm32") {
                        format!(
                            "Worker.terminate returned in {stop_ms:.2} ms; actual CPU stop latency is unmeasured. Previous result retained."
                        )
                    } else {
                        format!(
                            "Cancelled; child stopped/reaped in {stop_ms:.2} ms after supervisor observed cancellation. Previous result retained."
                        )
                    };
                }
                Event::Io { id, result } => self.finish_io(id, result, ctx),
                Event::RecoveryLoaded(result) => match result {
                    Ok(record) => {
                        self.recovery.revision = record.as_ref().map(|r| r.revision);
                        self.recovery.offered = record;
                        self.recovery.ready = true;
                        self.recovery.failed = false;
                        self.recovery.status = if self.recovery.offered.is_some() {
                            "Recovery available; choose Restore session or Keep current".into()
                        } else {
                            "Local recovery ready".into()
                        };
                    }
                    Err(error) => {
                        self.recovery.failed = true;
                        self.recovery.ready = false;
                        self.recovery.status =
                            format!("Recovery unavailable: {error}. Current draft retained.");
                    }
                },
                Event::RecoverySaved { edit, result } => self.recovery.written(edit, result),
                _ => {}
            }
        }
    }

    /// Move the stock display to an arbitrary motion prefix. A transported
    /// checkpoint is instant; between checkpoints the bounded replay restores
    /// the nearest checkpoint and re-integrates only the suffix.
    fn stock_seek(&mut self, target: usize) {
        let motions = self.motion_count();
        let Some(stock) = self.stock.as_mut() else {
            return;
        };
        let target = target.min(motions);
        if stock.prefix == target {
            return;
        }
        let mut note = None;
        let mut failed = false;
        let timer = crate::clock::Timer::start();
        let checkpoint = stock.checkpoint_for(target);
        let exact = stock.meta.frames[checkpoint].prefix == target;
        if exact {
            stock.range = stock.ranges[checkpoint].clone();
            stock.cell_versions = stock.versions[checkpoint].clone();
            stock.stats = stock.meta.frames[checkpoint].stats.clone();
            stock.local = None;
        } else if let Some((playback, motions)) = stock.playback.as_mut() {
            match playback.seek(motions, target) {
                Ok(()) => {
                    let cells = playback.field.packed_tile_bytes();
                    stock.cell_versions = Arc::new(playback.field.versions.clone());
                    stock.stats = playback.field.stats.clone();
                    stock.local = Some(Arc::new(cells));
                }
                Err(error) => {
                    note = Some(format!(
                        "Stock seek refused: {error}. Previous cells retained."
                    ));
                    failed = true;
                }
            }
        } else {
            // No replay stream was transported: snap to the nearest checkpoint.
            stock.range = stock.ranges[checkpoint].clone();
            stock.cell_versions = stock.versions[checkpoint].clone();
            stock.stats = stock.meta.frames[checkpoint].stats.clone();
            stock.local = None;
        }
        if !failed {
            stock.prefix = target;
            self.stock_step = stock.checkpoint_for(target);
            self.seek_ms = timer.elapsed_ms();
            self.playhead = target;
        }
        if let Some(note) = note {
            self.status = note;
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
        self.selection = picker.pick(&camera, rect_points, local, self.pick_tolerance_px, ppp);
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
            iso: self.iso,
            aspect: (rect.width() / rect.height().max(1.)).max(0.2),
            zoom: self.zoom,
            yaw: self.yaw,
        }
    }

    /// Selection fill and blade marker in the same normalized scene space as
    /// the motion vertices. Nothing here is CAM geometry.
    fn build_overlay(&mut self, rect: egui::Rect, ppp: f32) {
        let signature = (
            self.scene_revision,
            self.selection.map(|pick| pick.motion),
            self.playhead,
            self.stage,
            (self.zoom * 100.) as i32,
            rect.width() as i32,
            rect.height() as i32,
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
        let per_unit = self.zoom * rect.height().max(1.) / 2.;
        let selection = self.selection.and_then(|pick| {
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
                let motion_tool = scene
                    .meta
                    .report
                    .get("roughingMotions")
                    .and_then(|v| v.as_u64())
                    .map_or(0, |rough| if index < rough as usize { 0 } else { 1 });
                let tool = sim.tools.get(motion_tool).copied()?;
                let tip = [points[1][0], points[1][1], points[1][2]];
                Some(match tool {
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
        let built = overlay::build(selection, half, marker.as_slice());
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

    pub fn ui(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        self.heap = crate::alloc_probe::snapshot();
        self.fingerprint_pages();
        if let Some(id) = self.focus_after_modal {
            if ctx.memory(|m| m.top_modal_layer().is_none()) {
                ctx.memory_mut(|m| m.request_focus(id));
                self.focus_after_modal = None;
            } else {
                ctx.request_repaint();
            }
        }
        let now = ctx.input(|i| i.time);
        let mut draft_changed = false;
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Ime(event) = e {
                    match event {
                        egui::ImeEvent::Preedit(text) => self.ime_active = !text.is_empty(),
                        egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => {
                            self.ime_active = false
                        }
                        _ => {}
                    }
                }
            }
        });
        if self.io_pending.is_none() && !self.ime_active {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.open_dialog(false, ctx.memory(|m| m.focused()), ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.save_bytes(
                    "gui1-raw-draft.json".into(),
                    serde_json::to_vec_pretty(&self.draft).unwrap(),
                    ctx.memory(|m| m.focused()),
                    ctx,
                );
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("setting-search")));
            }
        }
        egui::TopBottomPanel::top("commands").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("FLAT / V").strong().size(18.));
                ui.label(RichText::new("GUI1 EXPERIMENT").color(Color32::from_rgb(235, 169, 79)));
                ui.separator();
                if ui.button("Small combined").clicked() {
                    self.start(
                        Request::Reference {
                            flower: false,
                            export: false,
                        },
                        ctx,
                    );
                }
                if ui.button("Flower reference").clicked() {
                    self.start(
                        Request::Reference {
                            flower: true,
                            export: false,
                        },
                        ctx,
                    );
                }
                let open = ui.add_enabled(self.io_pending.is_none(), egui::Button::new("Open job"));
                if open.clicked() {
                    self.open_dialog(false, Some(open.id), ctx);
                }
                if ui.button("Busy worker").clicked() {
                    self.start(Request::Busy, ctx);
                }
                if ui
                    .add_enabled(self.active.is_some(), egui::Button::new("Cancel"))
                    .clicked()
                {
                    self.port.cancel();
                    self.generation += 1;
                    self.active = None;
                    self.status =
                        "Cancellation requested; waiting for actual stopped-work confirmation"
                            .into();
                }
            });
            ui.horizontal(|ui| {
                ui.menu_button("Risk probes", |ui| {
                    for (label, segments, ops) in [
                        ("S · 20,000", 20_000, 10),
                        ("M · 200,000", 200_000, 100),
                        ("L · 1,000,000", 1_000_000, 1000),
                    ] {
                        if ui.button(label).clicked() {
                            self.operation_count = ops;
                            self.start(Request::Synthetic { segments }, ctx);
                            ui.close();
                        }
                    }
                    if ui.button("Crash compute child").clicked() {
                        self.start(Request::Crash, ctx);
                        ui.close();
                    }
                    if ui.button("Inject renderer failure").clicked() {
                        self.gpu_unavailable = true;
                        ui.close();
                    }
                    if ui.button("Resume injected renderer").clicked() {
                        self.gpu_unavailable = false;
                        self.scene_revision += 1;
                        self.overlay_signature = None;
                        ui.close();
                    }
                    if ui.button("GPU recovery drill").clicked() {
                        self.drill = render::Drill::Recover;
                        ui.close();
                    }
                    if ui.button("Inject GPU validation error").clicked() {
                        self.drill = render::Drill::InjectError;
                        self.error_probe_baseline =
                            self.render_stats.lock().unwrap().injected_errors;
                        ui.close();
                    }
                    ui.separator();
                    for (label, bytes) in [
                        ("GPU page budget 16 MiB", 16 * 1024 * 1024),
                        ("GPU page budget 64 MiB", 64 * 1024 * 1024),
                        ("GPU page budget 256 MiB", 256 * 1024 * 1024),
                    ] {
                        if ui.button(label).clicked() {
                            self.page_budget = bytes;
                            ui.close();
                        }
                    }
                    ui.checkbox(&mut self.deny_save, "Deny next save (injected)");
                });
                let save = ui.add_enabled(
                    self.io_pending.is_none(),
                    egui::Button::new("Save raw draft"),
                );
                if save.clicked() {
                    let bytes = serde_json::to_vec_pretty(&self.draft).expect("draft serializes");
                    self.save_bytes("gui1-raw-draft.json".into(), bytes, Some(save.id), ctx);
                }
                let recover = ui.add_enabled(
                    self.io_pending.is_none(),
                    egui::Button::new("Recover raw draft"),
                );
                if recover.clicked() {
                    self.open_dialog(true, Some(recover.id), ctx);
                }
                if ui.button("Copy status").clicked() {
                    ctx.copy_text(self.status.clone());
                }
                if ui.button("Scale −").clicked() {
                    ctx.set_zoom_factor((ctx.zoom_factor() - 0.1).max(0.7));
                }
                if ui.button("Scale +").clicked() {
                    ctx.set_zoom_factor((ctx.zoom_factor() + 0.1).min(2.));
                }
                ui.label("Prepare / motion inspection");
            });
            if self.recovery.enabled {
                ui.horizontal_wrapped(|ui| {
                    if !self.offline_status.is_empty() {
                        ui.label(&self.offline_status);
                    }
                    ui.label(&self.recovery.status);
                    if self.recovery.offered.is_some() {
                        if ui.button("Restore session").clicked() {
                            let stored = self.recovery.offered.take().unwrap();
                            self.restore_session(stored.snapshot, now, ctx);
                        }
                        if ui.button("Keep current").clicked() {
                            self.recovery.offered = None;
                            self.recovery.changed(now);
                        }
                    } else if self.recovery.failed {
                        if self.recovery.ready && ui.button("Retry recovery write").clicked() {
                            self.recovery.failed = false;
                        }
                        if ui.button("Reload recovery").clicked() {
                            self.recovery.ready = false;
                            self.port.load_recovery(ctx.clone());
                        }
                    }
                    if self.recovery.edit != self.recovery.saved_edit {
                        ui.label("Unsaved recovery edits");
                    }
                });
            }
            if self.retry_save && self.retained_save.is_some() {
                ui.horizontal(|ui| {
                    let retry = ui.add_enabled(
                        self.io_pending.is_none(),
                        egui::Button::new("Retry previous save"),
                    );
                    if retry.clicked() {
                        self.retry_retained(Some(retry.id), ctx);
                    }
                    if let Some((name, bytes)) = &self.retained_save {
                        ui.label(format!("{name} · {} retained bytes", bytes.len()));
                    }
                });
            }
        });
        egui::TopBottomPanel::bottom("timeline")
            .resizable(true)
            .default_height(140.)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(if self.playing { "Pause" } else { "Play" })
                        .clicked()
                    {
                        self.playing = !self.playing;
                    }
                    ui.selectable_value(&mut self.stage, 0, "All stages");
                    ui.selectable_value(&mut self.stage, 1, "Endmill");
                    ui.selectable_value(&mut self.stage, 2, "V-bit");
                    if self.show_stock {
                        let mut checkpoint = self.stock_step;
                        let frames = self
                            .stock
                            .as_ref()
                            .map_or(0, |stock| stock.meta.frames.len());
                        let checkpoint_widget = egui::Slider::new(
                            &mut checkpoint,
                            0..=frames.saturating_sub(1),
                        )
                        .text("Transported checkpoint");
                        if ui.add(checkpoint_widget).changed() {
                            let prefix = self
                                .stock
                                .as_ref()
                                .map(|stock| stock.meta.frames[checkpoint].prefix)
                                .unwrap_or(0);
                            self.stock_prefix = prefix;
                            self.stock_seek(prefix);
                        }
                        let mut prefix = self.stock_prefix;
                        let motions = self.motion_count();
                        let replayable = self
                            .stock
                            .as_ref()
                            .is_some_and(|stock| stock.playback.is_some());
                        let slider_widget =
                            egui::Slider::new(&mut prefix, 0..=motions).text("Stock motion");
                        let slider = ui.add_enabled(replayable, slider_widget);
                        if slider.changed() {
                            self.stock_prefix = prefix;
                            self.stock_seek(prefix);
                        }
                        if !replayable {
                            ui.label(
                                RichText::new(
                                    "Seek beyond checkpoints is unavailable for this workload",
                                )
                                .small()
                                .color(Color32::from_rgb(235, 169, 79)),
                            );
                        }
                    } else {
                        let motions = self.motion_count();
                        ui.add(egui::Slider::new(&mut self.playhead, 0..=motions).text("Motion playhead"));
                    }
                });
                ui.label(RichText::new(&self.status).color(Color32::from_rgb(210, 183, 131)));
                if let Some(stock) = &self.stock
                    && self.show_stock
                {
                    ui.label(format!(
                        "Stock preview · {:.4} mm cells (reference {:.4}) · motion {} · {:.2} mm³ removed · {} transported checkpoints · seek {:.2} ms",
                        stock.meta.cell_mm,
                        stock.meta.reference_cell_mm,
                        stock.prefix,
                        stock.stats.removed_volume_mm3,
                        stock.meta.frames.len(),
                        self.seek_ms
                    ));
                } else {
                    ui.label("Motion display only. Stock preview uses discrete checkpoints; playback does not establish export eligibility.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Prepare checked small reference").clicked() {
                        self.start(
                            Request::Reference {
                                flower: false,
                                export: true,
                            },
                            ctx,
                        );
                    }
                    if ui.button("Prepare checked flower").clicked() {
                        self.start(
                            Request::Reference {
                                flower: true,
                                export: true,
                            },
                            ctx,
                        );
                    }
                    let checked = ui.add_enabled(
                        self.io_pending.is_none()
                            && self.active.is_none()
                            && self
                                .scene
                                .as_ref()
                                .is_some_and(|scene| !scene.meta.programs.is_empty()),
                        egui::Button::new("Save checked bytes"),
                    );
                    if checked.clicked() {
                        let program = &self.scene.as_ref().unwrap().meta.programs[0];
                        self.save_bytes(
                            program.filename.clone(),
                            program.gcode.as_bytes().to_vec(),
                            Some(checked.id),
                            ctx,
                        );
                    }
                    let job = ui.add_enabled(
                        self.io_pending.is_none()
                            && self
                                .scene
                                .as_ref()
                                .is_some_and(|scene| !scene.meta.job.is_empty()),
                        egui::Button::new("Save reference job"),
                    );
                    if job.clicked() {
                        self.save_bytes(
                            "gui1-reference.job.json".into(),
                            self.scene.as_ref().unwrap().meta.job.as_bytes().to_vec(),
                            Some(job.id),
                            ctx,
                        );
                    }
                });
            });
        egui::SidePanel::left("navigator")
            .resizable(true)
            .default_width(220.)
            .width_range(160.0..=420.0)
            .show(ctx, |ui| {
                ui.heading("Workspace");
                ui.collapsing("Setup", |ui| {
                    ui.label("Stock & work zero");
                    ui.label("Machine: reference profile");
                    ui.label("Tool library: probe only");
                });
                ui.separator();
                ui.label("Artwork · synthetic identity probes");
                if ui.button("Reverse artwork order").clicked() {
                    self.draft.sources.reverse();
                    draft_changed = true;
                    self.restore_context_focus();
                }
                egui::ScrollArea::vertical()
                    .id_salt("artwork-scroll")
                    .max_height(130.)
                    .show(ui, |ui| {
                        for source in self.draft.sources.clone() {
                            ui.push_id(source, |ui| {
                                if ui
                                    .selectable_label(
                                        self.draft.selected_source == source,
                                        format!("flower.svg  ·  #{source}"),
                                    )
                                    .clicked()
                                {
                                    self.draft.selected_source = source;
                                    draft_changed = true;
                                    self.restore_context_focus();
                                }
                            });
                        }
                    });
                ui.separator();
                ui.label("Operations · synthetic list");
                self.laid_out_rows = 0;
                egui::ScrollArea::vertical()
                    .id_salt("operations-scroll")
                    .auto_shrink([false, false])
                    .show_rows(ui, 24., self.operation_count, |ui, rows| {
                        self.laid_out_rows = rows.len();
                        for index in rows {
                            let id = index as u64 + 1;
                            ui.push_id(id, |ui| {
                                if ui
                                    .selectable_label(
                                        self.draft.operation == id,
                                        format!("{id:02}  Flat V-carve"),
                                    )
                                    .clicked()
                                {
                                    self.draft.operation = id;
                                    draft_changed = true;
                                    self.restore_context_focus();
                                }
                            });
                        }
                    });
            });
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(300.)
            .width_range(240.0..=480.0)
            .show(ctx, |ui| {
                ui.heading("Operation inspector");
                ui.label(format!(
                    "Source #{} / Operation {}",
                    self.draft.selected_source, self.draft.operation
                ));
                ui.label(
                    RichText::new("Input probe only — does not edit the reference job")
                        .small()
                        .color(Color32::from_rgb(235, 169, 79)),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search settings")
                        .id(egui::Id::new("setting-search")),
                );
                egui::ScrollArea::vertical()
                    .id_salt("inspector-scroll")
                    .show(ui, |ui| {
                        for (group, name) in [
                            "Cut & stock",
                            "Roughing",
                            "V-bit",
                            "Placement & precision",
                            "Advanced probes",
                        ]
                        .iter()
                        .enumerate()
                        {
                            egui::CollapsingHeader::new(*name)
                                .id_salt(("group", group))
                                .default_open(group == 0)
                                .open(if self.focus_field.is_some_and(|f| f / 8 == group) {
                                    Some(true)
                                } else if self.search.is_empty() {
                                    None
                                } else {
                                    Some(true)
                                })
                                .show(ui, |ui| {
                                    for (field, label) in
                                        FIELDS.iter().enumerate().skip(group * 8).take(8)
                                    {
                                        if !self.search.is_empty()
                                            && !label
                                                .to_lowercase()
                                                .contains(&self.search.to_lowercase())
                                        {
                                            continue;
                                        }
                                        let key = self.draft.key(field);
                                        ui.push_id(&key, |ui| {
                                            let caption = ui.label(*label);
                                            let text =
                                                self.draft.raw.entry(key.clone()).or_default();
                                            ui.horizontal(|ui| {
                                                let response = ui
                                                    .add(
                                                        egui::TextEdit::singleline(text)
                                                            .id(egui::Id::new(("raw-field", &key)))
                                                            .desired_width(150.)
                                                            .char_limit(4096)
                                                            .hint_text("Unset"),
                                                    )
                                                    .labelled_by(caption.id);
                                                if response.changed() {
                                                    draft_changed = true;
                                                }
                                                if self.focus_field == Some(field) {
                                                    response.request_focus();
                                                    self.focus_field = None;
                                                }
                                                if response.has_focus() {
                                                    self.field_focus.insert(
                                                        (
                                                            self.draft.selected_source,
                                                            self.draft.operation,
                                                        ),
                                                        field,
                                                    );
                                                }
                                                ui.label(if field == 39 {
                                                    "text"
                                                } else if label.contains("feed") {
                                                    "mm/min"
                                                } else {
                                                    "probe"
                                                });
                                            });
                                            if field != 39
                                                && let Err(issue) = Draft::parse(text)
                                            {
                                                ui.label(
                                                    RichText::new(issue)
                                                        .small()
                                                        .color(Color32::from_rgb(244, 159, 101)),
                                                );
                                            }
                                        });
                                    }
                                });
                        }
                    });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.iso, false, "Top");
                ui.selectable_value(&mut self.iso, true, "Isometric");
                if ui.button("Fit").clicked() {
                    self.zoom = 1.;
                    self.yaw = 0.;
                }
                ui.add(egui::Slider::new(&mut self.zoom, 0.5..=3.).text("Zoom"));
                if self.stock.is_some() {
                    ui.checkbox(&mut self.show_stock, "Stock preview");
                }
                ui.add(
                    egui::Slider::new(&mut self.pick_tolerance_px, 2.0..=24.0)
                        .text("Pick tolerance (px)"),
                );
            });
            ui.label(
                self.scene
                    .as_ref()
                    .map(|scene| scene.meta.name.as_str())
                    .unwrap_or("Load a combined carving reference"),
            );
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0., Color32::from_rgb(21, 29, 38));
            if response.dragged() {
                self.yaw += response.drag_delta().x * 0.005;
            }
            if response.clicked()
                && let Some(position) = response.interact_pointer_pos()
            {
                self.pick_at(position, rect, ctx.pixels_per_point());
            }
            self.build_overlay(rect, ctx.pixels_per_point());
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
                if self.show_stock
                    && let Some(stock) = &self.stock
                {
                    let bounds = scene.meta.bounds;
                    let size = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]).max(0.001);
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
                                camera: [
                                    if self.iso { 1. } else { 0. },
                                    rect.width() / rect.height().max(1.),
                                    self.zoom,
                                    self.yaw,
                                ],
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
                                    0.,
                                ],
                                drill: stock_drill(self.drill),
                            },
                        ));
                }
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        render::Callback {
                            payload: scene.payload.clone(),
                            identity: scene.identity(),
                            table: page_table(scene),
                            hashes: self.page_hashes.clone(),
                            required: self.required_pages.clone(),
                            budget_bytes: self.page_budget,
                            contour_vertices: scene.meta.contour_vertices,
                            revision: self.overlay_revision,
                            camera: [
                                if self.iso { 1. } else { 0. },
                                rect.width() / rect.height().max(1.),
                                self.zoom,
                                self.yaw,
                            ],
                            lines: self.overlay_lines.clone(),
                            triangles: self.overlay_triangles.clone(),
                            drill: self.drill,
                        },
                    ));
            }
            let pick = self
                .selection
                .map(|pick| format!(" · picked motion {} at {:.1} px", pick.motion, pick.distance_pixels))
                .unwrap_or_default();
            let pages = self
                .scene
                .as_ref()
                .map_or(0, |scene| page_table(scene).page_count());
            // Read the shared counters once: two live guards on one mutex in a
            // single expression would deadlock.
            let (resident, omitted) = {
                let stats = self.render_stats.lock().unwrap();
                (stats.resident_pages, stats.budget_omitted)
            };
            let tiles_copied = self.stock_stats.lock().unwrap().tile_uploads;
            ui.painter().text(
                rect.left_bottom() + egui::vec2(12., -12.),
                egui::Align2::LEFT_BOTTOM,
                format!(
                    "{} motions · {} visible list rows · {:.2}× DPI · {} of {} pages resident ({} omitted by budget, {} tiles copied) · pick index {:.1} ms + fingerprint {:.1} ms{}{}",
                    self.motion_count(),
                    self.laid_out_rows,
                    ctx.pixels_per_point(),
                    resident,
                    pages,
                    omitted,
                    tiles_copied,
                    self.picker_build_ms,
                    self.hash_ms,
                    pick,
                    if self.heap.peak_bytes > 0 {
                        format!(" · heap {:.0} MiB peak", self.heap.peak_bytes as f64 / 1048576.)
                    } else {
                        String::new()
                    }
                ),
                egui::FontId::monospace(11.),
                Color32::GRAY,
            );
        });
        let now = ctx.input(|i| i.time);
        if self.playing {
            if let Some(last) = self.last_frame {
                self.frame_ms.push_back((now - last) * 1000.);
                if self.frame_ms.len() > 7200 {
                    self.frame_ms.pop_front();
                }
            }
            if self.show_stock {
                let step = (self.motion_count() / 90).max(1);
                self.stock_seek(
                    self.stock_prefix
                        .saturating_add(step)
                        .min(self.motion_count()),
                );
                if self.stock_prefix >= self.motion_count() {
                    self.stock_seek(0);
                }
            } else {
                self.playhead = (self.playhead + 200).min(self.motion_count());
                if self.playhead == self.motion_count() {
                    self.playhead = 0;
                }
            }
            ctx.request_repaint();
        }
        self.last_frame = Some(now);
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if !files.is_empty() {
            if files.len() != 1 || self.io_pending.is_some() {
                self.status = "Drop one JSON job after the current file action finishes".into();
            } else {
                let id = self.begin_io(ctx.memory(|m| m.focused()));
                self.port
                    .drop_file(id, files.into_iter().next().unwrap(), ctx.clone());
            }
        }
        if draft_changed {
            self.recovery.changed(now);
        }
        if self.recovery.due(now) {
            let edit = self.recovery.edit;
            self.recovery.pending = Some(edit);
            self.recovery.status = "Saving local recovery…".into();
            self.port.save_recovery(
                edit,
                self.recovery.revision,
                Snapshot::new(self.draft.clone(), self.recovery_job.clone()),
                ctx.clone(),
            );
        }
        if self.recovery.enabled
            && self.recovery.ready
            && self.recovery.offered.is_none()
            && self.recovery.edit != self.recovery.saved_edit
            && !self.recovery.failed
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.io_pending.is_some() {
            egui::Modal::new(egui::Id::new("file-action")).show(ctx, |ui| {
                ui.heading("File action in progress");
                ui.label("Finish or cancel the file picker to return to the workspace.");
                ui.spinner();
            });
        }
        // One-shot probes apply to exactly one painted frame.
        self.drill = render::Drill::None;
        let injected = self.render_stats.lock().unwrap().injected_errors;
        if injected > self.error_probe_baseline {
            let error = self
                .render_stats
                .lock()
                .unwrap()
                .last_error
                .clone()
                .unwrap_or_else(|| "unknown".into());
            self.status = format!(
                "Renderer reported a real wgpu error: {error}. Document and prior result retained."
            );
            self.error_probe_baseline = injected;
        }
        let render_stats = self.render_stats.lock().unwrap().clone();
        let stock_stats = self.stock_stats.lock().unwrap().clone();
        let frames = self.frame_ms.len();
        probe::publish(
            serde_json::json!({
                "status": self.status,
                "motions": self.motion_count(),
                "pages": self
                    .scene
                    .as_ref()
                    .map_or(0, |scene| page_table(scene).page_count()),
                "residentPages": render_stats.resident_pages,
                "omittedPages": render_stats.budget_omitted,
                "uploadBytes": render_stats.upload_bytes,
                "tilesCopied": stock_stats.tile_uploads,
                "tileBytes": stock_stats.tile_bytes,
                "selection": self.selection.map(|pick| pick.motion),
                "probeText": self
                    .draft
                    .raw
                    .get(&self.draft.key(39))
                    .cloned()
                    .unwrap_or_default(),
                "imeActive": self.ime_active,
                "search": self.search,
                "focused": ctx.memory(|memory| memory.focused().is_some()),
                "canvasFocused": ctx.input(|input| input.raw.focused),
                "dpi": ctx.pixels_per_point(),
                "stockPrefix": self
                    .stock
                    .as_ref()
                    .map_or(0, |stock| stock.prefix),
                "seekMs": self.seek_ms,
                "recovery": self.recovery.status,
                "offline": self.offline_status,
                "frames": frames,
                "heapPeakBytes": self.heap.peak_bytes,
            })
            .to_string(),
        );
    }
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    #[test]
    fn cancelled_dialog_restores_field_focus_and_stale_results_are_rejected() {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280., 800.))
            .build_state(|ctx, app: &mut App| app.ui(ctx), App::default());
        h.get_by_label("Maximum depth").click();
        h.run();
        h.get_by_label("Maximum depth").type_text("-");
        h.run();
        let focus = h.ctx.memory(|m| m.focused());
        let id = h.state_mut().begin_io(focus);
        h.step(); // Exercise the actual modal frame before the dialog completes.
        let ctx = h.ctx.clone();
        h.state_mut()
            .finish_io(id, Err("Open cancelled".into()), &ctx);
        h.run();
        assert!(h.get_by_label("Maximum depth").is_focused());
        assert_eq!(
            h.get_by_label("Maximum depth").value().as_deref(),
            Some("-")
        );
        let id = h.state_mut().begin_io(focus);
        h.state_mut().recovery.changed(1.);
        h.state_mut()
            .finish_io(id, Ok(IoValue::Draft(Draft::default())), &ctx);
        h.run();
        assert_eq!(
            h.get_by_label("Maximum depth").value().as_deref(),
            Some("-")
        );
        assert!(h.state().status.contains("newer edits"));
        h.state_mut().retained_save =
            Some(("checked.ngc".into(), b"exact retained output".to_vec()));
        h.state_mut().load_scene(crate::compute::synthetic(20_000));
        assert_eq!(
            h.state().retained_save.as_ref().unwrap().1,
            b"exact retained output"
        );
        h.state_mut().active = Some(99);
        h.state_mut()
            .restore_session(Snapshot::new(Draft::default(), None), 2., &ctx);
        assert!(h.state().active.is_none());
        assert!(h.state().scene.is_none());
        assert!(h.state().stock.is_none());
    }
    #[test]
    fn ime_preedit_does_not_trigger_shortcuts_and_commit_keeps_unicode() {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280., 800.))
            .build_state(|ctx, app: &mut App| app.ui(ctx), App::default());
        h.get_by_label("Maximum depth").click();
        h.run();
        h.event(egui::Event::Ime(egui::ImeEvent::Enabled));
        h.event(egui::Event::Ime(egui::ImeEvent::Preedit("工具".into())));
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
        h.run();
        assert!(h.state().io_pending.is_none());
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("工具".into())));
        h.event(egui::Event::Ime(egui::ImeEvent::Disabled));
        h.run();
        assert_eq!(
            h.get_by_label("Maximum depth").value().as_deref(),
            Some("工具")
        );
        h.key_press(egui::Key::Tab);
        h.run();
        assert!(h.get_by_label("Wall allowance").is_focused());
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        h.run();
        assert!(h.get_by_label("Maximum depth").is_focused());
    }
}
