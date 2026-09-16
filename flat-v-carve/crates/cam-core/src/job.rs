//! The V-carve engine's planning input.
//!
//! [`VcarveInput`] is the engine's one front door, and it is not a document:
//! no schema version, no reader, no user command. The only job document is
//! schema 5 ([`crate::project::v5`]); the input is fused from the shared
//! planner context, the operation's settings and the **resolved selected
//! region** ([`crate::operations::flat_vcarve::vcarve_input`]), so the region
//! is the geometry authority and no source is imported or re-imported
//! anywhere below this type.
//!
//! Two hidden helpers serve the engine's own regression corpus and the
//! benchmark examples that read a saved plan: [`FixtureJob`] is the fixture
//! form (`fixtures/m3`, `fixtures/m4`, which still name an SVG and the region
//! ids selected from it) and [`input_from_json`] reads the plan's embedded
//! input back.
use crate::{
    geometry::{Diagnostic, Grid, GridPoint, Region, Result},
    model::{Endmill, EndmillSpec, VBit, VBitSpec},
    svg::{ImportOptions, import_svg},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTolerances {
    pub motion_tolerance_mm: Option<f64>,
    pub verification_tolerance_mm: Option<f64>,
    /// Arc/line fitting of milling motion streams, in mm: the largest
    /// deviation the fitted program may have from the polyline the planners
    /// resolved (plan section 8.4). Unset leaves the stream exactly as the
    /// planners emitted it; `0` is the same as unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arc_fit_tolerance_mm: Option<f64>,
}

/// Everything one Flat V-carve planning run needs: the operation's settings,
/// the two cutter slots with their process values, the stock, the tolerances,
/// the rough/finish limits — and the resolved selected region the target is
/// built from.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "VcarveInputWire", into = "VcarveInputWire")]
pub struct VcarveInput {
    /// The union of the selected filled components in workpiece XY, on its own
    /// snapping grid. Carried whole, so a saved plan can rebuild the exact
    /// target it was planned against.
    pub region: Region,
    /// Import error carried with that region, in workpiece XY millimetres: the
    /// flattening bound plus the source-snapping bound. The region's grid adds
    /// its own snapping term, so the verification's source depth error is
    /// `(source_error_mm + region.grid().snap_bound_mm()) / slope`.
    pub source_error_mm: f64,
    pub stock: StockSettings,
    pub operation: OperationSettings,
    pub tools: Vec<ToolSettings>,
    pub tolerances: PlanningTolerances,
    pub endmill_planning: Option<crate::pocket::EndmillPlanningSettings>,
    pub vbit_planning: Option<crate::vcarve::VBitPlanningSettings>,
}

/// The serialized form of [`VcarveInput`]: the same input with the region
/// written as its snapping grid and grid coordinates, which rebuild through
/// [`Region::from_grid_rings`] instead of being trusted.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VcarveInputWire {
    region: RegionWire,
    source_error_mm: f64,
    stock: StockSettings,
    operation: OperationSettings,
    tools: Vec<ToolSettings>,
    tolerances: PlanningTolerances,
    #[serde(default)]
    endmill_planning: Option<crate::pocket::EndmillPlanningSettings>,
    #[serde(default)]
    vbit_planning: Option<crate::vcarve::VBitPlanningSettings>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionWire {
    ticks_per_mm: f64,
    geometry_tolerance_mm: f64,
    /// Rings of grid coordinates, in stored order.
    rings: Vec<Vec<[i64; 2]>>,
}

impl RegionWire {
    fn of(region: &Region) -> Self {
        Self {
            ticks_per_mm: region.grid().scale(),
            geometry_tolerance_mm: region.grid().tolerance_mm(),
            rings: region
                .rings()
                .iter()
                .map(|ring| ring.points().iter().map(|p| [p.x, p.y]).collect())
                .collect(),
        }
    }

    fn to_region(&self) -> Result<Region> {
        let rings: Vec<Vec<GridPoint>> = self
            .rings
            .iter()
            .map(|ring| ring.iter().map(|&[x, y]| GridPoint { x, y }).collect())
            .collect();
        let max_abs_mm = rings
            .iter()
            .flatten()
            .map(|p| (p.x.abs().max(p.y.abs())) as f64 / self.ticks_per_mm)
            .fold(0., f64::max);
        let grid = Grid::with_scale(self.geometry_tolerance_mm, max_abs_mm, self.ticks_per_mm)?;
        Region::from_grid_rings(grid, &rings)
    }
}

impl TryFrom<VcarveInputWire> for VcarveInput {
    type Error = Diagnostic;
    fn try_from(wire: VcarveInputWire) -> Result<Self> {
        Ok(Self {
            region: wire.region.to_region()?,
            source_error_mm: wire.source_error_mm,
            stock: wire.stock,
            operation: wire.operation,
            tools: wire.tools,
            tolerances: wire.tolerances,
            endmill_planning: wire.endmill_planning,
            vbit_planning: wire.vbit_planning,
        })
    }
}

impl From<VcarveInput> for VcarveInputWire {
    fn from(input: VcarveInput) -> Self {
        Self {
            region: RegionWire::of(&input.region),
            source_error_mm: input.source_error_mm,
            stock: input.stock,
            operation: input.operation,
            tools: input.tools,
            tolerances: input.tolerances,
            endmill_planning: input.endmill_planning,
            vbit_planning: input.vbit_planning,
        }
    }
}

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("job")
}

/// The engine's regression-fixture form: an attached source snapshot, the
/// import options that produced the geometry from it, and the region ids
/// selected out of that import, plus the planning settings.
///
/// Test data only. It exists so `fixtures/m3` and `fixtures/m4` stay readable
/// as "this SVG, this selection, these settings", and so the geometry they
/// describe still arrives through the one SVG importer. No command reads this
/// shape.
#[doc(hidden)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureJob {
    pub name: String,
    pub source: SourceSnapshot,
    pub import: ImportOptions,
    pub selected_region_ids: Vec<String>,
    pub stock: StockSettings,
    pub operation: OperationSettings,
    pub tools: Vec<ToolSettings>,
    pub tolerances: PlanningTolerances,
    #[serde(default)]
    pub endmill_planning: Option<crate::pocket::EndmillPlanningSettings>,
    #[serde(default)]
    pub vbit_planning: Option<crate::vcarve::VBitPlanningSettings>,
}
impl FixtureJob {
    #[doc(hidden)]
    pub fn parse(json: &str) -> Result<Self> {
        if json.len() > 64_000_000 {
            return Err(error(
                "JOB_RESOURCE_LIMIT",
                "job exceeds the 64 MB input limit",
            ));
        }
        serde_json::from_str(json).map_err(|e| error("JOB_JSON", e.to_string()))
    }

    /// Resolve the fixture's selection into the engine's planning input.
    #[doc(hidden)]
    pub fn resolve(&self) -> Result<VcarveInput> {
        let geometry = import_svg(
            &self.source.svg,
            &self.import,
            Some(&self.selected_region_ids),
        )?;
        Ok(VcarveInput {
            region: geometry.selected,
            source_error_mm: geometry.flattening_bound_mm + geometry.source_snap_bound_mm,
            stock: self.stock.clone(),
            operation: self.operation.clone(),
            tools: self.tools.clone(),
            tolerances: self.tolerances.clone(),
            endmill_planning: self.endmill_planning.clone(),
            vbit_planning: self.vbit_planning.clone(),
        })
    }
}

/// Resolve one engine fixture into the engine's planning input.
#[doc(hidden)]
pub fn input_from_fixture_json(json: &str) -> Result<VcarveInput> {
    FixtureJob::parse(json)?.resolve()
}

/// Read the input a saved plan embeds. Test and benchmark support: the plan's
/// own envelope is the only writer of this form.
#[doc(hidden)]
pub fn input_from_json(json: &str) -> Result<VcarveInput> {
    if json.len() > 64_000_000 {
        return Err(error(
            "JOB_RESOURCE_LIMIT",
            "job exceeds the 64 MB input limit",
        ));
    }
    let input: VcarveInput =
        serde_json::from_str(json).map_err(|e| error("JOB_JSON", e.to_string()))?;
    input.validate_settings()?;
    Ok(input)
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
impl VcarveInput {
    pub fn validate_settings(&self) -> Result<()> {
        if let Some(settings) = &self.vbit_planning {
            settings.validate()?;
        }
        if let Some(settings) = &self.endmill_planning {
            settings.validate()?;
        }
        if !self.source_error_mm.is_finite() || self.source_error_mm < 0. {
            return Err(error(
                "JOB_PARAMETER",
                "source_error_mm must be finite and nonnegative",
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
        Ok(())
    }
}
