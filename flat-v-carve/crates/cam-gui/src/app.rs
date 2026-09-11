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
#[path = "inspector.rs"]
mod inspector;
#[path = "resume.rs"]
mod resume;
#[path = "workspace_ui.rs"]
mod workspace_ui;

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
        _ => crate::authoring::value(job, field),
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
        _ => crate::authoring::set(&mut job, field, value)?,
    }
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(job)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub job: CamJobV5,
    pub raw: Draft,
    pub finish_draft: Option<cam_core::vcarve::VBitPlanningSettings>,
}
impl Document {
    pub fn new(job: CamJobV5) -> Self {
        Self {
            raw: Draft::for_job(&job),
            finish_draft: engine::settings(&job).finish.clone(),
            job,
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
        let group = crate::authoring::group(field);
        for &member in group {
            let previous = self.text(member);
            self.raw.raw.entry(self.raw.key(member)).or_insert(previous);
        }
        self.raw.raw.insert(self.raw.key(field), text.clone());
        let number = Draft::parse(&text).map_err(str::to_string)?;
        if group.is_empty() {
            self.job = set_value(&self.job, field, number)?;
        } else {
            let values = group
                .iter()
                .map(|&member| {
                    Draft::parse(&self.text(member))
                        .map_err(str::to_string)?
                        .ok_or("Complete all dimensions in this group".into())
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut candidate = self.job.clone();
            crate::authoring::set_group(&mut candidate, field, &values)?;
            candidate.validate_structure().map_err(|e| e.to_string())?;
            self.job = candidate;
        }
        Ok(())
    }
    pub fn pending(&self) -> bool {
        crate::authoring::FIELDS
            .iter()
            .filter(|&&f| crate::authoring::active(&self.job, f))
            .any(|&field| {
                let text = self.text(field);
                Draft::parse(&text).ok() != Some(value(&self.job, field))
            })
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            schema: 3,
            draft: self.raw.clone(),
            job: Some(self.job.to_json().expect("validated document")),
            finish_draft: self.finish_draft.clone(),
            workspace: Default::default(),
            undo: vec![],
            redo: vec![],
        }
    }
}

pub struct App {
    pub document: Option<Document>,
    pub revision: u64,
    saved_revision: Option<u64>,
    saved_job_hash: Option<String>,
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
    components: Vec<crate::authoring::Component>,
    inspector_tab: usize,
    preview_dirty: bool,
    inspector_width: f32,
    scroll: [f32; 6],
    simulate: bool,
    last_workspace: Option<crate::recovery::Workspace>,
    plan_fingerprint: Option<String>,
    resume_view: Option<(String, usize)>,
    cancelled_id: Option<u64>,
}
#[derive(Clone, Copy)]
enum IoKind {
    Svg,
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
            saved_job_hash: None,
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
            components: vec![],
            inspector_tab: 2,
            preview_dirty: false,
            inspector_width: 325.,
            scroll: [0.; 6],
            simulate: false,
            last_workspace: None,
            plan_fingerprint: None,
            resume_view: None,
            cancelled_id: None,
        }
    }
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            view: View::new_viewer(cc),
            ..Default::default()
        };
        Self::theme(&cc.egui_ctx);
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
        self.preview_dirty = true;
        self.recovery.changed(ctx.input(|i| i.time));
    }
    fn remember(&mut self) {
        if let Some(doc) = &self.document {
            self.undo.push(doc.clone());
            Self::trim_history(&mut self.undo);
        }
        self.redo.clear();
    }
    fn submit(&mut self, command: Command, ctx: &egui::Context) {
        if self.active.is_some() {
            return;
        }
        let id = self.id();
        self.active = Some((id, self.revision));
        self.cancelled_id = None;
        self.status = match command {
            Command::ImportSvg { .. } => "Importing SVG…",
            Command::Preview { .. } => "Updating artwork preview…",
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
        self.port.open(id, matches!(kind, IoKind::Svg), ctx.clone());
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
            Some("preview") => {
                self.components =
                    serde_json::from_value(reply["components"].clone()).unwrap_or_default();
                self.preview_dirty = false;
                self.view.load_scene(Ok((meta, payload)));
                self.status =
                    "Artwork preview updated. Generate to calculate cutting motions.".into();
            }
            Some("seek") => match self.view.accept_stock(meta, payload) {
                Ok(()) => self.status = "Stock position loaded from retained execution.".into(),
                Err(e) => self.status = e,
            },
            Some("opened" | "profile" | "imported") => {
                let job = match CamJobV5::from_json(&meta.job) {
                    Ok(job) => job,
                    Err(e) => {
                        self.status = e.to_string();
                        return;
                    }
                };
                self.remember();
                let mut raw = if reply["kind"] == "profile" {
                    self.document
                        .as_ref()
                        .map(|d| d.raw.clone())
                        .unwrap_or_else(|| Draft::for_job(&job))
                } else {
                    Draft::for_job(&job)
                };
                if reply["kind"] == "profile" {
                    for field in [7, 32, 33, 38, 39] {
                        raw.raw.remove(&raw.key(field));
                    }
                }
                self.document = Some(Document {
                    finish_draft: engine::settings(&job).finish.clone().or_else(|| {
                        (reply["kind"] == "profile")
                            .then(|| self.document.as_ref().and_then(|d| d.finish_draft.clone()))
                            .flatten()
                    }),
                    job,
                    raw,
                });
                self.changed(ctx);
                self.plan = None;
                self.preview_dirty = false;
                self.components =
                    serde_json::from_value(reply["components"].clone()).unwrap_or_default();
                if reply["kind"] == "imported" {
                    self.inspector_tab = 0;
                } else if reply["kind"] == "opened" {
                    self.inspector_tab = 2;
                }
                if reply["kind"] != "profile" {
                    self.search.clear();
                    self.scroll = [0.; 6];
                    self.simulate = false;
                    self.plan_fingerprint = None;
                    self.resume_view = None;
                    self.saved_job_hash = None;
                }
                self.status=if reply["kind"]=="profile" {"Machine snapshot applied. Save job includes its datum, mappings and process settings."}else{"Opened as schema 5. Save writes a new portable job; original input file is unchanged."}.into();
                self.view.load_scene(Ok((meta, payload)));
                if self.inspector_tab == 0 {
                    self.status="SVG imported. Select filled components and enter stock, target and cutting settings; no machining defaults were copied.".into();
                }
            }
            Some("generated") => {
                if let Some(handle) = reply["handle"].as_str() {
                    self.plan = Some((handle.into(), self.revision));
                    self.plan_fingerprint =
                        reply["executionFingerprint"].as_str().map(str::to_owned);
                    self.prepared = None;
                    self.status = format!(
                        "Generated {} motions · basic checks complete · ready to simulate or prepare output",
                        meta.motions
                    );
                    self.view.load_scene(Ok((meta, payload)));
                    if let Some((fingerprint, prefix)) = self.resume_view.take()
                        && self.plan_fingerprint.as_ref() == Some(&fingerprint)
                    {
                        self.view.seek(prefix);
                    }
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
            self.event(event, ctx);
        }
    }
    fn event(&mut self, event: Event, ctx: &egui::Context) {
        match event {
            Event::Computed { id, result, .. } => self.accept(id, result, ctx),
            Event::Cancelled { id, stop_ms } => {
                if self.cancelled_id == Some(id) && self.active.is_none() {
                    self.cancelled_id = None;
                    self.status = format!(
                        "Cancelled; retained execution expired. Worker stop reported in {stop_ms:.2} ms."
                    );
                }
            }
            Event::Io { id, result } => {
                let Some((expected, kind)) = self.io else {
                    return;
                };
                if id != expected {
                    return;
                }
                self.io = None;
                if let Some(focus) = self.focus.take() {
                    ctx.memory_mut(|m| m.request_focus(focus));
                }
                match result {
                    Ok(IoValue::Svg { filename, svg }) => {
                        self.submit(Command::ImportSvg { filename, svg }, ctx)
                    }
                    Ok(IoValue::Job(json)) => match kind {
                        IoKind::Svg => self.submit(
                            Command::ImportSvg {
                                filename: "Imported.svg".into(),
                                svg: json,
                            },
                            ctx,
                        ),
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
                            self.saved_job_hash = self
                                .retained_save
                                .as_ref()
                                .map(|(_, bytes, _)| crate::compute::hash(bytes));
                            self.recovery.changed(ctx.input(|i| i.time));
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
                    self.recovery.offered = stored.filter(|s| s.snapshot.schema == 3);
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
    fn undo(&mut self, ctx: &egui::Context) {
        if let Some(previous) = self.undo.pop() {
            if let Some(current) = self.document.replace(previous) {
                self.redo.push(current);
                Self::trim_history(&mut self.redo);
            }
            self.edit_group = None;
            self.plan = None;
            self.changed(ctx);
        }
    }
    fn redo(&mut self, ctx: &egui::Context) {
        if let Some(next) = self.redo.pop() {
            if let Some(current) = self.document.replace(next) {
                self.undo.push(current);
                Self::trim_history(&mut self.undo);
            }
            self.edit_group = None;
            self.plan = None;
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
        self.commands(ctx);
        self.navigator(ctx);
        self.inspector(ctx);
        if self.preview_dirty
            && self.plan.is_none()
            && self.active.is_none()
            && self.io.is_none()
            && ctx.input(|i| i.time) - self.recovery.last_edit > 0.3
            && let Some(doc) = &self.document
        {
            self.submit(
                Command::Preview {
                    job: doc.job.to_json().unwrap(),
                },
                ctx,
            );
        }
        self.view.stock_loading = self.active.is_some();
        self.view.show(ctx, self.simulate);
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
                self.status = "Drop one SVG or JSON job when the current action finishes.".into();
            } else {
                let id = self.id();
                self.io = Some((id, IoKind::Open));
                self.port.drop_file(id, files[0].clone(), ctx.clone());
            }
        }
        let now = ctx.input(|i| i.time);
        let workspace = self.workspace();
        if self.document.is_some() && self.last_workspace.as_ref() != Some(&workspace) {
            self.last_workspace = Some(workspace);
            self.recovery.changed(now);
        }
        if self.recovery.due(now) && self.document.is_some() {
            self.recovery.pending = Some(self.recovery.edit);
            self.port.save_recovery(
                self.recovery.edit,
                self.recovery.revision,
                self.recovery_snapshot().expect("document present"),
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
        crate::viewport::probe::publish(json!({"controls":CONTROLS.with(|c|c.borrow().clone()),"gui2":true,"workspace":self.workspace(),"undo":self.undo.len(),"redo":self.redo.len(),"status":self.status,"revision":self.revision,"active":self.active.is_some(),"motions":self.view.motion_count(),"stockPrefix":self.view.stock_prefix(),"current":self.current(),"prepared":self.prepared.is_some(),"preparedSha256":self.prepared.as_ref().map(|(p,_)|p["file"]["sha256"].clone()),"job":self.document.as_ref().map(|d|json!({"name":d.job.name,"depth":engine::settings(&d.job).max_depth_mm,"feed":engine::settings(&d.job).endmill.cutting_feed_mm_min,"machine":d.job.machine_configuration.is_some(),"rawDepth":d.text(0),"rawFeed":d.text(2),"components":engine::settings(&d.job).components.len(),"mode":engine::settings(&d.job).mode,"stock":d.job.setup.stock,"placement":d.job.artwork[0].placement,"workZero":d.job.setup.work_zero,"endmillGeometry":crate::authoring::tool(&d.job,false).and_then(|t|t.geometry.clone()),"vbitGeometry":crate::authoring::tool(&d.job,true).and_then(|t|t.geometry.clone())})),"pending":self.document.as_ref().is_some_and(Document::pending),"recovery":self.recovery.status}).to_string());
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
