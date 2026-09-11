use crate::{
    compute::{Request, Scene, Vertex},
    platform::{Event, Port},
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
        }
    }
}
impl App {
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
            app.status = format!("Experimental GUI1 · {:?}", state.adapter.get_info());
        }
        app
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
                Event::Computed {
                    id,
                    elapsed_ms,
                    result,
                } if self.active == Some(id) => {
                    self.active = None;
                    match result {
                        Ok(mut scene) => {
                            self.vertices = Arc::new(std::mem::take(&mut scene.vertices));
                            self.scene_revision += 1;
                            self.playhead = (self.vertices.len() - scene.contour_vertices) / 2;
                            self.status = format!(
                                "Calculation finished in {elapsed_ms:.1} ms · {} vertices · engine status {} · output programs {}",
                                self.vertices.len(),
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
                            self.scene = Some(scene);
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
                Event::Opened(Ok(json)) => self.start(Request::Open { json }, ctx),
                Event::Recovered(Ok(draft)) => {
                    self.draft = draft;
                    self.status =
                        "Recovered raw inspector draft; no machining settings changed".into();
                }
                Event::Opened(Err(e)) | Event::Recovered(Err(e)) | Event::Saved(Err(e)) => {
                    self.status = e
                }
                Event::Saved(Ok(message)) => self.status = message,
                _ => {}
            }
        }
    }
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
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
                if ui.button("Open job").clicked() {
                    self.port.open(false, ctx.clone());
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
                if ui.button("Save raw draft").clicked() {
                    let bytes = serde_json::to_vec_pretty(&self.draft).expect("draft serializes");
                    self.port.save(
                        "gui1-raw-draft.json".into(),
                        bytes,
                        self.deny_save,
                        ctx.clone(),
                    );
                }
                if ui.button("Recover raw draft").clicked() {
                    self.port.open(true, ctx.clone());
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
        });
        egui::TopBottomPanel::bottom("timeline").resizable(true).default_height(115.).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(if self.playing { "Pause" } else { "Play" }).clicked() { self.playing = !self.playing; }
                ui.selectable_value(&mut self.stage, 0, "All stages");
                ui.selectable_value(&mut self.stage, 1, "Endmill");
                ui.selectable_value(&mut self.stage, 2, "V-bit");
                let motion_count = self.motion_count();
                ui.add(egui::Slider::new(&mut self.playhead, 0..=motion_count).text("Motion playhead"));
            });
            ui.label(RichText::new(&self.status).color(Color32::from_rgb(210,183,131)));
            ui.label("Motion display only. Stock removal, export eligibility and physical machining are not established by playback.");
            ui.horizontal(|ui| {
                if ui.button("Prepare checked small reference").clicked() { self.start(Request::Reference { flower: false, export: true },ctx); }
                if ui.button("Prepare checked flower").clicked() { self.start(Request::Reference { flower: true, export: true },ctx); }
                if ui.add_enabled(self.active.is_none() && self.scene.as_ref().is_some_and(|s| !s.programs.is_empty()), egui::Button::new("Save checked bytes")).clicked() {
                    let program = &self.scene.as_ref().unwrap().programs[0];
                    self.port.save(program.filename.clone(), program.gcode.as_bytes().to_vec(), self.deny_save, ctx.clone());
                }
                if ui.add_enabled(self.scene.as_ref().is_some_and(|s| !s.job.is_empty()), egui::Button::new("Save reference job")).clicked() {
                    self.port.save("gui1-reference.job.json".into(), self.scene.as_ref().unwrap().job.as_bytes().to_vec(), self.deny_save,ctx.clone());
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
                        .id_salt("setting-search"),
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
                                .open(if self.search.is_empty() {
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
                                                ui.add(
                                                    egui::TextEdit::singleline(text)
                                                        .id_salt("raw")
                                                        .desired_width(150.)
                                                        .char_limit(4096)
                                                        .hint_text("Unset"),
                                                )
                                                .labelled_by(caption.id);
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
            self.playhead = (self.playhead + 200).min(self.motion_count());
            if self.playhead == self.motion_count() {
                self.playhead = 0;
            }
            ctx.request_repaint();
        }
        self.last_frame = Some(now);
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            // Native drop bytes are read on a separate thread in a future port probe.
            self.status =
                "File drop observed; use Open job. Drop admission is not yet qualified.".into();
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.ui(ctx);
    }
}
