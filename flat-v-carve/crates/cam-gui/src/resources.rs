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
    EditTool {
        tool: v5::JobToolV5,
    },
    Machine {
        profile: SequenceProfile,
    },
}
impl ResourceCommand {
    pub fn clear_fields(&self, job: &CamJobV5) -> Vec<usize> {
        if let Self::ApplyToolProfile { role, .. } = self {
            return match role {
                AssignmentRole::Endmill => vec![2, 8, 9, 10, 11, 12, 13, 32, 33],
                AssignmentRole::Vbit => vec![3, 20, 21, 22, 46, 16, 17, 18, 19, 38, 39],
                _ => vec![],
            };
        }
        if matches!(self, Self::StockPage { .. }) {
            return vec![40, 41, 42, 43];
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
            | Self::Reapply { role, .. } => Some(*role),
            _ => None,
        };
        let mut fields = match role {
            Some(AssignmentRole::Endmill) => vec![2, 8, 9, 10, 11],
            Some(AssignmentRole::Vbit) => vec![3, 20, 21, 22, 46],
            _ => vec![],
        };
        if matches!(self, Self::UseTool { .. } | Self::EditTool { .. }) {
            for (finish, extra) in [
                (false, &[12, 13, 32, 33][..]),
                (true, &[16, 17, 18, 19, 38, 39][..]),
            ] {
                let s = crate::session::settings(job);
                let target = if finish {
                    &s.vbit.tool_id
                } else {
                    &s.endmill.tool_id
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
                    let settings = crate::authoring::settings_mut(&mut selected);
                    match role {
                        AssignmentRole::Endmill => {
                            settings.endmill.spindle_direction = Some(direction)
                        }
                        AssignmentRole::Vbit => settings.vbit.spindle_direction = Some(direction),
                        _ => return Err("Unsupported milling assignment".into()),
                    }
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
    let s = crate::session::settings(job);
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

/// Editing state is deliberately outside Document and job Undo/recovery.
pub struct Editor {
    pub search: [String; 2],
    pub error: Option<String>,
    pub new_machine_id: String,
    pub machines_view: bool,
    pub picker_loaded: bool,
    pub picker_tools: [String; 2],
    pub picker_profiles: [String; 2],
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
