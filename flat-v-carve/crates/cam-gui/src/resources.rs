//! Reusable resources have their own revision, editing buffer and persistence.
//! The portable job receives copies only through explicit core commands.
use cam_core::{
    post::sequence::SequenceProfile,
    project::v5::{
        self, CamJobV5,
        resources::{self as core, AssignmentRole},
    },
    tool_library::{CuttingPreset, LibraryGeometry, LibraryTool, ToolLibrary},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_BYTES: usize = 8_000_000;

/// Explain missing machine guarantees without relaxing the checked-output contract.
pub fn machine_problems(machine: &SequenceProfile) -> Vec<String> {
    let mut issues = Vec::new();
    let m = &machine.m6;
    for (missing, message) in [
        (
            m.reference.trim().is_empty(),
            "Add tool-change notes/reference: a manual section, macro revision, or verified procedure.",
        ),
        (
            !m.reviewed,
            "Review the tool-change procedure and confirm Contract reviewed.",
        ),
        (
            !m.preserves_work_datum,
            "Confirm the same work zero is preserved without rotation after changing the tool.",
        ),
        (
            !m.local_offsets_unused,
            "Confirm the procedure leaves G52/G92 local coordinate shifts unused.",
        ),
        (
            !m.tool_offsets_z_only,
            "Confirm tool compensation changes Z only, with no X/Y offsets.",
        ),
    ] {
        if missing {
            issues.push(message.into());
        }
    }
    if let Err(error) = machine.validate_shape() {
        let message = error.to_string();
        if issues.is_empty() || !message.starts_with("POST_M6_CONTRACT") {
            issues.push(message);
        }
    }
    issues
}

/// A new editable configuration, with no invented machine-specific M6 claims.
pub fn new_machine(id: String) -> SequenceProfile {
    use cam_core::post::{Coolant, LengthCompensation, M6Contract, M6Return, PathControl};
    SequenceProfile {
        schema_version: 2,
        id,
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: LengthCompensation::ToolTable,
        path_control: PathControl::ExactPath,
        tools: vec![],
        spindle_spinup_seconds: 1.,
        coolant: Coolant::Off,
        m6: M6Contract {
            reference: String::new(),
            reviewed: false,
            return_position: M6Return::CallerPosition,
            preserves_work_datum: false,
            local_offsets_unused: false,
            tool_offsets_z_only: false,
        },
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    pub id: String,
    pub library: ToolLibrary,
    pub machines: Vec<SequenceProfile>,
}
impl Catalog {
    pub fn empty(id: String) -> Self {
        Self {
            schema: 1,
            id,
            library: ToolLibrary::default(),
            machines: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || !cam_core::preview::valid_id(&self.id) {
            return Err("Unsupported resource schema or invalid library identity".into());
        }
        self.library.validate().map_err(|e| e.to_string())?;
        if self.machines.len() > 100 {
            return Err("At most 100 machine configurations".into());
        }
        let mut ids = BTreeSet::new();
        for machine in &self.machines {
            let problems = machine_problems(machine);
            if !problems.is_empty() {
                return Err(format!("Machine {}: {}", machine.id, problems.join("\n")));
            }
            if !ids.insert(&machine.id) {
                return Err("Duplicate machine configuration ID".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("Resources exceed 8 MB".into());
        }
        Ok(())
    }
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > MAX_BYTES {
            return Err("Resources exceed 8 MB".into());
        }
        let result: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCatalog {
    pub revision: u64,
    pub snapshot: Catalog,
}
impl StoredCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision == 0
            || self.revision > cam_core::tool_library::MAX_LIBRARY_REVISION
            || self.snapshot.library.revision != self.revision
        {
            return Err("Invalid resource revision".into());
        }
        self.snapshot.validate()
    }
    pub fn next(expected: Option<u64>, mut snapshot: Catalog) -> Result<Self, String> {
        let revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Resource revision limit")?;
        snapshot.library.revision = revision;
        let result = Self { revision, snapshot };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ResourceCommand {
    ApplyToolProfile {
        catalog: Catalog,
        tool: String,
        preset: String,
        operation: String,
        role: AssignmentRole,
    },
    StockPage {
        item: v5::ArtworkItemId,
    },
    SelectLibraryTool {
        catalog: Catalog,
        tool: String,
        operation: String,
        role: AssignmentRole,
    },
    AddTool {
        catalog: Catalog,
        tool: String,
    },
    UseTool {
        operation: String,
        role: AssignmentRole,
        tool: String,
    },
    Apply {
        operation: String,
        role: AssignmentRole,
        catalog: Catalog,
        tool: String,
        preset: String,
    },
    Reset {
        operation: String,
        role: AssignmentRole,
    },
    Reapply {
        operation: String,
        role: AssignmentRole,
        catalog: Catalog,
    },
    /// Clear one assignment's copied cutting values and its applied baseline,
    /// keeping the chosen job tool bound.
    Clear {
        operation: String,
        role: AssignmentRole,
    },
    EditTool {
        tool: v5::JobToolV5,
    },
    Machine {
        profile: SequenceProfile,
    },
}
impl ResourceCommand {
    pub fn clear_fields(&self, job: &CamJobV5) -> Vec<usize> {
        // Page capture rewrites the stock rectangle, which belongs to the job
        // rather than to any operation: it clears with an empty operation list.
        if matches!(self, Self::StockPage { .. }) {
            return vec![40, 41, 42, 43];
        }
        if job.operations.is_empty() {
            return vec![];
        }
        if let Some(s) = crate::knife::settings(job) {
            return match self {
                Self::Machine { .. } => vec![7, 32, 33, 34, 35, 36],
                Self::ApplyToolProfile { .. }
                | Self::SelectLibraryTool { .. }
                | Self::UseTool { .. } => vec![61, 62, 63, 64, 65, 66, 32, 33],
                Self::Apply { .. } | Self::Reset { .. } | Self::Reapply { .. } => {
                    vec![63, 64, 65, 66]
                }
                Self::EditTool { tool } if tool.id == s.assignment.tool_id => vec![61, 62],
                _ => vec![],
            };
        }
        if let Self::ApplyToolProfile { role, .. } = self {
            return match role {
                AssignmentRole::Endmill => vec![2, 8, 9, 10, 11, 12, 13, 32, 33],
                AssignmentRole::Vbit => vec![3, 20, 21, 22, 46, 16, 17, 18, 19, 38, 39],
                // Applying a library tool + cutting profile rewrites the
                // operation's milling assignment and its cutter geometry.
                AssignmentRole::Milling => vec![2, 10, 11, 88, 12, 13],
                AssignmentRole::Knife => vec![],
            };
        }
        if let Self::Clear { role, .. } = self {
            return match role {
                AssignmentRole::Endmill => vec![2, 8, 9, 10, 11],
                AssignmentRole::Vbit => vec![3, 20, 21, 22, 46],
                AssignmentRole::Milling => vec![2, 10, 11, 88],
                AssignmentRole::Knife => vec![63, 64, 65, 66],
            };
        }
        if let Self::SelectLibraryTool {
            catalog,
            tool,
            operation,
            role,
        } = self
            && let Ok((added, id)) =
                core::add_library_tool(job, &catalog.library, &catalog.id, tool)
        {
            return Self::UseTool {
                operation: operation.clone(),
                role: *role,
                tool: id,
            }
            .clear_fields(&added.job);
        }
        if let Self::UseTool {
            operation,
            role,
            tool,
        } = self
            && core::assignment_statuses(job)
                .iter()
                .any(|s| &s.operation_id == operation && s.role == *role && &s.tool_id == tool)
        {
            return vec![];
        }
        let role = match self {
            Self::UseTool { role, .. }
            | Self::Apply { role, .. }
            | Self::Reset { role, .. }
            | Self::Reapply { role, .. }
            | Self::Clear { role, .. } => Some(*role),
            _ => None,
        };
        let mut fields = match role {
            Some(AssignmentRole::Endmill) => vec![2, 8, 9, 10, 11],
            Some(AssignmentRole::Vbit) => vec![3, 20, 21, 22, 46],
            // A Profile (or Face) operation's single milling assignment owns
            // the cutting feed, plunge feed, spindle speed and tool stepdown
            // limit; its own stepdown is a separate field and stays.
            Some(AssignmentRole::Milling) => vec![2, 10, 11, 88],
            _ => vec![],
        };
        if matches!(self, Self::UseTool { .. } | Self::EditTool { .. }) {
            // The assignment's tool ids come from the operation that owns the
            // action; a job without a Flat V-carve operation has no V-bit
            // assignment to address here.
            let Some(carving) = crate::session::carving(job) else {
                // A source-free Face operation carries its own cutter geometry
                // and tool stepdown limit; the raw drafts of those values are
                // stale after a resource edit. The same holds for a Profile
                // operation, whose assignment names a job tool of its own.
                if !fields.contains(&88) {
                    fields.push(88);
                }
                fields.extend([12, 13]);
                return fields;
            };
            for (finish, extra) in [
                (false, &[12, 13, 32, 33][..]),
                (true, &[16, 17, 18, 19, 38, 39][..]),
            ] {
                let target = if finish {
                    &carving.vbit.tool_id
                } else {
                    &carving.endmill.tool_id
                };
                let affected = match self {
                    Self::EditTool { tool } => &tool.id == target,
                    Self::UseTool { role, .. } => (*role == AssignmentRole::Vbit) == finish,
                    _ => false,
                };
                if affected {
                    fields.extend(extra);
                }
            }
        }
        if matches!(self, Self::Machine { .. }) {
            fields.extend([7, 32, 33, 34, 35, 36, 38, 39]);
        }
        fields
    }
    pub fn execute(self, job: &CamJobV5) -> Result<CamJobV5, String> {
        let result = match self {
            Self::ApplyToolProfile {
                catalog,
                tool,
                preset,
                operation,
                role,
            } => {
                let selected = Self::SelectLibraryTool {
                    catalog: catalog.clone(),
                    tool: tool.clone(),
                    operation: operation.clone(),
                    role,
                }
                .execute(job)?;
                return Self::Apply {
                    catalog,
                    tool,
                    preset,
                    operation,
                    role,
                }
                .execute(&selected);
            }
            Self::StockPage { item } => {
                let source = job
                    .artwork
                    .iter()
                    .find(|a| a.id == item)
                    .ok_or("Artwork no longer exists")?;
                let mut candidate = job.clone();
                candidate.setup.stock.xy = Some(crate::authoring::svg_page_stock(source)?);
                candidate.validate_structure().map_err(|e| e.to_string())?;
                return Ok(candidate);
            }
            Self::SelectLibraryTool {
                catalog,
                tool,
                operation,
                role,
            } => {
                catalog.validate()?;
                let (added, id) = core::add_library_tool(job, &catalog.library, &catalog.id, &tool)
                    .map_err(|e| e.to_string())?;
                let mut selected = core::use_job_tool(&added.job, &operation, role, &id)
                    .map_err(|e| e.to_string())?
                    .job;
                if let Some(direction) = catalog
                    .library
                    .tool(&tool)
                    .map_err(|e| e.to_string())?
                    .spindle_direction
                {
                    // The library tool's rotation is copied onto exactly the
                    // addressed assignment; the role decides whether that is a
                    // Flat V-carve stage or a Face/Profile milling assignment.
                    selected = core::set_assignment_spindle_direction(
                        &selected,
                        &operation,
                        role,
                        Some(direction),
                    )
                    .map_err(|e| e.to_string())?
                    .job;
                }
                return Ok(selected);
            }
            Self::AddTool { catalog, tool } => {
                catalog.validate()?;
                core::add_library_tool(job, &catalog.library, &catalog.id, &tool).map(|v| v.0)
            }
            Self::UseTool {
                operation,
                role,
                tool,
            } => core::use_job_tool(job, &operation, role, &tool),
            Self::Apply {
                operation,
                role,
                catalog,
                tool,
                preset,
            } => {
                catalog.validate()?;
                core::apply_cutting_profile(
                    job,
                    &operation,
                    role,
                    &catalog.library,
                    &catalog.id,
                    &tool,
                    &preset,
                )
            }
            Self::Reset { operation, role } => core::reset_assignment(job, &operation, role),
            Self::Reapply {
                operation,
                role,
                catalog,
            } => {
                catalog.validate()?;
                let status = core::assignment_statuses(job)
                    .into_iter()
                    .find(|s| s.operation_id == operation && s.role == role)
                    .ok_or("Unknown assignment")?;
                if status
                    .applied
                    .as_ref()
                    .is_none_or(|p| p.library_id != catalog.id)
                {
                    return Err(
                        "The reviewed library is not this assignment's source library".into(),
                    );
                }
                core::reapply_profile(job, &operation, role, &catalog.library, &catalog.id)
            }
            Self::Clear { operation, role } => core::clear_assignment_values(job, &operation, role),
            Self::EditTool { tool } => {
                let mut candidate = job.clone();
                let target = candidate
                    .tools
                    .iter_mut()
                    .find(|t| t.id == tool.id)
                    .ok_or("Unknown job tool")?;
                *target = tool;
                candidate.validate_structure().map_err(|e| e.to_string())?;
                return Ok(candidate);
            }
            Self::Machine { profile } => {
                v5::machine::apply_machine_configuration(job, &profile, &profile.id)
            }
        };
        result.map(|r| r.job).map_err(|e| e.to_string())
    }
}

/// Capture a physical tool independently; saving cutting values is separate.
pub fn capture_tool(tool: &v5::JobToolV5, id: String, name: String) -> Result<LibraryTool, String> {
    use cam_core::project::ToolGeometry;
    let geometry = match tool
        .geometry
        .clone()
        .ok_or("Complete the job tool geometry first")?
    {
        ToolGeometry::Endmill(g) => LibraryGeometry::Endmill(cam_core::model::EndmillSpec {
            diameter_mm: g.diameter_mm,
            cutting_length_mm: g.cutting_length_mm,
            plunge_capable: tool.capabilities.plunge_capable.unwrap_or(false),
        }),
        ToolGeometry::Vbit(g) => LibraryGeometry::Vbit(g),
        ToolGeometry::DragKnife(g) => LibraryGeometry::DragKnife(g),
    };
    let result = LibraryTool {
        spindle_direction: None,
        id,
        name,
        geometry,
        ramp_capable: tool.capabilities.ramp_capable,
        plunge_capable: tool.capabilities.plunge_capable,
        cutting_presets: vec![],
        knife_cutting_presets: vec![],
    };
    result.validate().map_err(|e| e.to_string())?;
    Ok(result)
}
pub fn capture_assignment(
    job: &CamJobV5,
    role: AssignmentRole,
    id: String,
    name: String,
) -> Result<CuttingPreset, String> {
    let s = crate::session::carving(job)
        .ok_or("Choose a milling assignment to capture milling values")?;
    let a = match role {
        AssignmentRole::Endmill => &s.endmill,
        AssignmentRole::Vbit => &s.vbit,
        _ => return Err("Unsupported assignment role".into()),
    };
    let preset = CuttingPreset {
        id,
        name,
        material: None,
        machine: None,
        spindle_rpm: a.spindle_rpm,
        cutting_feed_mm_min: a.cutting_feed_mm_min,
        plunge_feed_mm_min: a.plunge_feed_mm_min,
        max_stepdown_mm: a.max_stepdown_mm,
        stepover_mm: a.stepover_mm,
    };
    preset.validate().map_err(|e| e.to_string())?;
    Ok(preset)
}

/// One assignment's resolved display: the job tool it actually uses, the
/// library copy that tool came from, and the cutting profile applied to it.
///
/// Every place that names the tool of an assignment — the navigator's
/// assigned-tool rows, the job-tool inspector headings and the job tools list
/// — reads this one resolution, so none of them repeat the placeholder name an
/// operation is created with as if it were a chosen cutter.
#[derive(Clone, Debug, PartialEq)]
pub struct AssignedTool {
    pub tool_id: String,
    pub tool_name: String,
    /// `false` while the bound job tool still has no geometry: adding an
    /// operation binds a named placeholder, which is not a chosen cutter.
    pub chosen: bool,
    /// The library copy recorded on the job tool, when it has one.
    pub origin: Option<v5::LibraryOrigin>,
    /// The applied profile's own name at the time it was applied.
    pub profile: Option<String>,
    /// The copied cutting values were edited after that profile was applied.
    pub modified: bool,
}

impl AssignedTool {
    /// The tool one assignment addresses, or `None` when the operation carries
    /// no such assignment.
    pub fn of(job: &CamJobV5, operation_id: &str, role: AssignmentRole) -> Option<Self> {
        let status = core::assignment_statuses(job)
            .into_iter()
            .find(|status| status.operation_id == operation_id && status.role == role)?;
        let tool = job.tools.iter().find(|tool| tool.id == status.tool_id);
        Some(Self {
            tool_id: status.tool_id.clone(),
            tool_name: tool
                .map(|tool| tool.name.clone())
                .unwrap_or_else(|| status.tool_id.clone()),
            chosen: tool.is_some_and(|tool| tool.geometry.is_some()),
            origin: tool.and_then(|tool| tool.library_origin.clone()),
            profile: status
                .applied
                .as_ref()
                .map(|applied| applied.name_at_application.clone()),
            modified: status.status == core::ProfileStatus::Modified,
        })
    }

    /// The chosen tool's name, or an explicit "nothing chosen yet" instead of
    /// the placeholder name.
    pub fn tool_label(&self) -> String {
        if self.chosen {
            self.tool_name.clone()
        } else {
            "no tool chosen".into()
        }
    }

    /// The applied library profile's name, `*` once its copied values were
    /// edited, or `custom` while the values carry no library provenance.
    pub fn profile_label(&self) -> String {
        match (&self.profile, self.modified) {
            (Some(name), true) => format!("{name} *"),
            (Some(name), false) => name.clone(),
            (None, _) => "custom".into(),
        }
    }

    /// The one-line assignment label: "6 mm endmill · rough *", or
    /// "no tool chosen · custom" while the operation is still unconfigured.
    pub fn label(&self) -> String {
        format!("{} · {}", self.tool_label(), self.profile_label())
    }

    /// Where the copied snapshot came from, when the document recorded it.
    /// Provenance is never inferred from a matching name or geometry.
    pub fn origin_label(&self) -> Option<String> {
        let origin = self.origin.as_ref()?;
        let renamed = origin.name_at_copy != self.tool_name;
        Some(format!(
            "copied from library '{}' · tool '{}' r{}{}",
            origin.library_id,
            origin.tool_id,
            origin.copied_revision,
            if renamed { " (renamed here)" } else { "" }
        ))
    }
}

/// The word the workspace uses for one assignment role in listings.
pub fn role_word(role: AssignmentRole) -> &'static str {
    match role {
        AssignmentRole::Endmill => "endmill",
        AssignmentRole::Vbit => "V-bit",
        AssignmentRole::Milling => "cutter",
        AssignmentRole::Knife => "knife",
    }
}

/// Which assignment one job-tool surface addresses, and the word that surface
/// uses for it. One definition keeps the navigator rows, the inspector
/// headings and the tool panel titles in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolTab {
    pub noun: &'static str,
    pub role: AssignmentRole,
}

/// The tool surface an operation kind publishes. `finish` selects the V-bit
/// tab, which only a Flat V-carve operation owns; `None` is that tab for every
/// other kind, where the panel reports that no V-bit is used.
pub fn tool_tab(kind: Option<crate::session::OperationKind>, finish: bool) -> Option<ToolTab> {
    use crate::session::OperationKind;
    Some(match (kind?, finish) {
        (OperationKind::DragKnife, _) => ToolTab {
            noun: "Drag knife",
            role: AssignmentRole::Knife,
        },
        (OperationKind::FlatVcarve, false) => ToolTab {
            noun: "Endmill",
            role: AssignmentRole::Endmill,
        },
        (OperationKind::FlatVcarve, true) => ToolTab {
            noun: "V-bit",
            role: AssignmentRole::Vbit,
        },
        (_, false) => ToolTab {
            noun: "Cutter",
            role: AssignmentRole::Milling,
        },
        (_, true) => return None,
    })
}

/// Editing state is deliberately outside Document and job Undo/recovery.
pub struct Editor {
    pub search: [String; 2],
    pub error: Option<String>,
    pub new_machine_id: String,
    pub machines_view: bool,
    pub picker_loaded: bool,
    /// One selection per assignment role (endmill, V-bit, milling, knife).
    pub picker_tools: [String; 4],
    pub picker_profiles: [String; 4],
    pub open: bool,
    pub jobs_open: bool,
    pub base: Option<StoredCatalog>,
    pub draft: Catalog,
    pub ready: bool,
    pub busy: bool,
    pub dirty: bool,
    pub status: String,
    pub conflict: Option<StoredCatalog>,
    pub raw: std::collections::BTreeMap<String, String>,
    pub invalid: BTreeSet<String>,
    pub tool: String,
    pub preset: String,
    pub machine: String,
    pub job_tool: String,
    pub job_tool_draft: Option<(u64, LibraryTool)>,
    pub job_raw: std::collections::BTreeMap<String, String>,
    pub job_invalid: BTreeSet<String>,
    pub role: AssignmentRole,
}
impl Default for Editor {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        #[cfg(target_arch = "wasm32")]
        let unique = format!(
            "{}-{}",
            js_sys::Date::now() as u64,
            (js_sys::Math::random() * 1e15) as u64
        );
        Self {
            search: Default::default(),
            error: None,
            new_machine_id: "my-machine".into(),
            machines_view: false,
            picker_loaded: false,
            picker_tools: Default::default(),
            picker_profiles: Default::default(),
            open: false,
            jobs_open: false,
            base: None,
            draft: Catalog::empty(format!("library-{unique}")),
            ready: false,
            busy: false,
            dirty: false,
            status: "Load the local library to begin.".into(),
            conflict: None,
            raw: Default::default(),
            invalid: Default::default(),
            tool: String::new(),
            preset: String::new(),
            machine: String::new(),
            job_tool: String::new(),
            job_tool_draft: None,
            job_raw: Default::default(),
            job_invalid: Default::default(),
            role: AssignmentRole::Endmill,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_authoring::{self, Kind};

    fn profile_job() -> CamJobV5 {
        let empty = operation_authoring::empty_job();
        operation_authoring::apply(&empty, operation_authoring::add(Kind::Profile, &empty)).unwrap()
    }

    #[test]
    fn a_fresh_operation_reports_no_chosen_tool_and_custom_values() {
        let job = profile_job();
        let id = job.operations[0].id.clone();
        let tool = AssignedTool::of(&job, &id, AssignmentRole::Milling).unwrap();
        // Adding an operation binds a named placeholder. It is not a cutter
        // anyone chose, so the label never presents it as one.
        assert_eq!(tool.tool_name, "Endmill");
        assert!(!tool.chosen);
        assert_eq!(tool.tool_label(), "no tool chosen");
        assert_eq!(tool.profile_label(), "custom");
        assert_eq!(tool.label(), "no tool chosen · custom");
        assert_eq!(tool.origin_label(), None);
    }

    #[test]
    fn a_library_copy_reports_its_own_name_profile_and_provenance() {
        let catalog = Catalog::decode(include_str!("../../../fixtures/gui5/library.json")).unwrap();
        let job = profile_job();
        let id = job.operations[0].id.clone();
        let job = ResourceCommand::ApplyToolProfile {
            catalog,
            tool: "endmill".into(),
            preset: "rough".into(),
            operation: id.clone(),
            role: AssignmentRole::Milling,
        }
        .execute(&job)
        .unwrap();
        let tool = AssignedTool::of(&job, &id, AssignmentRole::Milling).unwrap();
        assert!(tool.chosen);
        // The copied snapshot keeps the library tool's own name, and the
        // applied profile keeps the preset's own name.
        assert_eq!(tool.tool_label(), "Endmill");
        assert_eq!(tool.profile_label(), "Lettering rough");
        assert_eq!(tool.label(), "Endmill · Lettering rough");
        assert_eq!(
            tool.origin_label().unwrap(),
            "copied from library 'gui5-lettering-library' · tool 'endmill' r1"
        );
        let mut edited = job;
        let v5::OperationSettingsV5::Profile(settings) = &mut edited.operations[0].settings else {
            panic!("the fixture added a Profile operation")
        };
        settings.assignment.cutting_feed_mm_min = Some(999.);
        let tool = AssignedTool::of(&edited, &id, AssignmentRole::Milling).unwrap();
        assert_eq!(tool.profile_label(), "Lettering rough *");
        assert_eq!(tool.label(), "Endmill · Lettering rough *");
    }

    #[test]
    fn tool_tabs_name_the_assignment_each_surface_edits() {
        use crate::session::OperationKind;
        let endmill = |noun, role| Some(ToolTab { noun, role });
        assert_eq!(
            tool_tab(Some(OperationKind::FlatVcarve), false),
            endmill("Endmill", AssignmentRole::Endmill)
        );
        assert_eq!(
            tool_tab(Some(OperationKind::FlatVcarve), true),
            endmill("V-bit", AssignmentRole::Vbit)
        );
        assert_eq!(
            tool_tab(Some(OperationKind::Profile), false),
            endmill("Cutter", AssignmentRole::Milling)
        );
        assert_eq!(
            tool_tab(Some(OperationKind::DragKnife), false),
            endmill("Drag knife", AssignmentRole::Knife)
        );
        // Only a Flat V-carve operation owns a V-bit stage, and a document
        // without an operation owns no tool surface at all.
        assert_eq!(tool_tab(Some(OperationKind::Profile), true), None);
        assert_eq!(tool_tab(None, false), None);
    }
}
