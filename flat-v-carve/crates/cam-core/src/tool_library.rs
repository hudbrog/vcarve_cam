//! Reusable local cutter definitions and explicit cutting presets.
//! Library data has its own schema; jobs always embed independent tool snapshots.
use crate::{
    geometry::{Diagnostic, Result},
    job::{Job, ToolGeometry, ToolSettings},
    preview::valid_id,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const LIBRARY_SCHEMA_VERSION: u32 = 1;
pub const MAX_LIBRARY_BYTES: usize = 8_000_000;
pub const MAX_TOOLS: usize = 1_000;
pub const MAX_PRESETS_PER_TOOL: usize = 100;
/// Revisions can be represented exactly by the browser's JSON number type.
pub const MAX_LIBRARY_REVISION: u64 = 9_007_199_254_740_991;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("tool_library")
}
fn label(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 1_000 || value.chars().any(char::is_control) {
        return Err(error(
            "LIBRARY_LABEL",
            "labels must contain 1–1000 bytes of non-control text",
        ));
    }
    Ok(())
}
fn id(value: &str) -> Result<()> {
    if !valid_id(value) {
        return Err(error(
            "LIBRARY_ID",
            "IDs must be 1–100 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSlot {
    Endmill,
    Vbit,
}
impl ToolSlot {
    pub fn job_id(self, job: &Job) -> &str {
        match self {
            Self::Endmill => &job.operation.endmill_id,
            Self::Vbit => &job.operation.vbit_id,
        }
    }
    fn accepts(self, geometry: &ToolGeometry) -> bool {
        matches!(
            (self, geometry),
            (Self::Endmill, ToolGeometry::Endmill(_)) | (Self::Vbit, ToolGeometry::Vbit(_))
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CuttingPreset {
    pub id: String,
    pub name: String,
    /// User-supplied context, not an automatic feed/speed recommendation.
    pub material: Option<String>,
    pub machine: Option<String>,
    pub spindle_rpm: Option<f64>,
    pub cutting_feed_mm_min: Option<f64>,
    pub plunge_feed_mm_min: Option<f64>,
    pub max_stepdown_mm: Option<f64>,
    pub stepover_mm: Option<f64>,
}
impl CuttingPreset {
    pub fn from_settings(id: String, name: String, settings: &ToolSettings) -> Result<Self> {
        settings.validate()?;
        let preset = Self {
            id,
            name,
            material: None,
            machine: None,
            spindle_rpm: settings.spindle_rpm,
            cutting_feed_mm_min: settings.cutting_feed_mm_min,
            plunge_feed_mm_min: settings.plunge_feed_mm_min,
            max_stepdown_mm: settings.max_stepdown_mm,
            stepover_mm: settings.stepover_mm,
        };
        preset.validate()?;
        Ok(preset)
    }
    fn copy_into(&self, settings: &mut ToolSettings) {
        settings.spindle_rpm = self.spindle_rpm;
        settings.cutting_feed_mm_min = self.cutting_feed_mm_min;
        settings.plunge_feed_mm_min = self.plunge_feed_mm_min;
        settings.max_stepdown_mm = self.max_stepdown_mm;
        settings.stepover_mm = self.stepover_mm;
    }
    pub fn validate(&self) -> Result<()> {
        id(&self.id)?;
        label(&self.name)?;
        for value in [&self.material, &self.machine].into_iter().flatten() {
            label(value)?;
        }
        let mut settings = empty_settings(self.id.clone());
        self.copy_into(&mut settings);
        settings.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryTool {
    pub id: String,
    pub name: String,
    pub geometry: ToolGeometry,
    pub ramp_capable: Option<bool>,
    pub plunge_capable: Option<bool>,
    pub cutting_presets: Vec<CuttingPreset>,
}
fn empty_settings(id: String) -> ToolSettings {
    ToolSettings {
        id,
        geometry: None,
        spindle_rpm: None,
        cutting_feed_mm_min: None,
        plunge_feed_mm_min: None,
        max_stepdown_mm: None,
        stepover_mm: None,
        ramp_capable: None,
        plunge_capable: None,
    }
}
impl LibraryTool {
    /// Capture dimensions/capabilities only. Capturing cutting values is a separate choice.
    pub fn from_settings(id: String, name: String, settings: &ToolSettings) -> Result<Self> {
        settings.validate()?;
        let tool = Self {
            id,
            name,
            geometry: settings.geometry.clone().ok_or_else(|| {
                error(
                    "LIBRARY_GEOMETRY",
                    "a saved tool requires complete geometry",
                )
            })?,
            ramp_capable: settings.ramp_capable,
            plunge_capable: settings.plunge_capable,
            cutting_presets: vec![],
        };
        tool.validate()?;
        Ok(tool)
    }
    fn snapshot(&self, job_id: String, preset: Option<&CuttingPreset>) -> ToolSettings {
        let mut settings = empty_settings(job_id);
        settings.geometry = Some(self.geometry.clone());
        settings.ramp_capable = self.ramp_capable;
        settings.plunge_capable = self.plunge_capable;
        if let Some(preset) = preset {
            preset.copy_into(&mut settings);
        }
        settings
    }
    pub fn validate(&self) -> Result<()> {
        id(&self.id)?;
        label(&self.name)?;
        self.snapshot(self.id.clone(), None).validate()?;
        if self.cutting_presets.len() > MAX_PRESETS_PER_TOOL {
            return Err(error(
                "LIBRARY_RESOURCE_LIMIT",
                "too many cutting presets for one tool",
            ));
        }
        let mut ids = BTreeSet::new();
        for preset in &self.cutting_presets {
            preset.validate()?;
            if !ids.insert(&preset.id) {
                return Err(error(
                    "LIBRARY_DUPLICATE_ID",
                    "cutting preset IDs must be unique within each tool",
                ));
            }
        }
        Ok(())
    }
    pub fn preset(&self, preset_id: &str) -> Result<&CuttingPreset> {
        self.cutting_presets
            .iter()
            .find(|p| p.id == preset_id)
            .ok_or_else(|| {
                error(
                    "LIBRARY_NOT_FOUND",
                    format!(
                        "cutting preset {preset_id:?} not found in tool {:?}",
                        self.id
                    ),
                )
            })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLibrary {
    pub schema_version: u32,
    pub revision: u64,
    pub tools: Vec<LibraryTool>,
}
impl Default for ToolLibrary {
    fn default() -> Self {
        Self {
            schema_version: LIBRARY_SCHEMA_VERSION,
            revision: 0,
            tools: vec![],
        }
    }
}

/// Serializable application operations. Replacements retain the record ID;
/// duplicate operations require a new ID and name. Import is a conflict-rejecting merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LibraryChange {
    AddTool {
        tool: LibraryTool,
    },
    ReplaceTool {
        tool: LibraryTool,
    },
    RemoveTool {
        tool_id: String,
    },
    DuplicateTool {
        tool_id: String,
        new_id: String,
        name: String,
    },
    AddPreset {
        tool_id: String,
        preset: CuttingPreset,
    },
    ReplacePreset {
        tool_id: String,
        preset: CuttingPreset,
    },
    RemovePreset {
        tool_id: String,
        preset_id: String,
    },
    DuplicatePreset {
        tool_id: String,
        preset_id: String,
        new_id: String,
        name: String,
    },
    Import {
        library: ToolLibrary,
    },
}
impl LibraryChange {
    pub fn from_json(json: &str) -> Result<Self> {
        check_size(json)?;
        serde_json::from_str(json).map_err(|e| error("LIBRARY_JSON", e.to_string()))
    }
}
fn check_size(json: &str) -> Result<()> {
    if json.len() > MAX_LIBRARY_BYTES {
        return Err(error(
            "LIBRARY_RESOURCE_LIMIT",
            "library JSON exceeds the 8 MB limit",
        ));
    }
    Ok(())
}
impl ToolLibrary {
    pub fn from_json(json: &str) -> Result<Self> {
        check_size(json)?;
        // Deserialize directly so duplicate JSON fields are rejected too.
        let library: Self =
            serde_json::from_str(json).map_err(|e| error("LIBRARY_JSON", e.to_string()))?;
        library.validate()?;
        Ok(library)
    }
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| error("LIBRARY_JSON", e.to_string()))?
            + "\n";
        check_size(&json)?;
        Ok(json)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != LIBRARY_SCHEMA_VERSION {
            return Err(error(
                "LIBRARY_SCHEMA_VERSION",
                "unsupported library schema version",
            ));
        }
        if self.revision > MAX_LIBRARY_REVISION || self.tools.len() > MAX_TOOLS {
            return Err(error(
                "LIBRARY_RESOURCE_LIMIT",
                "library revision or tool count exceeds its limit",
            ));
        }
        let mut ids = BTreeSet::new();
        for tool in &self.tools {
            tool.validate()?;
            if !ids.insert(&tool.id) {
                return Err(error("LIBRARY_DUPLICATE_ID", "tool IDs must be unique"));
            }
        }
        Ok(())
    }
    pub fn tool(&self, tool_id: &str) -> Result<&LibraryTool> {
        Ok(&self.tools[self.tool_index(tool_id)?])
    }
    fn tool_index(&self, tool_id: &str) -> Result<usize> {
        self.tools
            .iter()
            .position(|t| t.id == tool_id)
            .ok_or_else(|| error("LIBRARY_NOT_FOUND", format!("tool {tool_id:?} not found")))
    }
    pub fn require_revision(&self, expected_revision: u64) -> Result<()> {
        if self.revision != expected_revision {
            return Err(error(
                "LIBRARY_CONFLICT",
                format!(
                    "expected revision {expected_revision}, found {}; reload before editing",
                    self.revision
                ),
            ));
        }
        Ok(())
    }
    /// An immutable transaction: any conflict or invalid result leaves self untouched.
    pub fn changed(&self, expected_revision: u64, change: LibraryChange) -> Result<Self> {
        self.validate()?;
        self.require_revision(expected_revision)?;
        let mut next = self.clone();
        match change {
            LibraryChange::AddTool { tool } => next.tools.push(tool),
            LibraryChange::ReplaceTool { tool } => {
                let index = next.tool_index(&tool.id)?;
                next.tools[index] = tool;
            }
            LibraryChange::RemoveTool { tool_id } => {
                next.tools.remove(next.tool_index(&tool_id)?);
            }
            LibraryChange::DuplicateTool {
                tool_id,
                new_id,
                name,
            } => {
                let mut tool = next.tool(&tool_id)?.clone();
                tool.id = new_id;
                tool.name = name;
                next.tools.push(tool);
            }
            LibraryChange::Import { library } => {
                library.validate()?;
                next.tools.extend(library.tools);
            }
            change => {
                let tool_id = match &change {
                    LibraryChange::AddPreset { tool_id, .. }
                    | LibraryChange::ReplacePreset { tool_id, .. }
                    | LibraryChange::RemovePreset { tool_id, .. }
                    | LibraryChange::DuplicatePreset { tool_id, .. } => tool_id,
                    _ => unreachable!(),
                };
                let index = next.tool_index(tool_id)?;
                let tool = &mut next.tools[index];
                match change {
                    LibraryChange::AddPreset { preset, .. } => tool.cutting_presets.push(preset),
                    LibraryChange::ReplacePreset { preset, .. } => {
                        tool.preset(&preset.id)?;
                        let index = tool
                            .cutting_presets
                            .iter()
                            .position(|p| p.id == preset.id)
                            .unwrap();
                        tool.cutting_presets[index] = preset;
                    }
                    LibraryChange::RemovePreset { preset_id, .. } => {
                        tool.preset(&preset_id)?;
                        tool.cutting_presets.retain(|p| p.id != preset_id);
                    }
                    LibraryChange::DuplicatePreset {
                        preset_id,
                        new_id,
                        name,
                        ..
                    } => {
                        let mut preset = tool.preset(&preset_id)?.clone();
                        preset.id = new_id;
                        preset.name = name;
                        tool.cutting_presets.push(preset);
                    }
                    _ => unreachable!(),
                }
            }
        }
        // validate() enforces the browser-exact upper bound before any save.
        next.revision += 1;
        next.to_json()?;
        Ok(next)
    }
    /// Return a validated candidate job. No live library reference enters the job.
    /// Without a preset ALL cutting values are unset, so another cutter's settings
    /// cannot carry over accidentally. A partial preset copies its nulls as well.
    pub fn apply_to_job(
        &self,
        job: &Job,
        slot: ToolSlot,
        tool_id: &str,
        preset_id: Option<&str>,
    ) -> Result<Job> {
        self.validate()?;
        job.validate_settings()?;
        let tool = self.tool(tool_id)?;
        if !slot.accepts(&tool.geometry) {
            return Err(error(
                "LIBRARY_TOOL_KIND",
                "tool geometry does not match the requested job slot",
            ));
        }
        let preset = preset_id.map(|id| tool.preset(id)).transpose()?;
        let mut candidate = job.clone();
        let job_id = slot.job_id(job);
        let index = candidate
            .tools
            .iter()
            .position(|t| t.id == job_id)
            .ok_or_else(|| error("LIBRARY_NOT_FOUND", "job tool slot not found"))?;
        candidate.tools[index] = tool.snapshot(job_id.to_owned(), preset);
        candidate.validate_settings()?;
        Ok(candidate)
    }
}
