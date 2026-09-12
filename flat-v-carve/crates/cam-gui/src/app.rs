//! First usable Flat V-carve workspace. Document edits and raw text are
//! independent of derived executions; every asynchronous completion is bound
//! to the exact request and edit revision that submitted it.
use crate::session::{GenerateScope, OperationKind};
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
#[path = "export_ui.rs"]
mod export_ui;
#[path = "help.rs"]
mod help;
#[path = "inspector.rs"]
mod inspector;
#[path = "issues.rs"]
mod issues;
#[path = "operation_list.rs"]
mod operation_list;
#[path = "resource_ui.rs"]
mod resource_ui;
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

/// The geometry the current operation owns. Every operation keeps its own
/// explicit selection; artwork never assigns geometry on its own.
pub fn operation_selection(
    job: &CamJobV5,
    operation_id: &str,
) -> Vec<cam_core::project::v5::GeometryRef> {
    match &crate::session::operation(job, operation_id).map(|op| &op.settings) {
        Some(OperationSettingsV5::DragKnife(settings)) => settings.chains.clone(),
        Some(OperationSettingsV5::FlatVcarve(settings)) => settings.components.clone(),
        Some(OperationSettingsV5::Profile(settings)) => settings
            .contours
            .iter()
            .map(|c| c.geometry.clone())
            .collect(),
        _ => vec![],
    }
}

/// Stable field IDs use the same recovery keys as the qualified input binder.
pub const LIVE_FIELDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 8, 9, 10];

fn kind_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::FlatVcarve => "flat_vcarve",
        OperationKind::Face => "face",
        OperationKind::Profile => "profile",
        OperationKind::DragKnife => "drag_knife",
    }
}

/// The document field an anchor row's text belongs to.
fn anchor_field(kind: crate::profile::AnchorKind) -> usize {
    match kind {
        crate::profile::AnchorKind::Tab => 97,
        crate::profile::AnchorKind::Start => 108,
    }
}
/// The value one field shows for one explicit operation. The operation's kind
/// decides which settings own the field; everything shared (setup, machine,
/// placement, tool geometry) resolves through the shared authoring layer.
pub fn value(job: &CamJobV5, operation_id: &str, field: usize) -> Option<f64> {
    match engine::kind(job, operation_id) {
        None => crate::authoring::value_in(job, operation_id, field),
        Some(OperationKind::Face) => crate::face::value(job, operation_id, field),
        Some(OperationKind::Profile) => crate::profile::value(job, operation_id, field),
        Some(OperationKind::DragKnife) => crate::knife::value_in(job, operation_id, field),
        Some(OperationKind::FlatVcarve) => {
            let s = engine::settings_in(job, operation_id)?;
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
                _ => crate::authoring::value_in(job, operation_id, field),
            }
        }
    }
}

pub fn set_value(
    job: &CamJobV5,
    operation_id: &str,
    field: usize,
    value: Option<f64>,
) -> Result<CamJobV5, String> {
    let mut job = job.clone();
    if job.operations.is_empty() {
        if !crate::authoring::active_in(&job, "", field) {
            return Err("Add an operation before editing cutting fields".into());
        }
        if field == 6 {
            job.setup.stock.thickness_mm = value;
        } else {
            crate::authoring::set_in(&mut job, "", field, value)?;
        }
        job.validate_structure().map_err(|e| e.to_string())?;
        return Ok(job);
    }
    match engine::kind(&job, operation_id) {
        Some(OperationKind::Face) => crate::face::set(&mut job, operation_id, field, value)?,
        Some(OperationKind::Profile) => crate::profile::set(&mut job, operation_id, field, value)?,
        Some(OperationKind::DragKnife) => {
            crate::knife::set_in(&mut job, operation_id, field, value)?;
        }
        Some(OperationKind::FlatVcarve) => {
            if field == 6 {
                job.setup.stock.thickness_mm = value;
            } else if matches!(field, 0 | 1 | 2 | 3 | 4 | 5 | 8 | 9 | 10) {
                let OperationSettingsV5::FlatVcarve(s) =
                    &mut engine::operation_mut(&mut job, operation_id)
                        .ok_or("Unsupported operation")?
                        .settings
                else {
                    return Err("Unsupported operation".into());
                };
                match field {
                    0 => s.max_depth_mm = value,
                    1 => s.wall_allowance_mm = value,
                    2 => s.endmill.cutting_feed_mm_min = value,
                    3 => s.vbit.cutting_feed_mm_min = value,
                    4 => s.max_floor_ridge_mm = value,
                    5 => s.max_detail_residual_mm = value,
                    8 => s.endmill.max_stepdown_mm = value,
                    9 => s.endmill.stepover_mm = value,
                    _ => s.endmill.plunge_feed_mm_min = value,
                }
            } else {
                crate::authoring::set_in(&mut job, operation_id, field, value)?;
            }
        }
        _ => crate::authoring::set_in(&mut job, operation_id, field, value)?,
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
    pub fn active_artwork(&self) -> Option<&cam_core::project::v5::ArtworkItem> {
        self.job
            .artwork
            .iter()
            .find(|i| i.id.0 == self.raw.artwork_item)
    }
    pub fn select_artwork(&mut self, id: &str) -> bool {
        if !self.job.artwork.iter().any(|i| i.id.0 == id) {
            return false;
        }
        self.raw.artwork_item = id.into();
        true
    }
    /// The operation this editor session addresses.
    pub fn active_operation(&self) -> Option<&cam_core::project::v5::OperationV5> {
        crate::session::operation(&self.job, &self.raw.operation)
    }
    pub fn select_operation(&mut self, id: &str) -> bool {
        if !self.job.operations.iter().any(|op| op.id == id) {
            return false;
        }
        self.raw.operation = id.into();
        true
    }
    /// Keep the operation selection attached to a live operation after edits
    /// that add, delete or reorder them.
    pub fn sync_operation(&mut self) {
        if self.active_operation().is_some() {
            return;
        }
        self.raw.operation = self
            .job
            .operations
            .first()
            .map(|op| op.id.clone())
            .unwrap_or_default();
    }
    pub fn sync_artwork(&mut self) {
        if self.active_artwork().is_none() {
            self.raw.artwork_item = self
                .job
                .artwork
                .first()
                .map(|i| i.id.0.clone())
                .unwrap_or_default();
        }
        self.prune_scoped_drafts();
    }
    /// Drop drafts whose scoped entity no longer exists: placement text for a
    /// removed artwork item, and anchor text for an anchor the document no
    /// longer carries. Undo restores the whole document, including this text.
    pub fn prune_scoped_drafts(&mut self) {
        let Document { job, raw, .. } = self;
        raw.raw.retain(|key, _| {
            let Some((scope, operation, label)) = crate::state::scope_of(key) else {
                return false;
            };
            let Some(field) = FIELDS.iter().position(|name| *name == label) else {
                return false;
            };
            if crate::state::is_placement(field) {
                return job.artwork.iter().any(|item| item.id.0 == scope);
            }
            if crate::state::is_anchor(field) {
                return crate::profile::anchor_value(job, operation, scope, field).is_some();
            }
            true
        });
    }
    pub fn field_value(&self, field: usize) -> Option<f64> {
        if matches!(field, 26..=29) {
            let p = &self.active_artwork()?.placement;
            Some(match field {
                26 => p.origin_mm.x,
                27 => p.origin_mm.y,
                28 => p.rotation_deg,
                _ => p.scale,
            })
        } else {
            value(&self.job, &self.raw.operation, field)
        }
    }
    pub fn new(job: CamJobV5) -> Self {
        Self {
            raw: Draft::for_job(&job),
            finish_draft: engine::carving(&job).and_then(|s| s.finish.clone()),
            job,
        }
    }
    pub fn text(&self, field: usize) -> String {
        self.raw
            .raw
            .get(&self.raw.key(field))
            .cloned()
            .unwrap_or_else(|| {
                self.field_value(field)
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            })
    }
    pub fn edit(&mut self, field: usize, text: String) -> Result<(), String> {
        let group = crate::authoring::group(field);
        let face_group = crate::face::group(field);
        let group = if group.is_empty() { face_group } else { group };
        for &member in group {
            let previous = self.text(member);
            self.raw.raw.entry(self.raw.key(member)).or_insert(previous);
        }
        self.raw.raw.insert(self.raw.key(field), text.clone());
        let number = Draft::parse(&text).map_err(str::to_string)?;
        // Inactive entry fields are an editor draft until the explicit Ramp action.
        if matches!(field, 14 | 51)
            && !crate::authoring::active_in(&self.job, &self.raw.operation, field)
        {
            return Ok(());
        }
        if matches!(field, 26..=29) {
            let number = number.ok_or("Placement values cannot be unset")?;
            let mut candidate = self.job.clone();
            let item = candidate
                .artwork
                .iter_mut()
                .find(|i| i.id.0 == self.raw.artwork_item)
                .ok_or("Select artwork first")?;
            match field {
                26 => item.placement.origin_mm.x = number,
                27 => item.placement.origin_mm.y = number,
                28 => item.placement.rotation_deg = number,
                _ => item.placement.scale = number,
            };
            candidate.validate_structure().map_err(|e| e.to_string())?;
            self.job = candidate;
        } else if group.is_empty() {
            self.job = set_value(&self.job, &self.raw.operation, field, number)?;
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
            crate::authoring::set_group_in(&mut candidate, &self.raw.operation, field, &values)?;
            candidate.validate_structure().map_err(|e| e.to_string())?;
            self.job = candidate;
        }
        Ok(())
    }
    /// Raw text of one anchor-scoped field (a profile start or tab anchor).
    /// The row supplies the committed value; the text is the editor's own.
    pub fn anchor_text(&self, scope: &str, field: usize, value: Option<f64>) -> String {
        self.raw
            .raw
            .get(&self.raw.key_scoped(scope, field))
            .cloned()
            .unwrap_or_else(|| value.map(|n| n.to_string()).unwrap_or_default())
    }
    /// Commit one anchor position. The anchor is addressed by the contour it
    /// parameterizes, so the text always lands on that anchor.
    pub fn edit_anchor(
        &mut self,
        scope: &str,
        field: usize,
        text: String,
        target: crate::profile::AnchorKind,
    ) -> Result<(), String> {
        self.raw
            .raw
            .insert(self.raw.key_scoped(scope, field), text.clone());
        let number = Draft::parse(&text).map_err(str::to_string)?;
        let fraction = number.ok_or("An anchor position cannot be unset")?;
        let mut candidate = self.job.clone();
        crate::profile::set_anchor_fraction(
            &mut candidate,
            &self.raw.operation,
            target,
            scope,
            fraction,
        )?;
        candidate.validate_structure().map_err(|e| e.to_string())?;
        self.job = candidate;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        let scalar_pending = crate::authoring::FIELDS
            .iter()
            .filter(|&&f| {
                !matches!(f, 26..=29)
                    && crate::authoring::active_in(&self.job, &self.raw.operation, f)
            })
            .any(|&field| {
                let text = self.text(field);
                Draft::parse(&text).ok() != Some(value(&self.job, &self.raw.operation, field))
            });
        scalar_pending
            || self.job.artwork.iter().any(|item| {
                (26..=29).any(|f| {
                    self.raw
                        .raw
                        .get(&self.raw.key_for(&item.id.0, f))
                        .is_some_and(|text| {
                            let p = &item.placement;
                            let n = match f {
                                26 => p.origin_mm.x,
                                27 => p.origin_mm.y,
                                28 => p.rotation_deg,
                                _ => p.scale,
                            };
                            Draft::parse(text).ok() != Some(Some(n))
                        })
                })
            })
            || self.raw.raw.iter().any(|(key, text)| {
                let Some((scope, operation, label)) = crate::state::scope_of(key) else {
                    return false;
                };
                let Some(field) = FIELDS.iter().position(|name| *name == label) else {
                    return false;
                };
                if !crate::state::is_anchor(field) {
                    return false;
                }
                match crate::profile::anchor_value(&self.job, operation, scope, field) {
                    Some(value) => Draft::parse(text).ok() != Some(Some(value)),
                    // A removed anchor keeps its text but cannot block a save.
                    None => false,
                }
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
    plan_scope: Option<GenerateScope>,
    prepared: Option<(Value, u64)>,
    export_dialog: Option<export_ui::ExportDialog>,
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
    operation_tab: usize,
    operation_picker: Option<usize>,
    operation_rename: Option<(String, String)>,
    operation_scroll: [f32; 3],
    operation_ramp_draft: bool,
    preview_dirty: bool,
    inspector_width: f32,
    scroll: [f32; 8],
    simulate: bool,
    last_workspace: Option<crate::recovery::Workspace>,
    plan_fingerprint: Option<String>,
    resume_view: Option<(String, usize)>,
    cancelled_id: Option<u64>,
    issues: Vec<cam_core::operations::LocatedDiagnostic>,
    issue_focus: Option<String>,
    artwork_io: Option<(u64, String)>,
    artwork_rejections: Vec<String>,
    resources: crate::resources::Editor,
    resource_request: Option<(u64, ResourceIntent)>,
    resource_import_stamp: Option<String>,
    profile_io_revision: Option<u64>,
}
#[derive(Clone, Copy)]
enum ResourceIntent {
    Load,
    Compare,
    Save,
}
#[derive(Clone, Copy)]
enum IoKind {
    KnifeSvg,
    ProfileSvg,
    Svg,
    AddSvg,
    ReplaceSvg,
    Open,
    Migrate,
    Profile,
    LibraryImport,
    MachineImport,
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
            plan_scope: None,
            prepared: None,
            export_dialog: None,
            status: "Open a portable Flat V-carve project, or import SVG artwork to start.".into(),
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
            operation_tab: 0,
            operation_picker: None,
            operation_rename: None,
            operation_scroll: [0.; 3],
            operation_ramp_draft: false,
            preview_dirty: false,
            inspector_width: 325.,
            scroll: [0.; 8],
            simulate: false,
            last_workspace: None,
            plan_fingerprint: None,
            resume_view: None,
            cancelled_id: None,
            issues: vec![],
            issue_focus: None,
            artwork_io: None,
            artwork_rejections: vec![],
            resources: Default::default(),
            resource_request: None,
            resource_import_stamp: None,
            profile_io_revision: None,
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
    /// The selected operation ID (empty when the job has no operations).
    pub(super) fn operation_id(&self) -> String {
        self.document
            .as_ref()
            .map(|d| d.raw.operation.clone())
            .unwrap_or_default()
    }
    /// The selected operation's kind.
    pub(super) fn operation_kind(&self) -> Option<OperationKind> {
        let doc = self.document.as_ref()?;
        engine::kind(&doc.job, &doc.raw.operation)
    }
    fn changed(&mut self, ctx: &egui::Context) {
        self.revision += 1;
        self.issues.clear();
        self.prepared = None;
        self.view.stale_knife_evidence();
        if let Some(document) = &mut self.document {
            document.prune_scoped_drafts();
        }
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
        if self.operation_ramp_draft
            && matches!(command, Command::Generate { .. } | Command::Prepare { .. })
        {
            self.status = "Complete ramp angle and feed, or choose Plunge entry.".into();
            return;
        }
        if self.active.is_some() {
            return;
        }
        let id = self.id();
        self.active = Some((id, self.revision));
        self.cancelled_id = None;
        if matches!(command, Command::Prepare { .. }) {
            self.prepared = None;
            self.export_dialog = Some(export_ui::ExportDialog {
                request: Some(id),
                ..Default::default()
            });
        }
        self.status = match command {
            Command::ImportSvg { .. }
            | Command::ImportKnifeSvg { .. }
            | Command::ImportProfileSvg { .. } => "Importing SVG…",
            Command::Artwork { .. } => "Updating artwork collection…",
            Command::Operation { .. } => "Updating operations…",
            Command::Resource { .. } => "Copying reviewed resource values…",
            Command::Preview { .. } => "Updating artwork preview…",
            Command::ValidatePlan { .. } => {
                "Checking whether the retained execution still matches…"
            }
            Command::Seek { .. } => "Loading stock position…",
            Command::Open { .. } => "Opening document…",
            Command::Migrate { .. } => "Importing older job into the portable format…",
            Command::ApplyProfile { .. } => "Applying machine configuration…",
            Command::Generate { .. } => "Generating toolpaths and stock checkpoints…",
            Command::Prepare { .. } => {
                "Checking retained execution and reading back emitted output…"
            }
        }
        .into();
        self.port
            .start(id, Request::Gui2(Box::new(command)), ctx.clone());
    }

    /// Generate the whole enabled list, or the prefix ending at one operation.
    /// A prefix plan is what the retained service later prepares and exports:
    /// the GUI never re-plans a subset at export time.
    pub(super) fn generate(&mut self, scope: GenerateScope, ctx: &egui::Context) {
        let Some(doc) = &self.document else {
            self.status = "Open or create a job before generating.".into();
            return;
        };
        if doc.job.operations.is_empty() {
            self.status = "Add an operation before generating.".into();
            return;
        }
        let job = doc.job.to_json().unwrap();
        self.submit(Command::Generate { job, scope }, ctx);
    }

    /// The scope the retained plan was generated for, if one is current.
    pub(super) fn plan_scope(&self) -> Option<&GenerateScope> {
        self.plan_scope.as_ref()
    }

    /// Read-only browser probe of the current document. It publishes the
    /// ordered operation list plus the selected operation's own settings, and
    /// never touches machining state.
    fn job_probe(&self, d: &Document) -> Value {
        let operations: Vec<Value> = d
            .job
            .operations
            .iter()
            .map(|operation| {
                json!({
                    "id": operation.id,
                    "name": operation.name,
                    "enabled": operation.enabled,
                    "kind": engine::kind(&d.job, &operation.id).map(kind_name),
                })
            })
            .collect();
        let selected = d.raw.operation.clone();
        let base = json!({
            "name": d.job.name,
            "tools": d.job.tools,
            "stock": d.job.setup.stock,
            "machineSnapshot": d.job.machine_configuration,
            "workZero": d.job.setup.work_zero,
            "activeArtwork": d.raw.artwork_item,
            "artworks": d.job.artwork,
            "operations": operations,
            "selectedOperation": selected,
        });
        let mut probe = match engine::kind(&d.job, &selected) {
            None => json!({"kind": "empty"}),
            Some(OperationKind::DragKnife) => json!({
                "kind": "drag_knife",
                "knife": crate::knife::settings_in(&d.job, &selected),
            }),
            Some(OperationKind::Face) => {
                let settings = crate::session::face(&d.job, &selected);
                json!({
                    "kind": "face",
                    "face": settings,
                    "stepdown": settings.and_then(|s| s.stepdown_mm),
                    "stepover": settings.and_then(|s| s.stepover_mm),
                    "passAngle": settings.and_then(|s| s.pass_angle_deg),
                    "topOffset": settings.map(|s| s.top.offset_mm),
                    "bottomOffset": settings.map(|s| s.bottom.offset_mm),
                    "rawStepdown": d.text(8),
                    "rawBottomOffset": d.text(87),
                    "machine": d.job.machine_configuration.is_some(),
                })
            }
            Some(kind) => {
                let carving = engine::settings_in(&d.job, &selected);
                let mut probe = json!({
                    "depth": carving.and_then(|s| s.max_depth_mm),
                    "feed": carving.and_then(|s| s.endmill.cutting_feed_mm_min),
                    "machine": d.job.machine_configuration.is_some(),
                    "rawDepth": d.text(0),
                    "rawFeed": d.text(2),
                    "components": carving.map(|s| s.components.len()).unwrap_or(0),
                    "mode": carving.map(|s| s.mode),
                    "placement": d.active_artwork().map(|i| &i.placement),
                    "assignment": carving.map(|s| &s.components),
                    "endmillGeometry": crate::authoring::tool_in(&d.job, &selected, false)
                        .and_then(|t| t.geometry.clone()),
                    "vbitGeometry": crate::authoring::tool_in(&d.job, &selected, true)
                        .and_then(|t| t.geometry.clone()),
                    "profileStatuses": cam_core::project::v5::resources::assignment_statuses(&d.job),
                });
                probe["kind"] = json!(kind_name(kind));
                probe
            }
        };
        if let (Some(target), Some(source)) = (probe.as_object_mut(), base.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        probe
    }
    fn open(&mut self, kind: IoKind, ctx: &egui::Context) {
        self.resource_import_stamp = matches!(kind, IoKind::LibraryImport | IoKind::MachineImport)
            .then(|| self.resource_stamp());
        self.profile_io_revision = matches!(kind, IoKind::Profile).then_some(self.revision);
        let id = self.id();
        self.io = Some((id, kind));
        self.focus = ctx.memory(|m| m.focused());
        self.artwork_io = if matches!(kind, IoKind::AddSvg | IoKind::ReplaceSvg) {
            self.document
                .as_ref()
                .map(|d| (self.revision, d.raw.artwork_item.clone()))
        } else {
            None
        };
        self.port.open(
            id,
            matches!(
                kind,
                IoKind::Svg
                    | IoKind::KnifeSvg
                    | IoKind::ProfileSvg
                    | IoKind::AddSvg
                    | IoKind::ReplaceSvg
            ),
            matches!(kind, IoKind::AddSvg),
            ctx.clone(),
        );
    }
    fn import_file(&mut self, kind: IoKind, filename: String, svg: String, ctx: &egui::Context) {
        if matches!(kind, IoKind::AddSvg | IoKind::ReplaceSvg) {
            let Some((revision, item)) = self.artwork_io.take() else {
                return;
            };
            if revision != self.revision {
                self.status =
                    "Document changed while choosing artwork; choose the file again.".into();
                return;
            }
            let action = if matches!(kind, IoKind::ReplaceSvg) {
                engine::ArtworkCommand::Replace {
                    item: cam_core::project::v5::ArtworkItemId(item),
                    filename,
                    svg,
                }
            } else {
                engine::ArtworkCommand::Add { filename, svg }
            };
            self.artwork_command(action, ctx);
        } else if matches!(kind, IoKind::KnifeSvg) {
            self.submit(Command::ImportKnifeSvg { filename, svg }, ctx);
        } else if matches!(kind, IoKind::ProfileSvg) {
            self.submit(Command::ImportProfileSvg { filename, svg }, ctx);
        } else {
            self.submit(Command::ImportSvg { filename, svg }, ctx);
        }
    }
    fn artwork_command(&mut self, action: engine::ArtworkCommand, ctx: &egui::Context) {
        self.artwork_rejections.clear();
        if let Some(doc) = &self.document {
            self.submit(
                Command::Artwork {
                    job: doc.job.to_json().unwrap(),
                    operation_id: doc.raw.operation.clone(),
                    action,
                },
                ctx,
            );
        }
    }
    fn select_artwork(&mut self, item: &str, ctx: &egui::Context) {
        if self
            .document
            .as_mut()
            .is_some_and(|d| d.select_artwork(item))
        {
            self.view.select_artwork(item);
            self.edit_group = None;
            self.search.clear();
            self.scroll[0] = 0.;
            self.navigate(0);
            self.recovery.changed(ctx.input(|i| i.time));
        }
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
        let exporting = self
            .export_dialog
            .as_ref()
            .is_some_and(|d| d.request == Some(id));
        if exporting {
            self.export_dialog.as_mut().unwrap().request = None;
        }
        if revision != self.revision {
            self.status =
                "Discarded a result for an older edit; generate the current draft.".into();
            if exporting {
                self.export_dialog.as_mut().unwrap().error = Some(self.status.clone());
            }
            return;
        }
        let (meta, payload) = match result {
            Ok(v) => v,
            Err(error) => {
                if exporting {
                    self.export_dialog.as_mut().unwrap().error = Some(error.clone());
                }
                self.status = error;
                return;
            }
        };
        let reply = &meta.report["gui2"];
        if meta.report["protocol"] != engine::PROTOCOL {
            self.status = "GUI2 UI/worker version mismatch; reload matching assets.".into();
            if exporting {
                self.export_dialog.as_mut().unwrap().error = Some(self.status.clone());
            }
            self.plan = None;
            self.plan_scope = None;
            return;
        }
        match reply["kind"].as_str() {
            Some("resource") => {
                let job = match engine::open(&meta.job) {
                    Ok(job) => job,
                    Err(e) => {
                        self.status = e;
                        return;
                    }
                };
                self.remember();
                if let Some(doc) = &mut self.document {
                    doc.job = job;
                    if let Some(fields) = reply["clearFields"].as_array() {
                        for field in fields.iter().filter_map(Value::as_u64) {
                            doc.raw.raw.remove(&doc.raw.key(field as usize));
                        }
                    }
                }
                self.edit_group = None;
                self.changed(ctx);
                self.status =
                    "Reviewed values copied into the job. Undo restores the previous copy.".into();
            }
            Some("operation") => match CamJobV5::from_json(&meta.job) {
                Ok(job) => {
                    self.adopt_operation(job, reply["activeOperation"].as_str(), ctx);
                    self.view.load_scene(Ok((meta, payload)));
                }
                Err(error) => self.status = error.to_string(),
            },
            Some(
                "artwork" | "carve_selection" | "knife_start" | "knife_selection"
                | "knife_outlines",
            ) => {
                let job = match engine::open(&meta.job) {
                    Ok(job) => job,
                    Err(error) => {
                        self.status = error;
                        return;
                    }
                };
                self.remember();
                if let Some(doc) = &mut self.document {
                    doc.job = job;
                    doc.sync_artwork();
                    if let Some(id) = reply["activeArtwork"].as_str() {
                        doc.select_artwork(id);
                        self.search.clear();
                        self.scroll[0] = 0.;
                    }
                }
                self.edit_group = None;
                self.changed(ctx);
                self.adopt_artwork(reply);
                self.preview_dirty = false;
                self.issues = serde_json::from_value(reply["issues"].clone()).unwrap_or_default();
                self.artwork_rejections =
                    serde_json::from_value(reply["rejectedFiles"].clone()).unwrap_or_default();
                if reply["kind"] == "artwork" {
                    self.navigate(0);
                }
                self.simulate = false;
                self.status = if matches!(
                    reply["kind"].as_str(),
                    Some("carve_selection" | "knife_selection")
                ) {
                    "Operation geometry updated. Undo restores the previous selection.".into()
                } else {
                    "Artwork updated. Assignments retain their exact source revisions; repair unresolved references or Undo.".into()
                };
                if let Some(rejected) = reply["rejectedFiles"].as_array().filter(|r| !r.is_empty())
                {
                    self.status = format!(
                        "Accepted artwork added; rejected files: {}",
                        rejected
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join("; ")
                    );
                }
                if self.plan.is_some() && self.view.motion_count() > 0 {
                    self.resume_view = self
                        .plan_fingerprint
                        .clone()
                        .map(|f| (f, self.view.stock_prefix()));
                }
                self.view.load_scene(Ok((meta, payload)));
                // Collection edits may leave machining unchanged (for example,
                // adding an unassigned source). Only the worker can confirm reuse.
                self.preview_dirty = self.plan.is_some();
            }
            Some("stale") => {
                self.adopt_artwork(reply);
                self.issues = serde_json::from_value(reply["issues"].clone()).unwrap_or_default();
                self.preview_dirty = false;
                if !self.simulate {
                    if self.view.motion_count() > 0 {
                        self.resume_view = self
                            .plan_fingerprint
                            .clone()
                            .map(|f| (f, self.view.stock_prefix()));
                    }
                    self.view.load_scene(Ok((meta, payload)));
                }
                self.status = "Machining inputs changed. Generate to update the carving.".into();
            }
            Some("issues") => {
                self.issues = serde_json::from_value(reply["issues"].clone()).unwrap_or_default();
                self.status = format!(
                    "{} settings need attention. Select an issue to edit its field.",
                    self.issues.len()
                );
            }
            Some("preview") => {
                self.adopt_artwork(reply);
                self.issues = serde_json::from_value(reply["issues"].clone()).unwrap_or_default();
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
                    for field in [7, 32, 33, 34, 35, 36, 38, 39] {
                        raw.raw.remove(&raw.key(field));
                    }
                }
                self.document = Some(Document {
                    finish_draft: engine::carving(&job)
                        .and_then(|s| s.finish.clone())
                        .or_else(|| {
                            (reply["kind"] == "profile")
                                .then(|| {
                                    self.document.as_ref().and_then(|d| d.finish_draft.clone())
                                })
                                .flatten()
                        }),
                    job,
                    raw,
                });
                self.changed(ctx);
                if reply["kind"] != "profile" {
                    self.operation_tab = 0;
                    self.operation_scroll = [0.; 3];
                    self.operation_ramp_draft = false;
                    self.artwork_rejections.clear();
                    self.plan = None;
                    self.plan_scope = None;
                }
                self.preview_dirty = self.plan.is_some();
                self.adopt_artwork(reply);
                if reply["kind"] == "imported" {
                    self.inspector_tab = 0;
                } else if reply["kind"] == "opened" {
                    self.inspector_tab = 2;
                }
                self.issues = serde_json::from_value(reply["issues"].clone()).unwrap_or_default();
                if reply["kind"] != "profile" {
                    self.view.reset_inspection();
                    self.view.artwork.hidden.clear();
                    self.view.artwork.locked.clear();
                    self.search.clear();
                    self.scroll = [0.; 8];
                    self.simulate = false;
                    self.plan_fingerprint = None;
                    self.resume_view = None;
                    self.saved_job_hash = None;
                }
                self.status=if reply["kind"]=="profile" {"Machine snapshot applied. Save job includes its datum, mappings and process settings."}else{"Opened as schema 5. Save writes a new portable job; original input file is unchanged."}.into();
                if self.plan.is_some() && self.view.motion_count() > 0 {
                    self.resume_view = self
                        .plan_fingerprint
                        .clone()
                        .map(|f| (f, self.view.stock_prefix()));
                }
                self.view.load_scene(Ok((meta, payload)));
                if self.inspector_tab == 0 {
                    self.status="SVG imported. In the operation's Geometry to carve, or by clicking filled regions in the viewport, select the components to cut; no machining defaults were copied.".into();
                }
            }
            Some("generated" | "revalidated") => {
                let revalidated = reply["kind"] == "revalidated";
                let previous_prefix = self.view.stock_prefix();
                self.adopt_artwork(reply);
                self.preview_dirty = false;
                if let Some(handle) = reply["handle"].as_str() {
                    self.plan_scope = serde_json::from_value(reply["scope"].clone())
                        .ok()
                        .or_else(|| self.plan_scope.clone());
                    self.plan = Some((handle.into(), self.revision));
                    self.plan_fingerprint =
                        reply["executionFingerprint"].as_str().map(str::to_owned);
                    self.prepared = None;
                    self.issues = reply["generationIssues"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|i| cam_core::operations::LocatedDiagnostic {
                            code: i["code"].as_str().unwrap_or("GENERATION").into(),
                            message: i["message"].as_str().unwrap_or("Generation issue").into(),
                            operation_id: i["operation_id"].as_str().map(str::to_owned),
                            tool_id: None,
                            field_path: None,
                        })
                        .collect();
                    self.status = if reply["checks"]["exportReady"] == true {
                        format!(
                            "Generated {} motions · basic checks passed · ready to simulate or prepare output",
                            meta.motions
                        )
                    } else {
                        format!(
                            "Generated {} motions with unresolved issues · inspect the partial result before export",
                            meta.motions
                        )
                    };
                    self.view.load_scene(Ok((meta, payload)));
                    if revalidated {
                        self.view.seek(previous_prefix);
                        self.status="Retained carving still matches the machining inputs. Output settings will be checked when preparing.".into();
                    }
                    if let Some((fingerprint, prefix)) = self.resume_view.take()
                        && self.plan_fingerprint.as_ref() == Some(&fingerprint)
                    {
                        self.view.seek(prefix);
                    }
                }
            }
            Some("prepared") => {
                self.view.accept_knife_evidence(
                    &reply["bundle"]["report"]["knifeEvidence"],
                    &reply["file"]["sha256"],
                );
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
            Event::Resources { id, result } => self.accept_resources(id, result),
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
                if self.export_dialog.as_ref().is_some_and(|d| d.saving) {
                    match &result {
                        Ok(IoValue::Saved(_)) => self.export_dialog = None,
                        Err(error) => {
                            let dialog = self.export_dialog.as_mut().unwrap();
                            dialog.saving = false;
                            dialog.error = Some(error.clone());
                        }
                        _ => {}
                    }
                }
                if let Some(focus) = self.focus.take() {
                    ctx.memory_mut(|m| m.request_focus(focus));
                }
                match result {
                    Ok(IoValue::Svgs(files)) => {
                        if matches!(kind, IoKind::AddSvg)
                            && self
                                .artwork_io
                                .take()
                                .is_some_and(|(revision, _)| revision == self.revision)
                        {
                            self.artwork_command(engine::ArtworkCommand::AddMany { files }, ctx);
                        } else {
                            self.status =
                                "Document changed while choosing artwork; choose the files again."
                                    .into();
                        }
                    }
                    Ok(IoValue::Svg { filename, svg }) => {
                        self.import_file(kind, filename, svg, ctx)
                    }
                    Ok(IoValue::Job(json)) => match kind {
                        IoKind::Svg | IoKind::KnifeSvg | IoKind::AddSvg | IoKind::ReplaceSvg => {
                            self.import_file(kind, "Imported.svg".into(), json, ctx)
                        }
                        IoKind::Open => self.submit(Command::Open { json }, ctx),
                        IoKind::Migrate => self.submit(Command::Migrate { json }, ctx),
                        IoKind::LibraryImport | IoKind::MachineImport => {
                            if self.resource_import_stamp.take().as_deref()
                                != Some(self.resource_stamp().as_str())
                            {
                                self.resources.status="Library changed while choosing a file. Your edits are preserved; import again.".into();
                                self.resources.error = Some(self.resources.status.clone());
                            } else if matches!(kind, IoKind::LibraryImport) {
                                self.import_resources(&json);
                            } else {
                                self.import_machine(&json);
                            }
                        }
                        IoKind::Profile => {
                            if self.profile_io_revision.take() != Some(self.revision) {
                                self.status="Job changed while choosing a machine configuration; choose it again.".into();
                                return;
                            }
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
                        if matches!(kind, IoKind::LibraryImport | IoKind::MachineImport) {
                            self.resources.status = error.clone();
                            self.resources.error = Some(error.clone());
                        }
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
                    self.recovery.status = format!(
                        "Session recovery could not be loaded; automatic recovery is paused. You can still edit and save the job. Details: {e}"
                    );
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
            self.changed(ctx);
        }
    }
    fn current(&self) -> bool {
        !self.operation_ramp_draft
            && self.plan.as_ref().is_some_and(|(_, r)| *r == self.revision)
            && !self.document.as_ref().is_some_and(Document::pending)
    }
    fn adopt_artwork(&mut self, reply: &Value) {
        if !reply["components"].is_array() {
            return;
        }
        self.components = serde_json::from_value(reply["components"].clone()).unwrap_or_default();
        if let Some(doc) = &self.document {
            self.view.update_artwork(
                self.revision,
                doc.raw.artwork_item.clone(),
                doc.job
                    .artwork
                    .iter()
                    .map(|i| (i.id.0.clone(), i.placement.clone()))
                    .collect(),
                self.components.clone(),
                operation_selection(&doc.job, &doc.raw.operation),
            );
        }
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
        if self.io.is_none()
            && !self.ime
            && !self.resources.open
            && !self.resources.jobs_open
            && self.export_dialog.is_none()
        {
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
        self.issue_panel(ctx);
        self.view.result_current = self.current();
        self.inspector(ctx);
        self.resource_windows(ctx);
        if self.preview_dirty
            && self.active.is_none()
            && self.io.is_none()
            && self.export_dialog.is_none()
            && ctx.input(|i| i.time) - self.recovery.last_edit > 0.3
            && let Some(doc) = &self.document
        {
            let job = doc.job.to_json().unwrap();
            let command = match &self.plan {
                Some((handle, _)) => Command::ValidatePlan {
                    job,
                    handle: handle.clone(),
                    scope: self.plan_scope.clone().unwrap_or(GenerateScope::AllEnabled),
                },
                None => Command::Preview { job },
            };
            self.submit(command, ctx);
        }
        self.view.stock_loading = self.active.is_some() || self.export_dialog.is_some();
        self.view.result_current = self.current();
        self.view
            .set_knife_selected(self.operation_kind() == Some(OperationKind::DragKnife));
        self.view
            .set_profile_selected(self.operation_kind() == Some(OperationKind::Profile));
        let candidates = self.profile_candidates();
        self.view.set_profile_anchors(candidates);
        if let Some(doc) = &self.document {
            self.view.select_artwork(&doc.raw.artwork_item);
        }
        self.view.artwork.enabled = !self.simulate
            && self.active.is_none()
            && self.document.as_ref().is_some_and(|d| {
                d.active_artwork()
                    .is_some_and(|i| self.view.artwork_matches(&i.id.0, &i.placement))
            });
        self.view.artwork.revision = self.revision;
        self.view.show(ctx, self.simulate);
        for event in self.view.take_artwork_events() {
            match event {
                crate::viewport::ArtworkEvent::KnifeSelection(references) => {
                    self.artwork_command(
                        engine::ArtworkCommand::KnifeSelection { references },
                        ctx,
                    );
                    self.operation_tab = 0;
                    self.navigate(2);
                }
                crate::viewport::ArtworkEvent::CarveSelection(references) => {
                    self.artwork_command(
                        engine::ArtworkCommand::CarveSelection { references },
                        ctx,
                    );
                    self.operation_tab = 0;
                    self.navigate(2);
                }
                crate::viewport::ArtworkEvent::ProfileSelection(references) => {
                    self.viewport_profile_selection(references, ctx);
                }
                crate::viewport::ArtworkEvent::Placement {
                    item,
                    revision,
                    initial,
                    value,
                } => {
                    if revision == self.revision
                        && self.document.as_ref().is_some_and(|d| {
                            d.raw.artwork_item == item
                                && d.active_artwork().is_some_and(|i| i.placement == initial)
                        })
                    {
                        self.edit_job(ctx, &[26, 27, 28, 29], |job| {
                            job.artwork
                                .iter_mut()
                                .find(|i| i.id.0 == item)
                                .ok_or("Artwork was removed")?
                                .placement = value;
                            Ok(())
                        });
                    } else {
                        self.status = "Source changed during gesture; drag again.".into();
                    }
                }
            }
        }
        for event in self.view.take_profile_anchor_events() {
            match event {
                crate::viewport::ProfileAnchorEvent::Moved {
                    scope,
                    kind,
                    fraction,
                } => {
                    let field = anchor_field(kind);
                    let known = self.document.as_ref().is_some_and(|document| {
                        crate::profile::anchor_value(
                            &document.job,
                            &document.raw.operation,
                            &scope,
                            field,
                        )
                        .is_some()
                    });
                    if !known {
                        self.status = "That anchor no longer exists; nothing was changed.".into();
                        continue;
                    }
                    // One drag gesture is one undo transaction.
                    self.remember();
                    let result = self.document.as_mut().unwrap().edit_anchor(
                        &scope,
                        field,
                        fraction.to_string(),
                        kind,
                    );
                    match result {
                        Ok(()) => {
                            // A drag commits the value, not a long raw token:
                            // the field shows the document's own formatting.
                            if let Some(document) = &mut self.document {
                                document
                                    .raw
                                    .raw
                                    .remove(&document.raw.key_scoped(&scope, field));
                            }
                            self.changed(ctx);
                            self.edit_group = None;
                            self.status =
                                "Anchor moved; generate to update the cutting result.".into();
                        }
                        Err(error) => {
                            // The gesture changed nothing, so it is not an
                            // undoable step.
                            self.undo.pop();
                            self.status = error;
                        }
                    }
                }
            }
        }
        if self.active.is_none()
            && self.export_dialog.is_none()
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
        if !files.is_empty() && self.export_dialog.is_none() {
            if files.len() != 1 || self.io.is_some() || self.active.is_some() {
                self.status = "Drop one SVG or JSON job when the current action finishes.".into();
            } else {
                let id = self.id();
                let filename = files[0]
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| files[0].name.clone());
                let adding =
                    self.document.is_some() && filename.to_ascii_lowercase().ends_with(".svg");
                self.io = Some((id, if adding { IoKind::AddSvg } else { IoKind::Open }));
                self.artwork_io = if adding {
                    self.document
                        .as_ref()
                        .map(|d| (self.revision, d.raw.artwork_item.clone()))
                } else {
                    None
                };
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
        self.export_window(ctx);
        if self.io.is_some() && self.export_dialog.is_none() {
            egui::Modal::new(egui::Id::new("gui2-file")).show(ctx, |ui| {
                ui.heading("File action");
                ui.label("Finish or cancel the file picker to return.");
                ui.spinner();
            });
        }
        let resources_probe = json!({"open":self.resources.open,"jobsOpen":self.resources.jobs_open,"ready":self.resources.ready,"busy":self.resources.busy,"dirty":self.resources.dirty,"status":self.resources.status,"revision":self.resources.base.as_ref().map(|s|s.revision),"catalog":self.resources.draft,"selectedTool":self.resources.tool,"selectedProfile":self.resources.preset,"role":self.resources.role,"conflictRevision":self.resources.conflict.as_ref().map(|s|s.revision)});
        let job_probe = self.document.as_ref().map(|d| self.job_probe(d));
        crate::viewport::probe::publish(json!({"exportReady":self.view.export_ready(),"inspection":self.view.inspection_snapshot(),"issues":self.issues,"visibleMotions":self.view.visible_motion_range(),"bounds":self.view.scene_bounds(),"picked":self.view.artwork.selected,"gesture":self.view.artwork.mode,"controls":CONTROLS.with(|c|c.borrow().clone()),"gui2":true,"resources":resources_probe,"workspace":self.workspace(),"undo":self.undo.len(),"redo":self.redo.len(),"status":self.status,"revision":self.revision,"active":self.active.is_some(),"motions":self.view.motion_count(),"stockPrefix":self.view.stock_prefix(),"current":self.current(),"prepared":self.prepared.is_some(),"preparedSha256":self.prepared.as_ref().map(|(p,_)|p["file"]["sha256"].clone()),"job":job_probe,"pending":self.document.as_ref().is_some_and(Document::pending),"recovery":self.recovery.status}).to_string());
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
        if self.operation_ramp_draft {
            self.status =
                "Complete ramp entry before saving the job. The entry draft remains in recovery."
                    .into();
            return;
        }
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
