//! Portable editable jobs. Derived geometry is always rebuilt from the embedded SVG.
use crate::{
    geometry::{Diagnostic, Result},
    model::{Endmill, EndmillSpec, VBit, VBitSpec},
    svg::{ImportOptions, NormalizedGeometry, import_svg},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const JOB_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub filename: String,
    pub svg: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "dimensions",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ToolGeometry {
    Endmill(EndmillSpec),
    Vbit(VBitSpec),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSettings {
    pub id: String,
    pub geometry: Option<ToolGeometry>,
    pub spindle_rpm: Option<f64>,
    pub cutting_feed_mm_min: Option<f64>,
    pub plunge_feed_mm_min: Option<f64>,
    pub max_stepdown_mm: Option<f64>,
    pub stepover_mm: Option<f64>,
    #[serde(default)]
    pub ramp_capable: Option<bool>,
    #[serde(default)]
    pub plunge_capable: Option<bool>,
}
impl ToolSettings {
    /// Validate a reusable tool snapshot without requiring a complete job.
    /// Operation-role and stock/depth checks remain the job's responsibility.
    pub fn validate(&self) -> Result<()> {
        if !crate::preview::valid_id(&self.id) {
            return Err(error("JOB_TOOL_ID", "tool IDs must be valid and unique"));
        }
        for (value, name) in [
            (self.spindle_rpm, "spindle_rpm"),
            (self.cutting_feed_mm_min, "cutting_feed_mm_min"),
            (self.plunge_feed_mm_min, "plunge_feed_mm_min"),
            (self.max_stepdown_mm, "max_stepdown_mm"),
            (self.stepover_mm, "stepover_mm"),
        ] {
            number(value, name, false)?;
        }
        if let Some(g) = &self.geometry {
            match g {
                ToolGeometry::Endmill(s) => {
                    if self.plunge_capable.is_some_and(|v| v != s.plunge_capable) {
                        return Err(error(
                            "JOB_TOOL_CAPABILITY",
                            "endmill slot and dimensions disagree about plunge capability",
                        ));
                    }
                    Endmill::try_from(s.clone())?;
                }
                ToolGeometry::Vbit(s) => {
                    VBit::try_from(s.clone())?;
                }
            }
        }
        Ok(())
    }

    fn empty(id: &str) -> Self {
        Self {
            id: id.into(),
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
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockSettings {
    pub thickness_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationSettings {
    pub id: String,
    pub endmill_id: String,
    pub vbit_id: String,
    pub max_depth_mm: Option<f64>,
    pub wall_allowance_mm: Option<f64>,
    pub max_floor_ridge_mm: Option<f64>,
    pub max_detail_residual_mm: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTolerances {
    pub motion_tolerance_mm: Option<f64>,
    pub verification_tolerance_mm: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineProfile {
    pub id: String,
    pub work_offset: Option<String>,
    pub clearance_z_mm: Option<f64>,
    pub endmill_tool_number: Option<u32>,
    pub vbit_tool_number: Option<u32>,
    /// Editable description; implementing/validating its behavior belongs to M6.
    pub m6_contract: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub schema_version: u32,
    pub name: String,
    pub source: SourceSnapshot,
    pub import: ImportOptions,
    pub selected_region_ids: Vec<String>,
    pub stock: StockSettings,
    pub operation: OperationSettings,
    pub tools: Vec<ToolSettings>,
    pub tolerances: PlanningTolerances,
    pub machine_profile: Option<MachineProfile>,
    #[serde(default)]
    pub endmill_planning: Option<crate::pocket::EndmillPlanningSettings>,
    #[serde(default)]
    pub vbit_planning: Option<crate::vcarve::VBitPlanningSettings>,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobInspection {
    pub schema_version: u32,
    pub engine_version: String,
    pub name: String,
    pub geometry: NormalizedGeometry,
    /// Missing settings are expected during editing; import only supplies documented defaults.
    pub missing_machining_fields: Vec<String>,
    pub planning_available: bool,
}

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("job")
}
fn number(value: Option<f64>, name: &str, zero: bool) -> Result<()> {
    if value.is_some_and(|v| !v.is_finite() || if zero { v < 0. } else { v <= 0. }) {
        return Err(error(
            "JOB_PARAMETER",
            format!(
                "{name} must be finite and {}",
                if zero { "nonnegative" } else { "positive" }
            ),
        ));
    }
    Ok(())
}
impl Job {
    pub fn from_svg(filename: String, svg: String, options: ImportOptions) -> Result<Self> {
        let geometry = import_svg(&svg, &options, None)?;
        Ok(Self {
            schema_version: JOB_SCHEMA_VERSION,
            name: filename.clone(),
            source: SourceSnapshot { filename, svg },
            import: options,
            selected_region_ids: geometry.selected_region_ids,
            stock: StockSettings::default(),
            operation: OperationSettings {
                id: "flat-v-carve".into(),
                endmill_id: "endmill".into(),
                vbit_id: "vbit".into(),
                max_depth_mm: None,
                wall_allowance_mm: Some(0.),
                max_floor_ridge_mm: None,
                max_detail_residual_mm: None,
            },
            tools: vec![ToolSettings::empty("endmill"), ToolSettings::empty("vbit")],
            tolerances: PlanningTolerances::default(),
            machine_profile: None,
            endmill_planning: None,
            vbit_planning: None,
        })
    }
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 64_000_000 {
            return Err(error(
                "JOB_RESOURCE_LIMIT",
                "job exceeds the 64 MB input limit",
            ));
        }
        let mut value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| error("JOB_JSON", e.to_string()))?;
        if matches!(
            value.get("schema_version").and_then(|v| v.as_u64()),
            Some(1 | 2)
        ) {
            value["schema_version"] = serde_json::json!(JOB_SCHEMA_VERSION);
        }
        if value.get("schema_version").and_then(|v| v.as_u64()) != Some(JOB_SCHEMA_VERSION as u64) {
            return Err(error(
                "JOB_SCHEMA_VERSION",
                "unsupported or missing job schema version",
            ));
        }
        let job: Self =
            serde_json::from_value(value).map_err(|e| error("JOB_JSON", e.to_string()))?;
        job.validate_settings()?;
        Ok(job)
    }
    pub fn to_json(&self) -> Result<String> {
        self.validate_settings()?;
        serde_json::to_string_pretty(self)
            .map(|s| s + "\n")
            .map_err(|e| error("JOB_JSON", e.to_string()))
    }
    pub fn validate_settings(&self) -> Result<()> {
        if let Some(settings) = &self.vbit_planning {
            settings.validate()?;
        }
        if let Some(settings) = &self.endmill_planning {
            settings.validate()?;
        }
        if self.schema_version != JOB_SCHEMA_VERSION {
            return Err(error(
                "JOB_SCHEMA_VERSION",
                "unsupported job schema version",
            ));
        }
        if self.name.trim().is_empty()
            || self.name.len() > 1000
            || self.source.filename.len() > 1000
        {
            return Err(error(
                "JOB_NAME",
                "job/source names must be short and the job name nonempty",
            ));
        }
        if self.tools.len() != 2 {
            return Err(error(
                "JOB_TOOLS",
                "MVP jobs contain one endmill slot and one V-bit slot",
            ));
        }
        let mut ids = BTreeSet::new();
        for tool in &self.tools {
            if !crate::preview::valid_id(&tool.id) || !ids.insert(&tool.id) {
                return Err(error("JOB_TOOL_ID", "tool IDs must be valid and unique"));
            }
            tool.validate()?;
        }
        if !crate::preview::valid_id(&self.operation.id)
            || self.operation.endmill_id == self.operation.vbit_id
            || !ids.contains(&self.operation.endmill_id)
            || !ids.contains(&self.operation.vbit_id)
        {
            return Err(error(
                "JOB_OPERATION",
                "operation must reference distinct existing endmill and V-bit IDs",
            ));
        }
        for tool in &self.tools {
            if matches!(&tool.geometry, Some(ToolGeometry::Vbit(_)))
                && tool.id == self.operation.endmill_id
                || matches!(&tool.geometry, Some(ToolGeometry::Endmill(_)))
                    && tool.id == self.operation.vbit_id
            {
                return Err(error(
                    "JOB_TOOL_KIND",
                    "tool geometry does not match its operation role",
                ));
            }
        }
        for (v, n, z) in [
            (self.stock.thickness_mm, "stock.thickness_mm", false),
            (self.operation.max_depth_mm, "operation.max_depth_mm", false),
            (
                self.operation.wall_allowance_mm,
                "operation.wall_allowance_mm",
                true,
            ),
            (
                self.operation.max_floor_ridge_mm,
                "operation.max_floor_ridge_mm",
                true,
            ),
            (
                self.operation.max_detail_residual_mm,
                "operation.max_detail_residual_mm",
                true,
            ),
            (
                self.tolerances.motion_tolerance_mm,
                "motion_tolerance_mm",
                false,
            ),
            (
                self.tolerances.verification_tolerance_mm,
                "verification_tolerance_mm",
                false,
            ),
        ] {
            number(v, n, z)?;
        }
        if let Some(d) = self.operation.max_depth_mm {
            if self.stock.thickness_mm.is_some_and(|h| d > h) {
                return Err(error(
                    "JOB_STOCK_DEPTH",
                    "carve depth exceeds stock thickness",
                ));
            }
            for tool in &self.tools {
                match &tool.geometry {
                    Some(ToolGeometry::Endmill(s)) => Endmill::try_from(s.clone())?
                        .validate_depth(crate::model::Depth::new(d)?)?,
                    Some(ToolGeometry::Vbit(s)) => {
                        VBit::try_from(s.clone())?.validate_depth(crate::model::Depth::new(d)?)?
                    }
                    _ => {}
                }
            }
        }
        if let Some(m) = &self.machine_profile {
            if !crate::preview::valid_id(&m.id) {
                return Err(error("JOB_MACHINE", "invalid machine profile ID"));
            }
            number(m.clearance_z_mm, "clearance_z_mm", false)?;
            if m.endmill_tool_number == Some(0) || m.vbit_tool_number == Some(0) {
                return Err(error("JOB_MACHINE", "tool numbers must be positive"));
            }
            if m.work_offset.as_ref().is_some_and(|s| {
                !matches!(
                    s.as_str(),
                    "G54" | "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3"
                )
            }) {
                return Err(error("JOB_MACHINE", "unsupported work offset"));
            }
        }
        Ok(())
    }
    pub fn inspect(&self) -> Result<JobInspection> {
        self.validate_settings()?;
        let geometry = import_svg(
            &self.source.svg,
            &self.import,
            Some(&self.selected_region_ids),
        )?;
        let mut missing = vec![];
        if self.vbit_planning.is_none() {
            missing.push("vbit_planning".into());
        }
        if self.endmill_planning.is_none() {
            missing.push("endmill_planning".into());
        }
        if self.selected_region_ids.is_empty() {
            missing.push("selected_region_ids".into());
        }
        for (v, n) in [
            (self.stock.thickness_mm, "stock.thickness_mm"),
            (self.operation.max_depth_mm, "operation.max_depth_mm"),
            (
                self.operation.wall_allowance_mm,
                "operation.wall_allowance_mm",
            ),
            (
                self.operation.max_floor_ridge_mm,
                "operation.max_floor_ridge_mm",
            ),
            (
                self.operation.max_detail_residual_mm,
                "operation.max_detail_residual_mm",
            ),
            (
                self.tolerances.motion_tolerance_mm,
                "tolerances.motion_tolerance_mm",
            ),
            (
                self.tolerances.verification_tolerance_mm,
                "tolerances.verification_tolerance_mm",
            ),
        ] {
            if v.is_none() {
                missing.push(n.into());
            }
        }
        for t in &self.tools {
            if t.id == self.operation.vbit_id && t.plunge_capable.is_none() {
                missing.push(format!("tools.{}.plunge_capable", t.id));
            }
            if t.geometry.is_none() {
                missing.push(format!("tools.{}.geometry", t.id));
            }
            for (v, n) in [
                (t.spindle_rpm, "spindle_rpm"),
                (t.cutting_feed_mm_min, "cutting_feed_mm_min"),
                (t.plunge_feed_mm_min, "plunge_feed_mm_min"),
                (t.max_stepdown_mm, "max_stepdown_mm"),
                (t.stepover_mm, "stepover_mm"),
            ] {
                if v.is_none() {
                    missing.push(format!("tools.{}.{n}", t.id));
                }
            }
        }
        Ok(JobInspection {
            schema_version: JOB_SCHEMA_VERSION,
            engine_version: env!("CARGO_PKG_VERSION").into(),
            name: self.name.clone(),
            geometry,
            missing_machining_fields: missing,
            planning_available: true,
        })
    }
}
