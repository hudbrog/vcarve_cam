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
pub(crate) mod help;
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
#[path = "source_identity.rs"]
pub(crate) mod source_identity;
#[cfg(all(feature = "ui-review", not(target_arch = "wasm32")))]
#[path = "ui_review.rs"]
pub mod ui_review;
#[path = "workspace_route.rs"]
mod workspace_route;
#[path = "workspace_ui.rs"]
mod workspace_ui;
use workspace_route::ResourcePage;

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
/// The rect a named control reported during the last frame, in viewport
/// points. Test and probe surface: the same map [`observe_control`] fills.
pub fn control_rect(label: &str) -> Option<[f32; 4]> {
    CONTROLS.with(|c| c.borrow().get(label).copied())
}
fn button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    observe_control(label, response.rect);
    response
}

/// A user action that arrived while the worker was busy. It is an *intent*, not
/// the command that was refused: it is rebuilt from the current document when
/// it runs, so a queued edit or generation never replays a stale snapshot.
#[derive(Clone, Debug)]
enum Pending {
    Operation(Box<crate::operation_authoring::Action>),
    Artwork {
        operation_id: String,
        action: Box<engine::ArtworkCommand>,
    },
    Resource(Box<crate::resources::ResourceCommand>),
    Generate(GenerateScope),
}

impl Pending {
    /// Only actions that can be rebuilt from the current document are queued.
    /// Anything else keeps the pre-GUI9 behaviour of refusing while busy, but
    /// says so instead of doing nothing.
    fn from_command(command: &Command) -> Option<Self> {
        match command {
            Command::Operation { action, .. } => Some(Self::Operation(Box::new(action.clone()))),
            Command::Artwork {
                operation_id,
                action,
                ..
            } => Some(Self::Artwork {
                operation_id: operation_id.clone(),
                action: Box::new(action.clone()),
            }),
            Command::Resource { action, .. } => Some(Self::Resource(action.clone())),
            Command::Generate { scope, .. } => Some(Self::Generate(scope.clone())),
            _ => None,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Operation(_) => "operation edit",
            Self::Artwork { .. } => "artwork edit",
            Self::Resource(_) => "resource edit",
            Self::Generate(_) => "generation",
        }
    }
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
        Some(OperationSettingsV5::Drill(settings)) => settings.points.clone(),
        _ => vec![],
    }
}

/// Stable field IDs use the same recovery keys as the qualified input binder.
pub const LIVE_FIELDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 8, 9, 10];

/// The workspace's stock-editing context. It is deliberately outside the
/// document: the rectangle a resize produces is what gets saved, so a job
/// edited by an older build still means exactly the same thing.
#[derive(Clone, Copy, Debug, Default)]
pub struct StockEditContext {
    /// Which point of the rectangle a width or length edit keeps still.
    pub anchor: cam_core::project::v5::commands::StockAnchor,
    /// Placed artwork bounds, when the workspace has resolved them.
    pub artwork: Option<cam_core::project::v5::SetupBounds>,
}

/// Resolve a stock-rectangle edit against the chosen anchor. Only a size edit
/// (width or length, fields 42/43) consults it — moving the minimum corner is
/// already an explicit statement about where the stock is. Returns the values
/// to commit and the corner the fields should then show. The placement of one
/// artwork item is never part of this: only the stock rectangle moves.
pub fn anchored_stock_values(
    values: &[f64],
    previous: Option<cam_core::project::RectXY>,
    field: usize,
    anchor: cam_core::project::v5::commands::StockAnchor,
    artwork: Option<cam_core::project::v5::SetupBounds>,
) -> (Vec<f64>, [f64; 4]) {
    use cam_core::project::v5::commands::{StockAnchor, anchor_stock_rectangle};
    let requested = cam_core::project::RectXY {
        min_x_mm: values[0],
        min_y_mm: values[1],
        width_mm: values[2],
        length_mm: values[3],
    };
    let rect = if matches!(field, 42 | 43) && anchor != StockAnchor::MinCorner {
        anchor_stock_rectangle(requested, previous, anchor, artwork)
    } else {
        requested
    };
    (
        vec![rect.min_x_mm, rect.min_y_mm, rect.width_mm, rect.length_mm],
        [rect.min_x_mm, rect.min_y_mm, rect.width_mm, rect.length_mm],
    )
}

fn kind_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::FlatVcarve => "flat_vcarve",
        OperationKind::Face => "face",
        OperationKind::Profile => "profile",
        OperationKind::DragKnife => "drag_knife",
        OperationKind::Drill => "drill",
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
        Some(OperationKind::Drill) => crate::drill::value(job, operation_id, field),
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
        Some(OperationKind::Drill) => {
            crate::drill::set(&mut job, operation_id, field, value)?;
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
        if matches!(field, 32 | 33 | 38 | 39)
            && let Some(tool) =
                crate::authoring::tool_in(&self.job, &self.raw.operation, field >= 38)
        {
            return self.mapping_text(&tool.id, matches!(field, 33 | 39));
        }
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
        self.edit_with(field, text, StockEditContext::default())
    }
    /// One field edit with the workspace's stock context: which point of the
    /// stock rectangle a size edit keeps still, and the placed artwork bounds
    /// the artwork anchor is measured against.
    pub fn edit_with(
        &mut self,
        field: usize,
        text: String,
        stock: StockEditContext,
    ) -> Result<(), String> {
        if matches!(field, 32 | 33 | 38 | 39)
            && let Some(tool) =
                crate::authoring::tool_in(&self.job, &self.raw.operation, field >= 38)
        {
            let id = tool.id.clone();
            return self.edit_mapping(&id, matches!(field, 33 | 39), text);
        }
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
            if matches!(field, 40..=43) {
                // A width or length edit is also a decision about where the
                // rectangle sits. The anchor moves the stock rectangle only;
                // an explicit min-corner edit is taken as written.
                let (values, corner) = anchored_stock_values(
                    &values,
                    self.job.setup.stock.xy,
                    field,
                    stock.anchor,
                    stock.artwork,
                );
                crate::authoring::set_group_in(
                    &mut candidate,
                    &self.raw.operation,
                    field,
                    &values,
                )?;
                candidate.validate_structure().map_err(|e| e.to_string())?;
                self.job = candidate;
                // The fields carry the corner the anchor resolved, so the next
                // size edit starts from the rectangle the job actually has.
                // Only fields the user is not typing into are rewritten, and
                // only when the number actually changed: the field being
                // edited keeps its own text, and a value that already parses
                // to the resolved number keeps its spelling.
                for member in 40..=43 {
                    if member == field {
                        continue;
                    }
                    let key = self.raw.key(member);
                    let resolved = corner[member - 40];
                    let current = self
                        .raw
                        .raw
                        .get(&key)
                        .and_then(|text| Draft::parse(text).ok().flatten());
                    if current.is_none_or(|value| (value - resolved).abs() > 1e-9) {
                        self.raw.raw.insert(key, resolved.to_string());
                    }
                }
                return Ok(());
            }
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
                if crate::state::is_mapping(scope, operation, field) {
                    let active = self.job.tools.iter().any(|tool| tool.id == operation)
                        && self.job.machine_configuration.as_ref().is_some_and(|m| {
                            field == 32
                                || m.length_compensation
                                    == Some(cam_core::post::LengthCompensation::ToolTable)
                        });
                    return active
                        && Draft::parse(text).ok()
                            != Some(self.mapping_value(operation, field == 33));
                }
                if matches!(field, 32 | 33 | 38 | 39) {
                    let length = matches!(field, 33 | 39);
                    if let Some(tool) = self.legacy_mapping_tool(key, length) {
                        return self.job.machine_configuration.as_ref().is_some_and(|m| {
                            !length
                                || m.length_compensation
                                    == Some(cam_core::post::LengthCompensation::ToolTable)
                        }) && Draft::parse(text).ok()
                            != Some(self.mapping_value(tool, length));
                    }
                }
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
    /// Status line of the command currently in the worker, so a queued action
    /// can say what it is waiting for.
    busy: Option<String>,
    /// One user action that arrived while the worker was busy. Bounded to one:
    /// a later action replaces an earlier one, because a queue of stale edits
    /// is worse than doing the latest thing the user asked for.
    pending: Option<Pending>,
    plan: Option<(String, u64)>,
    plan_scope: Option<GenerateScope>,
    active_scope: Option<GenerateScope>,
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
    /// Drillable marker points of the current artwork, cached per document
    /// revision (the same freshness contract as `components`).
    points: Vec<cam_core::project::v5::artwork::PointEntry>,
    points_revision: u64,
    inspector_tab: usize,
    operation_tab: usize,
    operation_picker: Option<usize>,
    operation_rename: Option<(String, String)>,
    operation_scroll: [f32; 3],
    operation_views: std::collections::BTreeMap<String, (usize, [f32; 3])>,
    operation_ramp_draft: bool,
    preview_dirty: bool,
    inspector_width: f32,
    navigator_width: f32,
    navigator_collapsed: bool,
    inspector_collapsed: bool,
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
    resource_page: Option<ResourcePage>,
    resource_request: Option<(u64, ResourceIntent)>,
    resource_import_stamp: Option<String>,
    profile_io_revision: Option<u64>,
    /// Which point of the stock rectangle a width/length edit keeps still
    /// (W5). Workspace state, not document state: the rectangle it produces
    /// is the saved value, so a job saved by an older build still means the
    /// same thing.
    pub stock_anchor: cam_core::project::v5::commands::StockAnchor,
}
#[derive(Clone, Copy)]
enum ResourceIntent {
    Load,
    Compare,
    Save,
}
#[derive(Clone, Copy)]
enum IoKind {
    Svg,
    AddSvg,
    ReplaceSvg,
    Open,
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
            busy: None,
            pending: None,
            plan: None,
            plan_scope: None,
            active_scope: None,
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
            points: vec![],
            points_revision: 0,
            inspector_tab: 2,
            operation_tab: 0,
            operation_picker: None,
            operation_rename: None,
            operation_scroll: [0.; 3],
            operation_views: Default::default(),
            operation_ramp_draft: false,
            preview_dirty: false,
            inspector_width: crate::ui_theme::INSPECTOR,
            navigator_width: crate::ui_theme::NAVIGATOR,
            navigator_collapsed: false,
            inspector_collapsed: false,
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
            resource_page: None,
            resource_request: None,
            resource_import_stamp: None,
            profile_io_revision: None,
            stock_anchor: Default::default(),
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
    /// The placed artwork bounds the workspace already holds: the resolved
    /// component bounds from the last artwork reply, unioned. `None` until a
    /// reply arrives, and when nothing placed.
    pub fn artwork_bounds(&self) -> Option<cam_core::project::v5::SetupBounds> {
        self.components.iter().fold(None, |bounds, component| {
            let [min_x_mm, min_y_mm, max_x_mm, max_y_mm] = component.bounds;
            let next = cam_core::project::v5::SetupBounds {
                min_x_mm,
                min_y_mm,
                max_x_mm,
                max_y_mm,
            };
            Some(match bounds {
                Some(bounds) => bounds.union(next),
                None => next,
            })
        })
    }
    /// Placed bounds per artwork item, in document order, named as the user
    /// sees them. Items that place nothing are left out.
    pub fn placed_artwork_bounds(&self) -> Vec<(String, cam_core::project::v5::SetupBounds)> {
        let Some(document) = &self.document else {
            return vec![];
        };
        let mut items: Vec<(String, Option<cam_core::project::v5::SetupBounds>)> = document
            .job
            .artwork
            .iter()
            .map(|item| (item.name.clone(), None))
            .collect();
        for component in &self.components {
            let id = &component.reference.artwork_item_id.0;
            let Some(index) = document
                .job
                .artwork
                .iter()
                .position(|item| &item.id.0 == id)
            else {
                continue;
            };
            let [min_x_mm, min_y_mm, max_x_mm, max_y_mm] = component.bounds;
            let next = cam_core::project::v5::SetupBounds {
                min_x_mm,
                min_y_mm,
                max_x_mm,
                max_y_mm,
            };
            items[index].1 = Some(match items[index].1 {
                Some(bounds) => bounds.union(next),
                None => next,
            });
        }
        items
            .into_iter()
            .filter_map(|(name, bounds)| bounds.map(|bounds| (name, bounds)))
            .collect()
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
            // The worker holds one command at a time. A user action that can be
            // rebuilt from the current document waits; anything else is refused
            // out loud rather than dropped in silence.
            let busy = self
                .busy
                .clone()
                .unwrap_or_else(|| "A task is still running.".into());
            match Pending::from_command(&command) {
                Some(intent) => {
                    let replaced = self.pending.replace(intent.clone()).is_some();
                    self.status = format!(
                        "{busy} The queued {} will run when it finishes{}.",
                        intent.label(),
                        if replaced {
                            ", replacing the earlier queued action"
                        } else {
                            ""
                        }
                    );
                }
                None => {
                    self.status = format!("{busy} Finish or cancel it before doing that.");
                }
            }
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
            Command::ImportSvg { .. } => "Importing SVG…",
            Command::Artwork { .. } => "Updating artwork collection…",
            Command::Operation { .. } => "Updating operations…",
            Command::Resource { .. } => "Copying reviewed resource values…",
            Command::Preview { .. } => "Updating artwork preview…",
            Command::ValidatePlan { .. } => {
                "Checking whether the retained execution still matches…"
            }
            Command::Seek { .. } => "Loading stock position…",
            Command::DisplayPreset { .. } => "Rebuilding the display raster…",
            Command::Open { .. } => "Opening document…",
            Command::ApplyProfile { .. } => "Applying machine configuration…",
            Command::Generate { .. } => "Generating toolpaths and stock checkpoints…",
            Command::Prepare { .. } => {
                "Checking retained execution and reading back emitted output…"
            }
        }
        .into();
        self.active_scope = match &command {
            Command::Generate { scope, .. } => Some(scope.clone()),
            _ => None,
        };
        self.busy = Some(self.status.clone());
        self.port
            .start(id, Request::Gui2(Box::new(command)), ctx.clone());
    }

    /// Generate the whole enabled list, or the prefix ending at one operation.
    /// The command a queued intent means *now*, rebuilt from the current
    /// document rather than replayed from the snapshot that was refused.
    fn pending_command(&self, pending: &Pending) -> Option<Command> {
        let document = self.document.as_ref()?;
        if document.pending() {
            return None;
        }
        let job = document.job.to_json().ok()?;
        Some(match pending {
            Pending::Operation(action) => Command::Operation {
                job,
                action: (**action).clone(),
            },
            Pending::Artwork {
                operation_id,
                action,
            } => Command::Artwork {
                job,
                operation_id: operation_id.clone(),
                action: (**action).clone(),
            },
            Pending::Resource(action) => Command::Resource {
                job,
                action: action.clone(),
            },
            Pending::Generate(scope) => Command::Generate {
                job,
                scope: scope.clone(),
                preset: self.view.desired_preset(),
            },
        })
    }

    /// Run the queued action once the worker is free. The status keeps saying
    /// Cancel the command in the worker. The worker — and with it the retained
    /// execution — stops, so the plan handle is dropped and the display is
    /// stale until the next generation. The document, the last displayed result
    /// and any checked bundle already read back survive, and a queued action
    /// still runs when the worker is free.
    pub(crate) fn cancel_compute(&mut self) {
        self.cancelled_id = self.active.map(|active| active.0);
        self.port.cancel();
        self.active = None;
        self.busy = None;
        self.plan = None;
        self.plan_scope = None;
        self.status = match &self.pending {
            Some(intent) => format!(
                "Compute cancelled. Draft retained. The queued {} runs next.",
                intent.label()
            ),
            None => "Compute cancelled. Draft retained.".into(),
        };
        self.view.cancel_stock_requests();
    }

    /// what is happening, so a queued action is never a silent deferral.
    fn run_pending(&mut self, ctx: &egui::Context) {
        let Some(intent) = self.pending.take() else {
            return;
        };
        match self.pending_command(&intent) {
            Some(command) => {
                self.status = format!("Running the queued {}.", intent.label());
                self.submit(command, ctx);
            }
            None => {
                self.status = "The queued action no longer applies to the current document.".into();
            }
        }
    }

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
        self.submit(
            Command::Generate {
                job,
                scope,
                preset: self.view.desired_preset(),
            },
            ctx,
        );
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
            Some(OperationKind::Profile) => {
                let settings = crate::profile::settings_in(&d.job, &selected);
                json!({
                    "kind": "profile",
                    "profile": settings,
                    "stepdown": settings.and_then(|s| s.stepdown_mm),
                    "contours": settings.map(|s| s.contours.len()).unwrap_or(0),
                    "sides": settings.map(|s| {
                        s.contours.iter().map(|c| c.side).collect::<Vec<_>>()
                    }),
                    "direction": settings.and_then(|s| s.direction),
                    "finish": settings.map(|s| &s.finish),
                    "tabs": settings.and_then(|s| s.tabs.as_ref()),
                    "start": settings.map(|s| &s.start),
                    "entry": settings.map(|s| &s.entry),
                    "leadIn": settings.map(|s| &s.lead_in),
                    "rawStepdown": d.text(8),
                    "machine": d.job.machine_configuration.is_some(),
                    "profileStatuses": cam_core::project::v5::resources::assignment_statuses(&d.job),
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
            matches!(kind, IoKind::Svg | IoKind::AddSvg | IoKind::ReplaceSvg),
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
        self.busy = None;
        let exporting = self
            .export_dialog
            .as_ref()
            .is_some_and(|d| d.request == Some(id));
        if exporting {
            self.export_dialog.as_mut().unwrap().request = None;
        }
        if revision != self.revision {
            self.view.finish_stock_request();
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
                self.view.finish_stock_request();
                if exporting {
                    self.export_dialog.as_mut().unwrap().error = Some(error.clone());
                }
                self.status = error;
                return;
            }
        };
        let reply = &meta.report["gui2"];
        if meta.report["protocol"] != engine::PROTOCOL {
            self.view.finish_stock_request();
            self.status = "GUI2 UI/worker version mismatch; reload matching assets.".into();
            if exporting {
                self.export_dialog.as_mut().unwrap().error = Some(self.status.clone());
            }
            self.plan = None;
            self.plan_scope = None;
            return;
        }
        if reply["kind"] != "seek" {
            self.view.finish_stock_request();
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
                    if reply["clearMappings"] == true {
                        doc.clear_mapping_drafts(false);
                    }
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
                "artwork"
                | "carve_selection"
                | "knife_start"
                | "knife_selection"
                | "knife_outlines"
                | "profile_selection"
                | "profile_tab_anchor"
                | "profile_start"
                | "profile_anchor_reattach",
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
                self.status = match reply["kind"].as_str() {
                    Some("carve_selection" | "knife_selection" | "profile_selection") => {
                        "Operation geometry updated. Undo restores the previous selection."
                    }
                    Some(
                        "knife_start" | "profile_start" | "profile_tab_anchor"
                        | "profile_anchor_reattach",
                    ) => "Operation anchors updated. Undo restores the previous anchors.",
                    _ => "Artwork updated. Assignments retain their exact source revisions; repair unresolved references or Undo.",
                }
                .into();
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
            Some("seek") => {
                match self.view.accept_stock(meta, payload) {
                    Ok(()) => self.status = "Stock position loaded from retained execution.".into(),
                    Err(e) => self.status = e,
                }
                self.view.finish_stock_request();
            }
            Some("preset") => {
                // The rebuilt raster belongs to the plan we asked about. A
                // response for another execution is refused instead of being
                // shown next to the current one.
                let expected = self.plan.as_ref().map(|(handle, _)| handle.clone());
                let answered = reply["handle"].as_str().map(str::to_owned);
                let preset = reply["preset"].as_str().unwrap_or("standard").to_owned();
                if expected != answered {
                    self.status =
                        "The rebuilt display belongs to another execution; generate again.".into();
                } else {
                    match self.view.adopt_preset(meta, payload) {
                        Ok(()) => {
                            self.status = format!(
                                "Display rebuilt at the {preset} resolution for the same execution."
                            )
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
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
                    raw.raw.retain(|key, _| !key.starts_with("mapping/"));
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
                if reply["kind"] == "profile" {
                    self.document.as_mut().unwrap().clear_mapping_drafts(false);
                }
                self.changed(ctx);
                if reply["kind"] != "profile" {
                    self.operation_tab = 0;
                    self.operation_scroll = [0.; 3];
                    self.operation_views.clear();
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
                    // Importing artwork adds artwork only: the operation list
                    // stays empty until the user adds an operation to cut with.
                    self.status = if self
                        .document
                        .as_ref()
                        .is_some_and(|d| d.job.operations.is_empty())
                    {
                        "SVG imported. Add an operation from the Operations list, then select the geometry it cuts; no machining defaults were copied.".into()
                    } else {
                        "SVG imported. In the operation's Geometry to carve, or by clicking filled regions in the viewport, select the components to cut; no machining defaults were copied.".into()
                    };
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
        // Events one frame consumes. A burst is finished on later frames instead
        // of starving the frame; nothing is dropped, because the rest stay in
        // the channel and the repaint below comes straight back for them.
        const EVENTS_PER_FRAME: usize = 64;
        let mut consumed = 0;
        while consumed < EVENTS_PER_FRAME {
            let Some(event) = self.port.poll() else {
                break;
            };
            self.event(event, ctx);
            consumed += 1;
        }
        if consumed == EVENTS_PER_FRAME {
            ctx.request_repaint();
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
                        IoKind::Svg | IoKind::AddSvg | IoKind::ReplaceSvg => {
                            self.import_file(kind, "Imported.svg".into(), json, ctx)
                        }
                        IoKind::Open => self.submit(Command::Open { json }, ctx),
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
            Event::RecoveryCleared(result) => match result {
                Ok(()) => {
                    // The unreadable record is gone: recovery resumes from a
                    // fresh revision the next time the job is edited.
                    self.recovery.failed = false;
                    self.recovery.ready = true;
                    self.recovery.revision = None;
                    self.recovery.offered = None;
                    self.recovery.status =
                        "Stored recovery discarded; new edits are recovered again.".into();
                }
                Err(e) => {
                    self.recovery.status = format!("Could not discard the stored recovery: {e}");
                }
            },
            Event::RecoverySaved { edit, result } => self.recovery.written(edit, result),
            Event::Notice(text) => self.status = text,
            Event::OfflineStatus(_) => {}
        }
    }
    fn undo(&mut self, ctx: &egui::Context) {
        if let Some(previous) = self.undo.pop() {
            let previous_operation = self.operation_id();
            if let Some(current) = self.document.replace(previous) {
                self.redo.push(current);
                Self::trim_history(&mut self.redo);
            }
            self.restore_operation_view(&previous_operation);
            self.edit_group = None;
            self.changed(ctx);
        }
    }
    fn redo(&mut self, ctx: &egui::Context) {
        if let Some(next) = self.redo.pop() {
            let previous_operation = self.operation_id();
            if let Some(current) = self.document.replace(next) {
                self.undo.push(current);
                Self::trim_history(&mut self.undo);
            }
            self.restore_operation_view(&previous_operation);
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
            && !self.library_open()
            && self.resource_page != Some(ResourcePage::JobTools)
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
        if self.resource_page.is_none() {
            self.inspector(ctx);
        }
        let resource_presented = self.resource_page.is_some();
        self.resource_windows(ctx);
        // A user action that arrived while the worker was busy runs first, once
        // the worker is free: it was the click the user actually made.
        if self.active.is_none() && self.io.is_none() && self.export_dialog.is_none() {
            self.run_pending(ctx);
        }
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
                    preset: self.view.desired_preset(),
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
        if !resource_presented {
            self.view.show(ctx, self.simulate);
        }
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
            && let Some((handle, _)) = &self.plan
            && let Some(prefix) = self.view.take_stock_request()
        {
            self.submit(
                Command::Seek {
                    handle: handle.clone(),
                    prefix,
                },
                ctx,
            );
        }
        if self.plan.is_none() && self.active.is_none() && self.view.stock_request_pending() {
            self.view.cancel_stock_requests();
            self.status = "The retained execution is unavailable. Generate again to restore another stock position.".into();
        }
        // A display resolution change re-derives the raster from the retained
        // execution. It waits for the same idle point a seek does, so a slow
        // rebuild cannot overlap a stock request.
        if self.active.is_none()
            && self.export_dialog.is_none()
            && !self.view.stock_request_pending()
            && let Some(preset) = self.view.take_preset_request()
            && let Some((handle, _)) = &self.plan
        {
            self.submit(
                Command::DisplayPreset {
                    handle: handle.clone(),
                    preset,
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
        let resources_probe = json!({"page":self.resource_page.map(|p|format!("{p:?}")),"raw":self.resources.raw,"invalid":self.resources.invalid,"jobRaw":self.resources.job_raw,"selectedJobTool":self.resources.job_tool,"selectedAssignment":self.resources.job_assignment,"open":self.library_open(),"jobsOpen":self.resource_page == Some(ResourcePage::JobTools),"ready":self.resources.ready,"busy":self.resources.busy,"dirty":self.resources.dirty,"status":self.resources.status,"revision":self.resources.base.as_ref().map(|s|s.revision),"catalog":self.resources.draft,"selectedTool":self.resources.tool,"selectedProfile":self.resources.preset,"role":self.resources.role,"conflictRevision":self.resources.conflict.as_ref().map(|s|s.revision)});
        let job_probe = self.document.as_ref().map(|d| self.job_probe(d));
        let stock_transfer = self.view.last_stock_transfer();
        let display = self.view.display_probe();
        let renderer = self.view.renderer_probe();
        crate::viewport::probe::publish(json!({"planScope":self.plan_scope,"activeScope":self.active.as_ref().and(self.active_scope.as_ref()),"exportReady":self.view.export_ready(),"inspection":self.view.inspection_snapshot(),"issues":self.issues,"visibleMotions":self.view.visible_motion_range(),"bounds":self.view.scene_bounds(),"picked":self.view.artwork.selected,"gesture":self.view.artwork.mode,"controls":CONTROLS.with(|c|c.borrow().clone()),"gui2":true,"resources":resources_probe,"workspace":self.workspace(),"undo":self.undo.len(),"redo":self.redo.len(),"status":self.status,"revision":self.revision,"active":self.active.is_some(),"busy":self.busy,"queued":self.pending.as_ref().map(Pending::label),"motions":self.view.motion_count(),"stockPrefix":self.view.stock_prefix(),"requestedStock":self.view.requested_stock_prefix(),"stockTransferBytes":stock_transfer.0,"stockReplayed":stock_transfer.1,"timelineRows":self.view.timeline_rows(),"display":display,"renderer":renderer,"current":self.current(),"prepared":self.prepared.is_some(),"preparedSha256":self.prepared.as_ref().map(|(p,_)|p["file"]["sha256"].clone()),"job":job_probe,"pending":self.document.as_ref().is_some_and(Document::pending),"recovery":self.recovery.status}).to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> CamJobV5 {
        CamJobV5::from_json(engine::FLOWER).unwrap()
    }

    fn app_with_document() -> App {
        let mut app = App::default();
        let job = job();
        app.document = Some(Document {
            raw: Draft::for_job(&job),
            job,
            finish_draft: None,
        });
        if let Some(document) = &mut app.document {
            document.raw.operation = document
                .job
                .operations
                .first()
                .map(|operation| operation.id.clone())
                .unwrap_or_default();
        }
        app
    }

    /// A scene with a motion count and no geometry: enough for the display
    /// request paths, which only need the motion range.
    fn scene_with_motions(motions: usize) -> crate::compute::Scene {
        let motion_len = motions * 2 * 28;
        crate::compute::Scene {
            meta: crate::compute::SceneMeta {
                protocol: crate::compute::PROTOCOL.into(),
                name: "flower".into(),
                report: json!({"gui2": {"kind": "generated"}}),
                job: engine::FLOWER.into(),
                programs: vec![],
                bounds: [0., 0., 10., 10.],
                contour_vertices: 0,
                rough_vertices: 0,
                motions,
                page_motions: crate::pages::PAGE_MOTIONS,
                motion_offset: 0,
                motion_len,
                payload_bytes: motion_len,
                payload_sha256: "0".repeat(64),
                sections: vec![],
                stock: None,
                sim: None,
                transport: Default::default(),
            },
            payload: std::sync::Arc::new(vec![0u8; motion_len]),
        }
    }

    /// A user action that arrives while the worker is busy is queued as an
    /// intent, and the queue is bounded to one: the newest action wins instead
    /// of piling up stale snapshots.
    #[test]
    fn a_command_that_arrives_while_busy_is_queued_once_and_replaced_by_the_newest() {
        let mut app = app_with_document();
        let operation = app.document.as_ref().unwrap().raw.operation.clone();
        app.active = Some((1, app.revision));
        app.busy = Some("Generating toolpaths…".into());
        app.submit(
            Command::Operation {
                job: app.document.as_ref().unwrap().job.to_json().unwrap(),
                action: crate::operation_authoring::Action::SetEnabled {
                    operation_id: operation.clone(),
                    enabled: false,
                },
            },
            &egui::Context::default(),
        );
        assert!(matches!(app.pending, Some(Pending::Operation(_))));
        assert!(
            app.status.contains("Generating toolpaths"),
            "{}",
            app.status
        );
        assert!(
            app.active.is_some(),
            "the running command was not disturbed"
        );

        // A later action replaces the earlier one rather than queueing behind it.
        app.submit(
            Command::Generate {
                job: app.document.as_ref().unwrap().job.to_json().unwrap(),
                scope: GenerateScope::AllEnabled,
                preset: crate::stock_preview::DisplayPreset::Standard,
            },
            &egui::Context::default(),
        );
        assert!(matches!(app.pending, Some(Pending::Generate(_))));
        assert!(app.status.contains("replacing the earlier queued action"));

        // Something that cannot be rebuilt from the document is refused out
        // loud instead of being dropped in silence.
        app.submit(
            Command::Open {
                json: engine::FLOWER.into(),
            },
            &egui::Context::default(),
        );
        assert!(matches!(app.pending, Some(Pending::Generate(_))));
        assert!(app.status.contains("Finish or cancel"), "{}", app.status);
    }

    /// The queued generation is rebuilt from the document as it is *now*, so an
    /// edit made while the worker was busy is included.
    #[test]
    fn a_queued_generation_uses_the_current_document_and_resolution() {
        let mut app = app_with_document();
        let operation = app.document.as_ref().unwrap().raw.operation.clone();
        app.document.as_mut().unwrap().job = set_value(&job(), &operation, 6, Some(12.)).unwrap();
        let current = app.document.as_ref().unwrap().job.to_json().unwrap();
        app.view
            .set_display_preset(crate::stock_preview::DisplayPreset::Fine);
        let command = app
            .pending_command(&Pending::Generate(GenerateScope::AllEnabled))
            .unwrap();
        match command {
            Command::Generate { job, preset, .. } => {
                assert_eq!(preset, crate::stock_preview::DisplayPreset::Fine);
                assert_eq!(
                    job, current,
                    "the queued generation carries the current edit, not a stale snapshot"
                );
            }
            other => panic!("expected a generation, got {other:?}"),
        }
        // The same intent for a document with uncommitted raw text is not run.
        let mut app = app_with_document();
        let key = app.document.as_ref().unwrap().raw.key(2);
        app.document
            .as_mut()
            .unwrap()
            .raw
            .raw
            .insert(key, "7".into());
        assert!(
            app.pending_command(&Pending::Generate(GenerateScope::AllEnabled))
                .is_none()
        );
    }

    /// The renderer drill is a display drill: injecting a failure and rebuilding
    /// Cancelling stops the worker, so the retained execution is gone; the
    /// Display requests coalesce: a scrub sets one pending target, and each new
    /// request replaces it, so the worker is never asked for a position the
    /// user has already moved past. A completion for another request id is
    /// ignored rather than shown.
    #[test]
    fn accepting_a_stock_reply_keeps_the_fraction_requested_by_the_time_track() {
        let mut app = app_with_document();
        let ctx = egui::Context::default();
        let (meta, payload) = engine::run(Command::generate(include_str!(
            "../../../fixtures/gui4/lettering.job.json"
        )))
        .unwrap();
        let handle = meta.report["gui2"]["handle"].as_str().unwrap().to_owned();
        app.view.load_scene(Ok((meta, payload)));
        let total = app.view.program_time().unwrap().1;
        let frame = |app: &mut App, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000., 700.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.view.show(ctx, true),
            );
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        for (id, fraction) in [(1, 0.73), (2, 0.21)] {
            let rect = control_rect("Program time").unwrap();
            let point = egui::pos2(
                rect[0] + 6. + (rect[2] - rect[0] - 12.) * fraction,
                (rect[1] + rect[3]) / 2.,
            );
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            if let Some(prefix) = app.view.take_stock_request() {
                let result = engine::run(Command::Seek {
                    handle: handle.clone(),
                    prefix,
                });
                app.active = Some((id, app.revision));
                app.accept(id, result, &ctx);
            }
            assert!((app.view.program_time().unwrap().0 - total * fraction as f64).abs() < 0.001);
            assert_eq!(app.view.requested_stock_prefix(), None);
        }
    }

    #[test]
    fn display_requests_coalesce_and_late_answers_are_ignored() {
        let mut app = app_with_document();
        app.view.set_scene(scene_with_motions(2_000));
        app.view.seek(10);
        app.view.seek(2_000);
        app.view.seek(600);
        assert_eq!(
            app.view.take_stock_request(),
            Some(600),
            "only the latest scrub target is submitted"
        );
        assert_eq!(app.view.requested_stock_prefix(), Some(600));
        app.view
            .set_display_preset(crate::stock_preview::DisplayPreset::Fine);
        app.view
            .set_display_preset(crate::stock_preview::DisplayPreset::Coarse);
        assert_eq!(
            app.view.take_preset_request(),
            Some(crate::stock_preview::DisplayPreset::Coarse),
            "only the latest resolution is rebuilt"
        );

        // A completion that belongs to another request cannot touch this one.
        app.active = Some((7, app.revision));
        app.status = "Loading stock position…".into();
        app.accept(
            6,
            Err("late answer for another request".into()),
            &egui::Context::default(),
        );
        assert_eq!(app.active, Some((7, app.revision)));
        assert_eq!(app.status, "Loading stock position…");
    }

    /// Cancelling stops the worker, so the retained execution is gone; the
    /// document, the last displayed result and any checked bundle are not, and
    /// a queued action still runs.
    #[test]
    fn cancelling_a_computation_keeps_the_draft_and_the_checked_bundle() {
        let mut app = app_with_document();
        app.revision = 3;
        app.saved_revision = Some(3);
        app.plan = Some(("handle-1".into(), 3));
        app.plan_scope = Some(GenerateScope::AllEnabled);
        app.prepared = Some((json!({"file": {"sha256": "abc"}}), 3));
        app.active = Some((9, 3));
        app.busy = Some("Generating toolpaths…".into());
        app.pending = Some(Pending::Generate(GenerateScope::AllEnabled));
        app.view.set_scene(scene_with_motions(200));
        app.view.seek(50);
        assert_eq!(app.view.take_stock_request(), Some(50));
        app.view.seek(80);
        app.cancel_compute();
        assert!(app.active.is_none());
        assert!(app.busy.is_none());
        assert_eq!(app.view.requested_stock_prefix(), None);
        assert!(!app.view.stock_request_pending());
        assert!(
            app.plan.is_none(),
            "the retained execution died with the worker"
        );
        assert!(app.plan_scope.is_none());
        assert!(
            app.prepared.is_some(),
            "a checked bundle read back before the cancellation is still retryable"
        );
        assert!(app.document.is_some(), "the draft survives a cancellation");
        assert!(app.status.contains("cancelled"), "{}", app.status);
        assert!(
            app.pending.is_some(),
            "the queued action still runs once the worker is free"
        );
        assert!(app.status.contains("queued generation"), "{}", app.status);
    }

    /// the resources must not touch the document, the retained result, the
    /// playhead, the raster resolution or the camera.
    #[test]
    fn the_renderer_drill_preserves_the_document_result_and_view() {
        let mut app = app_with_document();
        app.revision = 7;
        app.saved_revision = Some(7);
        app.plan = Some(("handle-1".into(), 7));
        app.prepared = Some((json!({"file": {"sha256": "abc"}}), 7));
        app.view.set_scene(crate::compute::Scene {
            meta: crate::compute::SceneMeta {
                protocol: crate::compute::PROTOCOL.into(),
                name: "flower".into(),
                report: json!({"gui2": {"kind": "generated"}}),
                job: engine::FLOWER.into(),
                programs: vec![],
                bounds: [0., 0., 10., 10.],
                contour_vertices: 0,
                rough_vertices: 0,
                motions: 0,
                page_motions: crate::pages::PAGE_MOTIONS,
                motion_offset: 0,
                motion_len: 0,
                payload_bytes: 0,
                payload_sha256: "0".repeat(64),
                sections: vec![],
                stock: None,
                sim: None,
                transport: Default::default(),
            },
            payload: std::sync::Arc::new(Vec::new()),
        });
        app.view.restore_settings(&crate::viewport::ViewSettings {
            isometric: true,
            tilt_deg: None,
            zoom: 1.4,
            yaw: 0.3,
            pan: [0., 0.],
            stock_style: crate::stock_style::StockStyle::default(),
            stage: 1,
            stock: true,
            prefix: 123,
            inspection_xy: Some([9., 23.]),
            inspection_tab: 1,
            section_y: true,
            preset: crate::stock_preview::DisplayPreset::Fine,
        });
        app.view.inject_renderer_failure();
        assert!(app.view.renderer_probe()["unavailable"] == true);
        let settings = app.view.settings();
        let revision = app.revision;
        let plan = app.plan.clone();
        let prepared = app.prepared.clone();

        app.view.rebuild_renderer_resources();
        let probe = app.view.renderer_probe();
        assert!(probe["unavailable"] == false);
        assert_eq!(app.view.settings(), settings, "the view is unchanged");
        assert_eq!(app.revision, revision);
        assert_eq!(app.plan, plan);
        assert_eq!(app.prepared, prepared);
        assert!(
            app.document.is_some(),
            "the document survives a renderer failure"
        );
    }
}
