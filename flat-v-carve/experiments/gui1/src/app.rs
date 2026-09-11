use crate::{
    compute::{Request, Scene, Vertex},
    platform::{Event, IoValue, Port},
    recovery::{Snapshot, Tracker},
    render,
    state::{Draft, FIELDS},
};
use egui::{Color32, RichText};
use std::sync::Arc;

pub struct App {
    pub draft: Draft,
    pub search: String,
    port: Port,
    generation: u64,
    active: Option<u64>,
    pub status: String,
    scene: Option<Scene>,
    vertices: Arc<Vec<Vertex>>,
    stock_cells: Vec<Arc<Vec<u32>>>,
    show_stock: bool,
    stock_step: usize,
    last_stock_tick: f64,
    scene_revision: u64,
    iso: bool,
    zoom: f32,
    yaw: f32,
    playhead: usize,
    playing: bool,
    stage: usize,
    failed_renderer: bool,
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
            vertices: Arc::new(Vec::new()),
            stock_cells: Vec::new(),
            show_stock: false,
            stock_step: 0,
            last_stock_tick: 0.,
            scene_revision: 0,
            iso: false,
            zoom: 1.,
            yaw: 0.,
            playhead: 0,
            playing: false,
            stage: 0,
            failed_renderer: false,
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
        }
    }
}
impl App {
    pub fn set_scene(&mut self, mut scene: Scene) {
        let job = if scene.job.is_empty() {
            None
        } else {
            Some(scene.job.clone())
        };
        if self.recovery_job != job {
            self.recovery_job = job;
            self.recovery.changed(self.last_frame.unwrap_or(0.));
        }
        self.vertices = Arc::new(std::mem::take(&mut scene.vertices));
        self.stock_cells = scene
            .stock_preview
            .as_mut()
            .map(|p| {
                p.frames
                    .iter_mut()
                    .map(|f| Arc::new(std::mem::take(&mut f.cells)))
                    .collect()
            })
            .unwrap_or_default();
        self.stock_step = self.stock_cells.len().saturating_sub(1);
        self.show_stock = !self.stock_cells.is_empty();
        self.playing = false;
        self.playhead = (self.vertices.len() - scene.contour_vertices) / 2;
        self.scene_revision += 1;
        self.scene = Some(scene);
    }
    fn motion_count(&self) -> usize {
        self.vertices
            .len()
            .saturating_sub(self.scene.as_ref().map_or(0, |s| s.contour_vertices))
            / 2
    }
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = Self::default();
        if let Some(state) = &cc.wgpu_render_state {
            state
                .renderer
                .write()
                .callback_resources
                .insert(render::Resources::new(&state.device, state.target_format));
            app.gpu = true;
            state
                .renderer
                .write()
                .callback_resources
                .insert(crate::stock_render::Resources::new(
                    &state.device,
                    state.target_format,
                ));
            app.status = format!("Experimental GUI1 · {:?}", state.adapter.get_info());
        }
        app.recovery.enabled = true;
        app.recovery.status = "Checking local recovery…".into();
        app.port.load_recovery(cc.egui_ctx.clone());
        app
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
        self.vertices = Arc::new(Vec::new());
        self.stock_cells.clear();
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
                        Ok(scene) => {
                            self.status = format!(
                                "Calculation finished in {elapsed_ms:.1} ms · {} vertices · engine status {} · output programs {}",
                                scene.vertices.len(),
                                scene
                                    .report
                                    .pointer("/summary/status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("synthetic"),
                                scene.programs.len()
                            );
                            if let Some(check) = scene
                                .report
                                .pointer("/export/status")
                                .and_then(|v| v.as_str())
                            {
                                self.status = format!(
                                    "Output check: {check}; {} checked programs. Reference settings were not changed.",
                                    scene.programs.len()
                                );
                            }
                            if let Some(error) = scene
                                .report
                                .get("stockPreviewError")
                                .and_then(|v| v.as_str())
                            {
                                self.status
                                    .push_str(&format!(" · Stock preview unavailable: {error}"));
                            }
                            self.set_scene(scene);
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
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
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
                        self.failed_renderer = true;
                        ui.close();
                    }
                    if ui.button("Resume injected renderer").clicked() {
                        self.failed_renderer = false;
                        self.scene_revision += 1;
                        ui.close();
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
        egui::TopBottomPanel::bottom("timeline").resizable(true).default_height(115.).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(if self.playing { "Pause" } else { "Play" }).clicked() { self.playing = !self.playing; }
                ui.selectable_value(&mut self.stage, 0, "All stages");
                ui.selectable_value(&mut self.stage, 1, "Endmill");
                ui.selectable_value(&mut self.stage, 2, "V-bit");
                if self.show_stock {
                    ui.add(egui::Slider::new(&mut self.stock_step,0..=self.stock_cells.len().saturating_sub(1)).text("Stock checkpoint"));
                    if let Some(p)=self.scene.as_ref().and_then(|s|s.stock_preview.as_ref()) {self.playhead=p.frames[self.stock_step].prefix;}
                } else {
                    let motion_count = self.motion_count();
                    ui.add(egui::Slider::new(&mut self.playhead, 0..=motion_count).text("Motion playhead"));
                }
            });
            ui.label(RichText::new(&self.status).color(Color32::from_rgb(210,183,131)));
            if self.show_stock {
                let p=self.scene.as_ref().unwrap().stock_preview.as_ref().unwrap();
                ui.label(format!("Stock preview · {:.4} mm cells (reference {:.4}) · motion {} · {:.2} mm³ removed · {} checkpoints",p.cell_mm,p.reference_cell_mm,self.playhead,p.frames[self.stock_step].stats.removed_volume_mm3,p.frames.len()));
            } else {ui.label("Motion display only. Stock preview uses discrete checkpoints; playback does not establish export eligibility.");}
            ui.horizontal(|ui| {
                if ui.button("Prepare checked small reference").clicked() { self.start(Request::Reference { flower: false, export: true },ctx); }
                if ui.button("Prepare checked flower").clicked() { self.start(Request::Reference { flower: true, export: true },ctx); }
                let checked=ui.add_enabled(self.io_pending.is_none() && self.active.is_none() && self.scene.as_ref().is_some_and(|s| !s.programs.is_empty()), egui::Button::new("Save checked bytes"));
                if checked.clicked() {
                    let program = &self.scene.as_ref().unwrap().programs[0];
                    self.save_bytes(program.filename.clone(), program.gcode.as_bytes().to_vec(),Some(checked.id),ctx);
                }
                let job=ui.add_enabled(self.io_pending.is_none() && self.scene.as_ref().is_some_and(|s| !s.job.is_empty()), egui::Button::new("Save reference job"));
                if job.clicked() {
                    self.save_bytes("gui1-reference.job.json".into(), self.scene.as_ref().unwrap().job.as_bytes().to_vec(),Some(job.id),ctx);
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
                ui.selectable_value(&mut self.iso, false, "Top"); ui.selectable_value(&mut self.iso, true, "Isometric");
                if ui.button("Fit").clicked() { self.zoom = 1.; self.yaw = 0.; }
                ui.add(egui::Slider::new(&mut self.zoom, 0.5..=3.).text("Zoom"));
                if !self.stock_cells.is_empty() {ui.checkbox(&mut self.show_stock,"Stock preview");}
            });
            ui.label(self.scene.as_ref().map(|s| s.name.as_str()).unwrap_or("Load a combined carving reference"));
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0., Color32::from_rgb(21,29,38));
            if response.dragged() { self.yaw += response.drag_delta().x*0.005; }
            if self.failed_renderer {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Injected renderer failure\nDraft and prior result retained", egui::FontId::proportional(18.), Color32::LIGHT_RED);
            } else if self.gpu && !self.vertices.is_empty() {
                let scene = self.scene.as_ref().unwrap();
                let contour = scene.contour_vertices as u32;
                let split = contour + scene.rough_vertices as u32;
                let end = (contour + (self.playhead*2) as u32).min(self.vertices.len() as u32);
                let mut ranges = Vec::with_capacity(3);
                ranges.push(0..contour);
                if self.stage != 2 { ranges.push(contour..end.min(split).max(contour)); }
                if self.stage != 1 { ranges.push(split..end.max(split)); }
                if self.show_stock {
                    let p=scene.stock_preview.as_ref().unwrap();
                    let b=scene.bounds;let size=(b[2]-b[0]).max(b[3]-b[1]).max(0.001);let scale=1.6/size;
                    ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(rect,crate::stock_render::Callback {
                        cells:self.stock_cells[self.stock_step].clone(),revision:(self.scene_revision,self.stock_step),
                        camera:[if self.iso {1.} else {0.},rect.width()/rect.height().max(1.),self.zoom,self.yaw],
                        grid:[((p.stock.x0-(b[0]+b[2])/2.)*scale) as f32,((p.stock.y0-(b[1]+b[3])/2.)*scale) as f32,
                            (p.cell_mm*scale) as f32,(p.stock.thickness_mm*scale) as f32,p.cols as f32,p.rows as f32,
                            ((p.stock.x1-p.stock.x0)*scale) as f32,((p.stock.y1-p.stock.y0)*scale) as f32],
                    }));
                }
                ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(rect, render::Callback {
                    vertices: self.vertices.clone(), revision: self.scene_revision, ranges,
                    camera: [if self.iso {1.} else {0.}, rect.width()/rect.height().max(1.), self.zoom, self.yaw],
                }));
            }
            ui.painter().text(rect.left_bottom()+egui::vec2(12.,-12.), egui::Align2::LEFT_BOTTOM,
                format!("{} vertices · {} visible list rows · {:.2}× DPI\nDrag to rotate · cyan endmill / amber V-bit", self.vertices.len(), self.laid_out_rows,ctx.pixels_per_point()),
                egui::FontId::monospace(11.), Color32::GRAY);
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
                if now - self.last_stock_tick >= 0.25 {
                    self.stock_step = (self.stock_step + 1) % self.stock_cells.len();
                    self.last_stock_tick = now;
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
    }
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
        h.state_mut()
            .set_scene(crate::compute::synthetic(20_000).unwrap());
        assert_eq!(
            h.state().retained_save.as_ref().unwrap().1,
            b"exact retained output"
        );
        h.state_mut().active = Some(99);
        h.state_mut()
            .restore_session(Snapshot::new(Draft::default(), None), 2., &ctx);
        assert!(h.state().active.is_none());
        assert!(h.state().scene.is_none());
        assert!(h.state().vertices.is_empty());
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
