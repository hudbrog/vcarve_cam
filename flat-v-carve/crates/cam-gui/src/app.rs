//! First usable Flat V-carve workspace. Document edits and raw text are
//! independent of derived executions; every asynchronous completion is bound
//! to the exact request and edit revision that submitted it.
use crate::{
    compute::{Request, SceneMeta},
    platform::{Event, IoValue, Port},
    recovery::{Snapshot, Tracker},
    session::{self as engine, Command},
    state::{Draft, FIELDS},
    viewport::Viewport as View,
};
use cam_core::project::v5::{CamJobV5, OperationSettingsV5};
use egui::{Color32, RichText};
use serde_json::{Value, json};

// Read-only widget geometry for real-browser pointer tests of the canvas.
// No command or document mutation is exposed by this probe.
thread_local! {static CONTROLS: std::cell::RefCell<std::collections::BTreeMap<String,[f32;4]>> = const {std::cell::RefCell::new(std::collections::BTreeMap::new())};}
pub fn observe_control(label: &str, rect: egui::Rect) {
    CONTROLS.with(|c| {
        c.borrow_mut().insert(
            label.into(),
            [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
        );
    });
}
fn button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    observe_control(label, response.rect);
    response
}

/// Stable field IDs use the same recovery keys as the qualified input binder.
pub const LIVE_FIELDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 8, 9, 10];
pub fn value(job: &CamJobV5, field: usize) -> Option<f64> {
    let s = engine::settings(job);
    match field {
        0 => s.max_depth_mm,
        1 => s.wall_allowance_mm,
        2 => s.endmill.cutting_feed_mm_min,
        3 => s.vbit.cutting_feed_mm_min,
        4 => s.max_floor_ridge_mm,
        5 => s.max_detail_residual_mm,
        6 => job.setup.stock.thickness_mm,
        8 => s.endmill.max_stepdown_mm,
        9 => s.endmill.stepover_mm,
        10 => s.endmill.plunge_feed_mm_min,
        _ => None,
    }
}
pub fn set_value(job: &CamJobV5, field: usize, value: Option<f64>) -> Result<CamJobV5, String> {
    let mut job = job.clone();
    let OperationSettingsV5::FlatVcarve(s) = &mut job.operations[0].settings else {
        return Err("Unsupported operation".into());
    };
    match field {
        0 => s.max_depth_mm = value,
        1 => s.wall_allowance_mm = value,
        2 => s.endmill.cutting_feed_mm_min = value,
        3 => s.vbit.cutting_feed_mm_min = value,
        4 => s.max_floor_ridge_mm = value,
        5 => s.max_detail_residual_mm = value,
        6 => job.setup.stock.thickness_mm = value,
        8 => s.endmill.max_stepdown_mm = value,
        9 => s.endmill.stepover_mm = value,
        10 => s.endmill.plunge_feed_mm_min = value,
        _ => return Err("Unknown field".into()),
    }
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(job)
}

#[derive(Clone, Debug)]
pub struct Document {
    pub job: CamJobV5,
    pub raw: Draft,
}
impl Document {
    pub fn new(job: CamJobV5) -> Self {
        Self {
            job,
            raw: Draft::default(),
        }
    }
    pub fn text(&self, field: usize) -> String {
        self.raw
            .raw
            .get(&self.raw.key(field))
            .cloned()
            .unwrap_or_else(|| {
                value(&self.job, field)
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            })
    }
    pub fn edit(&mut self, field: usize, text: String) -> Result<(), String> {
        self.raw.raw.insert(self.raw.key(field), text.clone());
        let number = Draft::parse(&text).map_err(str::to_string)?;
        self.job = set_value(&self.job, field, number)?;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        LIVE_FIELDS.iter().any(|&field| {
            let text = self.text(field);
            Draft::parse(&text)
                .ok()
                .filter(|v| set_value(&self.job, field, *v).is_ok())
                != Some(value(&self.job, field))
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            schema: 2,
            draft: self.raw.clone(),
            job: Some(self.job.to_json().expect("validated document")),
        }
    }
}

pub struct App {
    pub document: Option<Document>,
    pub revision: u64,
    saved_revision: Option<u64>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    edit_group: Option<usize>,
    view: View,
    port: Port,
    next: u64,
    active: Option<(u64, u64)>,
    plan: Option<(String, u64)>,
    prepared: Option<(Value, u64)>,
    pub status: String,
    io: Option<(u64, IoKind)>,
    retained_save: Option<(String, Vec<u8>, Option<u64>)>,
    retained_save_revision: u64,
    retry: bool,
    pub recovery: Tracker,
    search: String,
    ime: bool,
    focus: Option<egui::Id>,
}
#[derive(Clone, Copy)]
enum IoKind {
    Open,
    Profile,
    Save(Option<u64>),
}
impl Default for App {
    fn default() -> Self {
        Self {
            document: None,
            revision: 0,
            saved_revision: None,
            undo: vec![],
            redo: vec![],
            edit_group: None,
            view: View::default(),
            port: Port::default(),
            next: 0,
            active: None,
            plan: None,
            prepared: None,
            status: "Open an existing single-SVG Flat V-carve job, or choose the flower fixture."
                .into(),
            io: None,
            retained_save: None,
            retained_save_revision: 0,
            retry: false,
            recovery: Tracker::default(),
            search: String::new(),
            ime: false,
            focus: None,
        }
    }
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            view: View::new_viewer(cc),
            ..Default::default()
        };
        app.recovery.enabled = true;
        app.recovery.status = "Checking local recovery…".into();
        app.port.load_recovery(cc.egui_ctx.clone());
        app
    }
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
    fn changed(&mut self, ctx: &egui::Context) {
        self.revision += 1;
        self.prepared = None;
        self.recovery.changed(ctx.input(|i| i.time));
    }
    fn remember(&mut self) {
        if let Some(doc) = &self.document {
            self.undo.push(doc.clone());
            if self.undo.len() > 32 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }
    fn submit(&mut self, command: Command, ctx: &egui::Context) {
        if self.active.is_some() {
            return;
        }
        let id = self.id();
        self.active = Some((id, self.revision));
        self.status = match command {
            Command::Seek { .. } => "Loading stock position…",
            Command::Open { .. } => "Opening document…",
            Command::ApplyProfile { .. } => "Applying machine configuration…",
            Command::Generate { .. } => {
                "Generating ordered endmill / V-bit execution and stock checkpoints…"
            }
            Command::Prepare { .. } => {
                "Checking retained execution and reading back emitted output…"
            }
        }
        .into();
        self.port.start(id, Request::Gui2(command), ctx.clone());
    }
    fn open(&mut self, kind: IoKind, ctx: &egui::Context) {
        let id = self.id();
        self.io = Some((id, kind));
        self.focus = ctx.memory(|m| m.focused());
        self.port.open(id, false, ctx.clone());
    }
    fn save(&mut self, name: String, bytes: Vec<u8>, revision: Option<u64>, ctx: &egui::Context) {
        self.retained_save = Some((name.clone(), bytes.clone(), revision));
        self.retained_save_revision = self.revision;
        self.retry = false;
        let id = self.id();
        self.io = Some((id, IoKind::Save(revision)));
        self.focus = ctx.memory(|m| m.focused());
        self.port.save(id, name, bytes, false, ctx.clone());
    }
    pub fn accept(
        &mut self,
        id: u64,
        result: Result<(SceneMeta, Vec<u8>), String>,
        ctx: &egui::Context,
    ) {
        let Some((expected, revision)) = self.active else {
            return;
        };
        if id != expected {
            return;
        }
        self.active = None;
        if revision != self.revision {
            self.status =
                "Discarded a result for an older edit; generate the current draft.".into();
            return;
        }
        let (meta, payload) = match result {
            Ok(v) => v,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let reply = &meta.report["gui2"];
        if meta.report["protocol"] != engine::PROTOCOL {
            self.status = "GUI2 UI/worker version mismatch; reload matching assets.".into();
            self.plan = None;
            return;
        }
        match reply["kind"].as_str() {
            Some("seek") => match self.view.accept_stock(meta, payload) {
                Ok(()) => self.status = "Stock position loaded from retained execution.".into(),
                Err(e) => self.status = e,
            },
            Some("opened" | "profile") => {
                let job = match CamJobV5::from_json(&meta.job) {
                    Ok(job) => job,
                    Err(e) => {
                        self.status = e.to_string();
                        return;
                    }
                };
                self.remember();
                let raw = if reply["kind"] == "profile" {
                    self.document
                        .as_ref()
                        .map(|d| d.raw.clone())
                        .unwrap_or_default()
                } else {
                    Draft::default()
                };
                self.document = Some(Document { job, raw });
                self.changed(ctx);
                self.plan = None;
                self.status=if reply["kind"]=="profile" {"Machine snapshot applied. Save job includes its datum, mappings and process settings."}else{"Opened as schema 5. Save writes a new portable job; original input file is unchanged."}.into();
                self.view.load_scene(Ok((meta, payload)));
            }
            Some("generated") => {
                if let Some(handle) = reply["handle"].as_str() {
                    self.plan = Some((handle.into(), self.revision));
                    self.prepared = None;
                    self.status = format!(
                        "Generated {} motions · basic checks complete · ready to simulate or prepare output",
                        meta.motions
                    );
                    self.view.load_scene(Ok((meta, payload)));
                }
            }
            Some("prepared") => {
                self.prepared = Some((reply.clone(), self.revision));
                self.status =
                    "Checked output ready. Exact bytes are retained for save and retry.".into();
            }
            _ => self.status = "Unknown GUI2 worker response".into(),
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.port.poll() {
            match event {
                Event::Computed { id, result, .. } => self.accept(id, result, ctx),
                Event::Cancelled { stop_ms, .. } => {
                    if self.active.is_none() {
                        self.status = format!(
                            "Cancelled; retained execution expired. Worker stop reported in {stop_ms:.2} ms."
                        );
                    }
                }
                Event::Io { id, result } => {
                    let Some((expected, kind)) = self.io else {
                        continue;
                    };
                    if id != expected {
                        continue;
                    }
                    self.io = None;
                    if let Some(focus) = self.focus.take() {
                        ctx.memory_mut(|m| m.request_focus(focus));
                    }
                    match result {
                        Ok(IoValue::Job(json)) => match kind {
                            IoKind::Open => self.submit(Command::Open { json }, ctx),
                            IoKind::Profile => {
                                if let Some(doc) = &self.document {
                                    self.submit(
                                        Command::ApplyProfile {
                                            job: doc.job.to_json().unwrap(),
                                            json,
                                        },
                                        ctx,
                                    )
                                }
                            }
                            _ => {}
                        },
                        Ok(IoValue::Saved(message)) => {
                            if let IoKind::Save(Some(revision)) = kind
                                && revision == self.revision
                                && message.starts_with("Saved")
                            {
                                self.saved_revision = Some(revision);
                            }
                            self.status = message;
                            self.retry = false;
                        }
                        Err(error) => {
                            self.status = error;
                            if matches!(kind, IoKind::Save(_)) {
                                self.retry = true;
                            }
                        }
                        _ => {}
                    }
                }
                Event::RecoveryLoaded(result) => match result {
                    Ok(stored) => {
                        self.recovery.revision = stored.as_ref().map(|s| s.revision);
                        self.recovery.offered = stored.filter(|s| s.snapshot.schema == 2);
                        self.recovery.ready = true;
                        self.recovery.status = "Local recovery ready".into();
                    }
                    Err(e) => {
                        self.recovery.failed = true;
                        self.recovery.status = e;
                    }
                },
                Event::RecoverySaved { edit, result } => self.recovery.written(edit, result),
                Event::Notice(text) => self.status = text,
                Event::OfflineStatus(_) => {}
            }
        }
    }
    fn undo(&mut self, ctx: &egui::Context) {
        if let Some(previous) = self.undo.pop() {
            if let Some(current) = self.document.replace(previous) {
                self.redo.push(current);
            }
            self.edit_group = None;
            self.changed(ctx);
        }
    }
    fn redo(&mut self, ctx: &egui::Context) {
        if let Some(next) = self.redo.pop() {
            if let Some(current) = self.document.replace(next) {
                self.undo.push(current);
            }
            self.edit_group = None;
            self.changed(ctx);
        }
    }
    fn current(&self) -> bool {
        self.plan.as_ref().is_some_and(|(_, r)| *r == self.revision)
            && !self.document.as_ref().is_some_and(Document::pending)
    }
    pub fn ui(&mut self, ctx: &egui::Context) {
        CONTROLS.with(|c| c.borrow_mut().clear());
        self.poll(ctx);
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Ime(event) = event {
                    match event {
                        egui::ImeEvent::Preedit(s) => self.ime = !s.is_empty(),
                        egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => self.ime = false,
                        _ => {}
                    }
                }
            }
        });
        if self.io.is_none() && !self.ime {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.undo(ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)) {
                self.redo(ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("gui2-search")));
            }
            if self.active.is_none()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O))
            {
                self.open(IoKind::Open, ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.save_job(ctx);
            }
        }
        egui::TopBottomPanel::top("gui2-commands").show(ctx,|ui|{
            ui.horizontal_wrapped(|ui|{
                ui.label(RichText::new("FLAT / V").strong().size(20.));ui.label("GUI2 · Flat V-carve");
                let idle=self.active.is_none()&&self.io.is_none();
                if button(ui,"Open job",idle).clicked(){self.open(IoKind::Open,ctx);}
                if button(ui,"Flower fixture",idle).clicked(){self.submit(Command::Open{json:engine::FLOWER.into()},ctx);}
                if button(ui,"Save job",self.document.is_some()&&self.io.is_none()).clicked(){self.save_job(ctx);}
                if button(ui,"Undo",!self.undo.is_empty()).clicked(){self.undo(ctx);}
                if button(ui,"Redo",!self.redo.is_empty()).clicked(){self.redo(ctx);}
                let ready=self.document.as_ref().is_some_and(|d|!d.pending());
                if button(ui,"Generate",idle&&ready).clicked(){self.submit(Command::Generate{job:self.document.as_ref().unwrap().job.to_json().unwrap()},ctx);}
                if button(ui,"Prepare checked output",idle&&self.current()).clicked(){self.submit(Command::Prepare{job:self.document.as_ref().unwrap().job.to_json().unwrap(),handle:self.plan.as_ref().unwrap().0.clone()},ctx);}
                if button(ui,"Save checked bytes",idle&&self.prepared.is_some()).clicked(){self.save_output(ctx);}
                if button(ui,"Cancel",self.active.is_some()).clicked(){self.port.cancel();self.active=None;self.plan=None;self.prepared=None;self.status="Stopping compute; document and previous display retained.".into();}
                if self.active.is_some(){ui.spinner();}
                ui.label(if self.document.is_some()&&self.saved_revision!=Some(self.revision){"Unsaved job / draft"}else{""});
            });
            ui.label(RichText::new(&self.status).color(Color32::from_rgb(221,192,136)));
            if self.view.motion_count()>0&&!self.current(){ui.colored_label(Color32::YELLOW,"Previous simulation is stale. Generate the current draft before export.");}
            ui.horizontal_wrapped(|ui|{
                ui.small(&self.recovery.status);
                if self.recovery.offered.is_some(){
                    if button(ui,"Restore draft",true).clicked(){let snapshot=self.recovery.offered.take().unwrap().snapshot;
                        if let Some(job)=snapshot.job.and_then(|j|CamJobV5::from_json(&j).ok()) {self.remember();self.document=Some(Document{job,raw:snapshot.draft});self.changed(ctx);self.plan=None;self.status="Draft restored. Generate again; saved artifact handles are never recovered.".into();}}
                    if button(ui,"Keep current",true).clicked(){self.recovery.offered=None;self.recovery.changed(ctx.input(|i|i.time));}
                }
                if self.recovery.failed&&ui.button("Reload recovery").clicked(){self.recovery.failed=false;self.port.load_recovery(ctx.clone());}
                if self.retry&&self.io.is_none()&&button(ui,"Retry previous save", self.retained_save_revision==self.revision || self.retained_save.as_ref().is_some_and(|(_,_,r)|r.is_some())).clicked() && let Some((name,bytes,revision))=self.retained_save.clone(){self.save(name,bytes,revision,ctx);}
            });
        });
        egui::SidePanel::left("gui2-inspector").default_width(300.).width_range(260.0..=440.0).resizable(true).show(ctx,|ui|{
            egui::ScrollArea::vertical().show(ui,|ui|{
                ui.heading("Setup & cutting");
                if let Some(doc)=&self.document {
                    ui.label(RichText::new(&doc.job.name).strong());
                    ui.label(format!("{} · {} selected filled components",doc.job.artwork[0].name,engine::settings(&doc.job).components.len()));
                    ui.small("Job includes artwork and applied machine settings.");
                    ui.separator();
                    for (label,assignment) in [("Endmill",&engine::settings(&doc.job).endmill),("V-bit",&engine::settings(&doc.job).vbit)]{
                        let direction=match assignment.spindle_direction {Some(cam_core::project::SpindleDirection::Clockwise)=>"CW",Some(cam_core::project::SpindleDirection::Counterclockwise)=>"CCW",None=>"direction unset"};
                        ui.label(format!("{label} · {} RPM · {direction}",assignment.spindle_rpm.map_or("unset".into(),|v|v.to_string())));
                        if let Some(tool)=doc.job.tools.iter().find(|t|t.id==assignment.tool_id){let description=match &tool.geometry {
                            Some(cam_core::project::ToolGeometry::Endmill(t))=>format!("Ø {} mm · cutting length {} mm",t.diameter_mm,t.cutting_length_mm),
                            Some(cam_core::project::ToolGeometry::Vbit(t))=>format!("{}° · tip Ø {} mm · cutting Ø {} mm",t.included_angle_deg,t.tip_diameter_mm,t.max_cutting_diameter_mm),
                            _=>"Tool geometry unset".into(),};ui.small(description);}
                    }
                    ui.separator();
                    ui.label("Machine");
                    if let Some(machine)=&doc.job.machine_configuration {ui.label(&machine.origin.name);for row in &machine.tools{ui.small(format!("{} → T{}{}",row.job_tool_id,row.tool_number.map_or("unset".into(),|n|n.to_string()),row.length_offset_number.map_or(String::new(),|n|format!(" · H{n}"))));}}
                    else {ui.colored_label(Color32::YELLOW,"No machine configuration applied");}
                    ui.small(format!("Z zero: {}",match doc.job.setup.work_zero.z {cam_core::project::WorkZeroZ::StockBottom=>"stock bottom",cam_core::project::WorkZeroZ::StockTop=>"stock top"}));
                    if doc.job.setup.stock.xy.is_none(){ui.small("Stock XY is unset; display uses inferred artwork + cutter bounds.");}
                }
                let idle=self.active.is_none()&&self.io.is_none()&&self.document.is_some();
                if button(ui,"Load machine profile",idle).clicked(){self.open(IoKind::Profile,ctx);}
                if button(ui,"Apply flower machine profile",idle).clicked(){self.submit(Command::ApplyProfile{job:self.document.as_ref().unwrap().job.to_json().unwrap(),json:engine::PROFILE.into()},ctx);}
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.search).id(egui::Id::new("gui2-search")).hint_text("Search cutting settings"));
                for field in LIVE_FIELDS {
                    if !FIELDS[field].to_lowercase().contains(&self.search.to_lowercase()){continue;}
                    let Some(doc)=&self.document else{break;};let mut text=doc.text(field);
                    let response=ui.horizontal(|ui| {
                        let label=ui.add_sized([112.,20.],egui::Label::new(FIELDS[field]));
                        let response=ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new(("gui2-field",&doc.job.operations[0].id,field))).desired_width(72.).char_limit(128).hint_text("Unset")).labelled_by(label.id);
                        ui.small(if matches!(field,2|3|10){"mm/min"}else{"mm"});response
                    }).inner;
                    observe_control(FIELDS[field],response.rect);
                    if response.changed(){if self.edit_group!=Some(field){self.remember();self.edit_group=Some(field);}
                        let result=self.document.as_mut().unwrap().edit(field,text.clone());self.changed(ctx);
                        self.status=result.err().unwrap_or_else(||"Cutting setting changed; generate to update simulation.".into());
                    }
                    if response.lost_focus(){self.edit_group=None;}
                    if let Err(error)=Draft::parse(&text){ui.colored_label(Color32::LIGHT_RED,error);}
                }
                if self.document.as_ref().is_some_and(Document::pending){ui.colored_label(Color32::YELLOW,"Partial/invalid text is kept in recovery. Complete it before generation or job save.");}
                ui.separator();ui.heading("Export");

                let prepared=self.prepared.as_ref().filter(|(_,r)|*r==self.revision).map(|(v,_)|v);
                if let Some(prepared)=prepared {
                    ui.label(format!("{} · {} bytes",prepared["file"]["filename"].as_str().unwrap_or(""),prepared["file"]["byteLength"]));
                    ui.small(format!("SHA256 {}",prepared["file"]["sha256"].as_str().unwrap_or("")));
                    ui.collapsing("Process and check report",|ui|{ui.monospace(serde_json::to_string_pretty(&prepared["bundle"]["report"]).unwrap());});

                }
            });
        });
        self.view.stock_loading = self.active.is_some();
        self.view.show(ctx);
        if self.active.is_none()
            && let Some(prefix) = self.view.take_stock_request()
            && let Some((handle, _)) = &self.plan
        {
            self.submit(
                Command::Seek {
                    handle: handle.clone(),
                    prefix,
                },
                ctx,
            );
        }
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if !files.is_empty() {
            if files.len() != 1 || self.io.is_some() || self.active.is_some() {
                self.status = "Drop one JSON job when the current action finishes.".into();
            } else {
                let id = self.id();
                self.io = Some((id, IoKind::Open));
                self.port.drop_file(id, files[0].clone(), ctx.clone());
            }
        }
        let now = ctx.input(|i| i.time);
        if self.recovery.due(now)
            && let Some(doc) = &self.document
        {
            self.recovery.pending = Some(self.recovery.edit);
            self.port.save_recovery(
                self.recovery.edit,
                self.recovery.revision,
                doc.snapshot(),
                ctx.clone(),
            );
        }
        if self.recovery.edit != self.recovery.saved_edit && !self.recovery.failed {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.io.is_some() {
            egui::Modal::new(egui::Id::new("gui2-file")).show(ctx, |ui| {
                ui.heading("File action");
                ui.label("Finish or cancel the file picker to return.");
                ui.spinner();
            });
        }
        crate::viewport::probe::publish(json!({"controls":CONTROLS.with(|c|c.borrow().clone()),"gui2":true,"status":self.status,"revision":self.revision,"active":self.active.is_some(),"motions":self.view.motion_count(),"stockPrefix":self.view.stock_prefix(),"current":self.current(),"prepared":self.prepared.is_some(),"preparedSha256":self.prepared.as_ref().map(|(p,_)|p["file"]["sha256"].clone()),"job":self.document.as_ref().map(|d|json!({"name":d.job.name,"depth":engine::settings(&d.job).max_depth_mm,"feed":engine::settings(&d.job).endmill.cutting_feed_mm_min,"machine":d.job.machine_configuration.is_some(),"rawDepth":d.text(0),"rawFeed":d.text(2)})),"pending":self.document.as_ref().is_some_and(Document::pending),"recovery":self.recovery.status}).to_string());
    }
    fn save_output(&mut self, ctx: &egui::Context) {
        if let Some((prepared, revision)) = &self.prepared
            && *revision == self.revision
        {
            self.save(
                prepared["file"]["filename"].as_str().unwrap().into(),
                prepared["file"]["gcode"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .to_vec(),
                None,
                ctx,
            );
        }
    }
    fn save_job(&mut self, ctx: &egui::Context) {
        if self.io.is_some() {
            return;
        }
        if let Some(doc) = &self.document {
            if doc.pending() {
                self.status =
                    "Complete pending text before job save; raw draft remains in local recovery."
                        .into();
                return;
            }
            self.save(
                "carving.gui2.job.json".into(),
                doc.job.to_json().unwrap().into_bytes(),
                Some(self.revision),
                ctx,
            );
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.ui(ctx);
    }
}
