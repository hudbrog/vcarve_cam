//! Canonical schema-4 CAM job: editable setup, tools and an ordered operation list.
//!
//! This is the frozen schema-4 migration DTO: the schema-5 collection model
//! ([`v5`]) migrates through `CamJob::from_json`, and later slices must not
//! change this serialized shape without a new named compatibility step. The
//! legacy schema-3 [`crate::job::Job`] remains the model used by the existing
//! V-carve engine; import it as `LegacyJob` when adapting, and never construct
//! a fake legacy job for non-V-carve planners.
use crate::{
    geometry::{Diagnostic, Point, Result},
    job::{MachineProfile, PlanningTolerances, SourceSnapshot},
    model::{VBit, VBitSpec},
    pocket::{ClearingStrategy, EntryStrategy},
    preview,
    svg::ImportOptions,
    vcarve::VBitPlanningSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod migrate;
pub mod v5;

pub const CAM_JOB_SCHEMA_VERSION: u32 = 4;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("project")
}

fn number(value: Option<f64>, name: &str, positive: bool) -> Result<()> {
    if value.is_some_and(|v| !v.is_finite() || if positive { v <= 0. } else { v < 0. }) {
        return Err(error(
            "PROJECT_PARAMETER",
            format!(
                "{name} must be finite and {}",
                if positive { "positive" } else { "nonnegative" }
            ),
        ));
    }
    Ok(())
}

/// Axis-aligned setup-space rectangle with an arbitrary minimum corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectXY {
    pub min_x_mm: f64,
    pub min_y_mm: f64,
    pub width_mm: f64,
    pub length_mm: f64,
}
impl RectXY {
    fn validate(&self) -> Result<()> {
        let finite = [self.min_x_mm, self.min_y_mm, self.width_mm, self.length_mm]
            .iter()
            .all(|v| v.is_finite());
        if !finite || self.width_mm <= 0. || self.length_mm <= 0. {
            return Err(error(
                "PROJECT_PARAMETER",
                "rectangles require finite extents and positive width/length",
            ));
        }
        Ok(())
    }
}

/// Stock anchor fraction restricted to min/center/max on each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub enum AnchorFraction {
    Min,
    Center,
    Max,
}
impl AnchorFraction {
    pub fn fraction(self) -> f64 {
        match self {
            Self::Min => 0.,
            Self::Center => 0.5,
            Self::Max => 1.,
        }
    }
}
impl TryFrom<f64> for AnchorFraction {
    type Error = Diagnostic;
    fn try_from(value: f64) -> Result<Self> {
        match value {
            0. => Ok(Self::Min),
            0.5 => Ok(Self::Center),
            1. => Ok(Self::Max),
            _ => Err(error(
                "PROJECT_PARAMETER",
                "stock anchor fractions must be 0, 0.5, or 1",
            )),
        }
    }
}
impl From<AnchorFraction> for f64 {
    fn from(value: AnchorFraction) -> Self {
        value.fraction()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockSetup {
    pub thickness_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xy: Option<RectXY>,
}
impl StockSetup {
    fn validate(&self) -> Result<()> {
        number(self.thickness_mm, "setup.stock.thickness_mm", true)?;
        if let Some(xy) = &self.xy {
            xy.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkZeroXY {
    /// XY=(0,0) in setup coordinates, valid even when stock XY is unset.
    #[default]
    SetupOrigin,
    StockAnchor {
        x_fraction: AnchorFraction,
        y_fraction: AnchorFraction,
    },
    CustomPoint {
        x_mm: f64,
        y_mm: f64,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkZeroZ {
    #[default]
    StockTop,
    StockBottom,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkZero {
    #[serde(default)]
    pub xy: WorkZeroXY,
    #[serde(default)]
    pub z: WorkZeroZ,
}
impl WorkZero {
    fn validate(&self) -> Result<()> {
        if let WorkZeroXY::CustomPoint { x_mm, y_mm } = &self.xy
            && (!x_mm.is_finite() || !y_mm.is_finite())
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "custom work-zero points must be finite",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SetupSettings {
    pub stock: StockSetup,
    #[serde(default)]
    pub work_zero: WorkZero,
    /// Height above original stock top; the setup-space clearance plane is
    /// `+clearance_above_stock_mm` regardless of the selected Z datum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance_above_stock_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_xy_mm: Option<Point>,
}
impl SetupSettings {
    fn validate(&self) -> Result<()> {
        self.stock.validate()?;
        self.work_zero.validate()?;
        number(
            self.clearance_above_stock_mm,
            "setup.clearance_above_stock_mm",
            true,
        )?;
        if self.start_xy_mm.is_some_and(|p| !p.finite()) {
            return Err(error(
                "PROJECT_PARAMETER",
                "setup.start_xy_mm must be finite",
            ));
        }
        Ok(())
    }
}

/// Pure cutter geometry; entry capabilities live in [`ToolCapabilities`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndmillGeometry {
    pub diameter_mm: f64,
    pub cutting_length_mm: f64,
}
impl EndmillGeometry {
    fn validate(&self) -> Result<()> {
        if !self.diameter_mm.is_finite()
            || self.diameter_mm <= 0.
            || !self.cutting_length_mm.is_finite()
            || self.cutting_length_mm <= 0.
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "endmill diameter and cutting length must be finite and positive",
            ));
        }
        Ok(())
    }
}

/// Passive drag-knife geometry. `blade_offset_mm` is the pivot-to-tip distance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DragKnifeSpec {
    pub blade_offset_mm: f64,
    pub max_cut_depth_mm: f64,
}
impl DragKnifeSpec {
    pub fn validate(&self) -> Result<()> {
        if !self.blade_offset_mm.is_finite()
            || self.blade_offset_mm <= 0.
            || !self.max_cut_depth_mm.is_finite()
            || self.max_cut_depth_mm <= 0.
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "knife blade offset and max cut depth must be finite and positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "dimensions",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ToolGeometry {
    Endmill(EndmillGeometry),
    Vbit(VBitSpec),
    DragKnife(DragKnifeSpec),
}
impl ToolGeometry {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Endmill(g) => g.validate(),
            Self::Vbit(spec) => VBit::try_from(spec.clone()).map(|_| ()),
            Self::DragKnife(spec) => spec.validate(),
        }
    }
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Endmill(_) => "endmill",
            Self::Vbit(_) => "vbit",
            Self::DragKnife(_) => "drag_knife",
        }
    }
}

/// Entry capabilities. Plunge capability is stored exactly once: here, not in
/// the geometry. The legacy duplicated endmill representation is collapsed by
/// migration after the legacy validator has rejected conflicting values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ToolCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_capable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp_capable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobTool {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub geometry: Option<ToolGeometry>,
    #[serde(default)]
    pub capabilities: ToolCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpindleDirection {
    Clockwise,
    Counterclockwise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MillingAssignment {
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spindle_rpm: Option<f64>,
    /// Unset for migrated legacy jobs until a machine profile is applied;
    /// required before new Face/Profile planning and before process export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spindle_direction: Option<SpindleDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutting_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stepdown_mm: Option<f64>,
    /// Required only where the operation uses a stepover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_mm: Option<f64>,
}
impl MillingAssignment {
    fn validate(&self, prefix: &str) -> Result<()> {
        for (value, name) in [
            (self.spindle_rpm, "spindle_rpm"),
            (self.cutting_feed_mm_min, "cutting_feed_mm_min"),
            (self.plunge_feed_mm_min, "plunge_feed_mm_min"),
            (self.max_stepdown_mm, "max_stepdown_mm"),
            (self.stepover_mm, "stepover_mm"),
        ] {
            number(value, &format!("{prefix}.{name}"), true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnifeAssignment {
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutting_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swivel_feed_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_stepdown_mm: Option<f64>,
}
impl KnifeAssignment {
    fn validate(&self, prefix: &str) -> Result<()> {
        for (value, name) in [
            (self.cutting_feed_mm_min, "cutting_feed_mm_min"),
            (self.plunge_feed_mm_min, "plunge_feed_mm_min"),
            (self.swivel_feed_mm_min, "swivel_feed_mm_min"),
            (self.max_stepdown_mm, "max_stepdown_mm"),
        ] {
            number(value, &format!("{prefix}.{name}"), true)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HeightReference {
    #[default]
    StockTop,
    StockBottom,
    /// That same operation's resolved top; legal only for bottom heights.
    OperationTop,
    /// A plane published by a preceding face operation.
    FaceResult {
        operation_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HeightRef {
    pub reference: HeightReference,
    /// Signed offset from the reference, positive upward.
    #[serde(default)]
    pub offset_mm: f64,
}
impl HeightRef {
    fn validate(&self, field: &str, is_bottom: bool) -> Result<()> {
        if !self.offset_mm.is_finite() {
            return Err(error(
                "PROJECT_PARAMETER",
                format!("{field}.offset_mm must be finite"),
            ));
        }
        if !is_bottom && matches!(self.reference, HeightReference::OperationTop) {
            return Err(error(
                "HEIGHT_REFERENCE_INVALID",
                format!("{field} may not use operation_top; it is legal only for bottoms"),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlatVcarveMode {
    /// Rough endmill clearing only; the V-bit stage is not executed.
    EndmillOnly,
    /// Rough endmill clearing followed by V-bit finishing.
    Combined,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatVcarveRoughSettings {
    pub strategy: ClearingStrategy,
    pub entry: EntryStrategy,
    pub max_layers: usize,
    pub max_loops_per_layer: usize,
    pub max_motions: usize,
}
impl Default for FlatVcarveRoughSettings {
    fn default() -> Self {
        Self {
            strategy: ClearingStrategy::DepthDependent,
            entry: EntryStrategy::Plunge,
            max_layers: 32,
            max_loops_per_layer: 256,
            max_motions: 20_000,
        }
    }
}
impl FlatVcarveRoughSettings {
    fn validate(&self) -> Result<()> {
        if !(1..=256).contains(&self.max_layers)
            || !(1..=1024).contains(&self.max_loops_per_layer)
            || !(1..=100_000).contains(&self.max_motions)
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "rough-stage limits must be 1..256 layers, 1..1024 loops/layer, and 1..100000 motions",
            ));
        }
        if let EntryStrategy::Ramp {
            max_angle_deg,
            feed_mm_min,
        } = &self.entry
            && (!max_angle_deg.is_finite()
                || max_angle_deg <= &0.
                || max_angle_deg >= &90.
                || !feed_mm_min.is_finite()
                || feed_mm_min <= &0.)
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "ramp angle must be between 0 and 90 degrees, with an explicit positive feed",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatVcarveSettings {
    /// Selected filled components; may be empty while the job is incomplete.
    pub component_ids: Vec<String>,
    pub mode: FlatVcarveMode,
    pub endmill: MillingAssignment,
    /// Kept even in endmill-only mode: its geometry defines the nominal
    /// V-shaped target the rough stage clears toward.
    pub vbit: MillingAssignment,
    /// Where carving starts: the original stock top by default, or a plane
    /// published by a preceding face operation. Depth is measured below it.
    #[serde(default)]
    pub top: HeightRef,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_allowance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_floor_ridge_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_detail_residual_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rough: Option<FlatVcarveRoughSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<VBitPlanningSettings>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacePattern {
    #[default]
    ZigZag,
    OneWay,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaceArea {
    EntireStock,
    Rectangle { rect: RectXY },
}

/// Per-side coverage expansion of the requested face area, all nonnegative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FaceMargins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_x_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_x_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_y_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_y_mm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceSettings {
    pub area: FaceArea,
    #[serde(default)]
    pub margins: FaceMargins,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_overrun_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_overrun_mm: Option<f64>,
    pub top: HeightRef,
    pub bottom: HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_angle_deg: Option<f64>,
    #[serde(default)]
    pub pattern: FacePattern,
    pub assignment: MillingAssignment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContourSide {
    Inside,
    Outside,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraversalDirection {
    Forward,
    Reverse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileContour {
    pub contour_id: String,
    pub side: ContourSide,
    /// Required for on-contour selections, which have no retained side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traversal: Option<TraversalDirection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutDirection {
    Climb,
    Conventional,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContourOrder {
    #[default]
    InnerBeforeOuter,
    Explicit,
}

/// Start or tab anchor: parameterized by source geometry, not offset vertices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContourAnchor {
    pub contour_id: String,
    pub source_geometry_fingerprint: String,
    /// Fraction in [0,1) along the source contour.
    pub fraction_along_source_contour: f64,
}
impl ContourAnchor {
    fn validate(&self) -> Result<()> {
        if !preview::valid_id(&self.contour_id) {
            return Err(error(
                "PROJECT_PARAMETER",
                "anchor contour IDs must be valid identifiers",
            ));
        }
        let f = self.fraction_along_source_contour;
        if !f.is_finite() || !(0. ..1.).contains(&f) {
            return Err(error(
                "PROJECT_PARAMETER",
                "anchor fraction must lie in [0,1)",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartSelection {
    #[default]
    Automatic,
    Anchor(Box<ContourAnchor>),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LeadSpec {
    #[default]
    None,
    TangentLine {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length_mm: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feed_mm_min: Option<f64>,
    },
    TangentArc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius_mm: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sweep_deg: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feed_mm_min: Option<f64>,
    },
}
impl LeadSpec {
    fn validate(&self, prefix: &str) -> Result<()> {
        match self {
            Self::None => Ok(()),
            Self::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                number(*length_mm, &format!("{prefix}.length_mm"), true)?;
                number(*feed_mm_min, &format!("{prefix}.feed_mm_min"), true)
            }
            Self::TangentArc {
                radius_mm,
                sweep_deg,
                feed_mm_min,
            } => {
                number(*radius_mm, &format!("{prefix}.radius_mm"), true)?;
                number(*feed_mm_min, &format!("{prefix}.feed_mm_min"), true)?;
                if sweep_deg.is_some_and(|v| !v.is_finite() || v <= 0. || v >= 360.) {
                    return Err(error(
                        "PROJECT_PARAMETER",
                        format!("{prefix}.sweep_deg must be in (0,360)"),
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileEntry {
    #[default]
    Plunge,
    Ramp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_angle_deg: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feed_mm_min: Option<f64>,
    },
}
impl ProfileEntry {
    fn validate(&self) -> Result<()> {
        if let Self::Ramp {
            max_angle_deg,
            feed_mm_min,
        } = self
        {
            number(*max_angle_deg, "entry.max_angle_deg", true)?;
            number(*feed_mm_min, "entry.feed_mm_min", true)?;
            if max_angle_deg.is_some_and(|v| v >= 90.) {
                return Err(error(
                    "PROJECT_PARAMETER",
                    "entry.max_angle_deg must be below 90 degrees",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFinishSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Radial roughing allowance; zero is meaningful and preserved exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radial_allowance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_mm_min: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabShape {
    #[default]
    Rectangular,
    Ramped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TabPlacement {
    Automatic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spacing_mm: Option<f64>,
    },
    Manual {
        anchors: Vec<ContourAnchor>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabSettings {
    pub height_mm: Option<f64>,
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub shape: TabShape,
    pub placement: TabPlacement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    pub contours: Vec<ProfileContour>,
    pub assignment: MillingAssignment,
    pub top: HeightRef,
    pub bottom: HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_cut_allowance_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<CutDirection>,
    #[serde(default)]
    pub order: ContourOrder,
    #[serde(default)]
    pub start: StartSelection,
    #[serde(default)]
    pub finish: ProfileFinishSettings,
    #[serde(default)]
    pub entry: ProfileEntry,
    #[serde(default)]
    pub lead_in: LeadSpec,
    #[serde(default)]
    pub lead_out: LeadSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<TabSettings>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct KnifeAlignment {
    /// Initial heading for a cold start. Never defaulted silently; planning
    /// readiness requires it (or an explicit alignment lead) to be set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_heading_deg: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DragKnifeSettings {
    /// Selected open chains and/or closed contours; may be empty while incomplete.
    pub chains: Vec<String>,
    pub assignment: KnifeAssignment,
    pub top: HeightRef,
    pub bottom: HeightRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swivel_depth_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_threshold_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_cut_allowance_mm: Option<f64>,
    #[serde(default)]
    pub start: StartSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_overlap_mm: Option<f64>,
    #[serde(default)]
    pub alignment: KnifeAlignment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OperationSettings {
    FlatVcarve(FlatVcarveSettings),
    Face(FaceSettings),
    Profile(ProfileSettings),
    DragKnife(DragKnifeSettings),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub settings: OperationSettings,
}
fn default_enabled() -> bool {
    true
}

/// The canonical schema-4 CAM job. The `operations` array defines execution order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamJob {
    pub schema_version: u32,
    pub name: String,
    /// `None` is valid for source-free jobs (e.g. stock facing only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceSnapshot>,
    pub import: ImportOptions,
    #[serde(default)]
    pub setup: SetupSettings,
    pub tools: Vec<JobTool>,
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub tolerances: PlanningTolerances,
    /// Preserved legacy descriptive metadata; never a reviewed export contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_machine_profile: Option<MachineProfile>,
}

/// Structural validation context shared by all operations of one job.
struct OperationContext<'a> {
    tool_ids: BTreeSet<&'a str>,
    tools: &'a [JobTool],
    operations: &'a [Operation],
}

impl<'a> OperationContext<'a> {
    fn new(job: &'a CamJob) -> Self {
        Self {
            tool_ids: job.tools.iter().map(|t| t.id.as_str()).collect(),
            tools: &job.tools,
            operations: &job.operations,
        }
    }
    fn require_tool(&self, tool_id: &str, operation_id: &str, role: &str) -> Result<()> {
        if !self.tool_ids.contains(tool_id) {
            return Err(error(
                "PROJECT_TOOL_REFERENCE",
                format!("operation '{operation_id}' references unknown {role} tool '{tool_id}'"),
            ));
        }
        Ok(())
    }
    fn require_geometry_kind(
        &self,
        tool_id: &str,
        expected: &'static str,
        operation_id: &str,
        role: &str,
    ) -> Result<()> {
        if let Some(tool) = self.tools.iter().find(|t| t.id == tool_id)
            && let Some(geometry) = &tool.geometry
            && geometry.kind_name() != expected
        {
            return Err(error(
                "PROJECT_TOOL_KIND",
                format!(
                    "operation '{operation_id}' {role} tool '{tool_id}' has {} geometry",
                    geometry.kind_name()
                ),
            ));
        }
        Ok(())
    }
    fn validate_height_ref(
        &self,
        height: &HeightRef,
        field: &str,
        is_bottom: bool,
        operation_index: usize,
        operation_id: &str,
    ) -> Result<()> {
        height.validate(field, is_bottom)?;
        if let HeightReference::FaceResult {
            operation_id: target_id,
        } = &height.reference
        {
            let Some((index, target)) = self
                .operations
                .iter()
                .enumerate()
                .find(|(_, op)| &op.id == target_id)
            else {
                return Err(error(
                    "PROJECT_HEIGHT_REFERENCE",
                    format!(
                        "operation '{operation_id}' {field} references unknown operation '{target_id}'"
                    ),
                ));
            };
            if !matches!(target.settings, OperationSettings::Face(_)) {
                return Err(error(
                    "PROJECT_HEIGHT_REFERENCE",
                    format!(
                        "operation '{operation_id}' {field} references non-face operation '{target_id}'"
                    ),
                ));
            }
            if index >= operation_index {
                return Err(error(
                    "HEIGHT_REFERENCE_FORWARD",
                    format!(
                        "operation '{operation_id}' {field} references later operation '{target_id}'"
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn validate_face(settings: &FaceSettings, ctx: &OperationContext, id: &str) -> Result<()> {
    if let FaceArea::Rectangle { rect } = &settings.area {
        rect.validate()?;
    }
    number(settings.entry_overrun_mm, "face.entry_overrun_mm", false)?;
    number(settings.exit_overrun_mm, "face.exit_overrun_mm", false)?;
    for (value, name) in [
        (settings.margins.min_x_mm, "face.margins.min_x_mm"),
        (settings.margins.max_x_mm, "face.margins.max_x_mm"),
        (settings.margins.min_y_mm, "face.margins.min_y_mm"),
        (settings.margins.max_y_mm, "face.margins.max_y_mm"),
    ] {
        number(value, name, false)?;
    }
    number(settings.stepdown_mm, "face.stepdown_mm", true)?;
    number(settings.stepover_mm, "face.stepover_mm", true)?;
    if settings.pass_angle_deg.is_some_and(|v| !v.is_finite()) {
        return Err(error(
            "PROJECT_PARAMETER",
            "face.pass_angle_deg must be finite",
        ));
    }
    settings.assignment.validate("face.assignment")?;
    ctx.require_tool(&settings.assignment.tool_id, id, "milling")?;
    Ok(())
}

fn validate_profile(settings: &ProfileSettings, ctx: &OperationContext, id: &str) -> Result<()> {
    for contour in &settings.contours {
        if !preview::valid_id(&contour.contour_id) {
            return Err(error(
                "PROJECT_PARAMETER",
                format!(
                    "operation '{id}' references invalid contour ID '{}'",
                    contour.contour_id
                ),
            ));
        }
        if contour.side == ContourSide::On && contour.traversal.is_none() {
            return Err(error(
                "PROJECT_PARAMETER",
                format!(
                    "operation '{id}' contour '{}' is on-contour and requires an explicit traversal direction",
                    contour.contour_id
                ),
            ));
        }
    }
    number(settings.stepdown_mm, "profile.stepdown_mm", true)?;
    number(
        settings.through_cut_allowance_mm,
        "profile.through_cut_allowance_mm",
        false,
    )?;
    settings.entry.validate()?;
    settings.lead_in.validate("profile.lead_in")?;
    settings.lead_out.validate("profile.lead_out")?;
    if let Some(tabs) = &settings.tabs {
        number(tabs.height_mm, "tabs.height_mm", true)?;
        number(tabs.width_mm, "tabs.width_mm", true)?;
        if let TabPlacement::Automatic { count, spacing_mm } = &tabs.placement {
            if count.is_none() && spacing_mm.is_none() {
                return Err(error(
                    "PROJECT_PARAMETER",
                    "automatic tab placement requires a count or a spacing",
                ));
            }
            number(*spacing_mm, "tabs.placement.spacing_mm", true)?;
        }
        if let TabPlacement::Manual { anchors } = &tabs.placement {
            for anchor in anchors {
                anchor.validate()?;
            }
        }
    }
    if settings.finish.enabled {
        number(
            settings.finish.radial_allowance_mm,
            "finish.radial_allowance_mm",
            false,
        )?;
        number(settings.finish.feed_mm_min, "finish.feed_mm_min", true)?;
    }
    settings.assignment.validate("profile.assignment")?;
    ctx.require_tool(&settings.assignment.tool_id, id, "milling")?;
    if let StartSelection::Anchor(anchor) = &settings.start {
        anchor.validate()?;
    }
    Ok(())
}

fn validate_drag_knife(
    settings: &DragKnifeSettings,
    ctx: &OperationContext,
    id: &str,
) -> Result<()> {
    for chain in &settings.chains {
        if !preview::valid_id(chain) {
            return Err(error(
                "PROJECT_PARAMETER",
                format!("operation '{id}' references invalid chain ID '{chain}'"),
            ));
        }
    }
    number(settings.stepdown_mm, "knife.stepdown_mm", true)?;
    number(settings.swivel_depth_mm, "knife.swivel_depth_mm", true)?;
    number(
        settings.corner_threshold_deg,
        "knife.corner_threshold_deg",
        true,
    )?;
    number(
        settings.through_cut_allowance_mm,
        "knife.through_cut_allowance_mm",
        false,
    )?;
    number(
        settings.closure_overlap_mm,
        "knife.closure_overlap_mm",
        false,
    )?;
    if settings
        .alignment
        .initial_heading_deg
        .is_some_and(|v| !v.is_finite())
    {
        return Err(error(
            "PROJECT_PARAMETER",
            "knife.alignment.initial_heading_deg must be finite",
        ));
    }
    settings.assignment.validate("knife.assignment")?;
    ctx.require_tool(&settings.assignment.tool_id, id, "knife")?;
    ctx.require_geometry_kind(&settings.assignment.tool_id, "drag_knife", id, "knife")?;
    if let StartSelection::Anchor(anchor) = &settings.start {
        anchor.validate()?;
    }
    Ok(())
}

fn validate_flat_vcarve(
    settings: &FlatVcarveSettings,
    ctx: &OperationContext,
    id: &str,
) -> Result<()> {
    ctx.require_tool(&settings.endmill.tool_id, id, "endmill")?;
    ctx.require_tool(&settings.vbit.tool_id, id, "V-bit")?;
    if settings.endmill.tool_id == settings.vbit.tool_id {
        return Err(error(
            "PROJECT_OPERATION",
            format!("operation '{id}' must reference distinct endmill and V-bit tools"),
        ));
    }
    ctx.require_geometry_kind(&settings.endmill.tool_id, "endmill", id, "endmill")?;
    ctx.require_geometry_kind(&settings.vbit.tool_id, "vbit", id, "V-bit")?;
    settings.endmill.validate("flat_vcarve.endmill")?;
    settings.vbit.validate("flat_vcarve.vbit")?;
    number(settings.max_depth_mm, "flat_vcarve.max_depth_mm", true)?;
    number(
        settings.wall_allowance_mm,
        "flat_vcarve.wall_allowance_mm",
        false,
    )?;
    number(
        settings.max_floor_ridge_mm,
        "flat_vcarve.max_floor_ridge_mm",
        false,
    )?;
    number(
        settings.max_detail_residual_mm,
        "flat_vcarve.max_detail_residual_mm",
        false,
    )?;
    if let Some(rough) = &settings.rough {
        rough.validate()?;
    }
    match (settings.mode, &settings.finish) {
        (FlatVcarveMode::Combined, Some(finish)) => finish.validate()?,
        (FlatVcarveMode::Combined, None) => {
            return Err(error(
                "PROJECT_OPERATION",
                format!("operation '{id}' is combined mode and requires V-bit finish settings"),
            ));
        }
        (FlatVcarveMode::EndmillOnly, None) => {}
        (FlatVcarveMode::EndmillOnly, Some(_)) => {
            return Err(error(
                "PROJECT_OPERATION",
                format!(
                    "operation '{id}' is endmill-only mode and must not carry V-bit finish settings"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_machine_profile(profile: &MachineProfile) -> Result<()> {
    if !preview::valid_id(&profile.id) {
        return Err(error("PROJECT_MACHINE", "invalid machine profile ID"));
    }
    number(
        profile.clearance_z_mm,
        "legacy_machine_profile.clearance_z_mm",
        true,
    )?;
    if profile.endmill_tool_number == Some(0) || profile.vbit_tool_number == Some(0) {
        return Err(error("PROJECT_MACHINE", "tool numbers must be positive"));
    }
    if profile.work_offset.as_ref().is_some_and(|s| {
        !matches!(
            s.as_str(),
            "G54" | "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3"
        )
    }) {
        return Err(error("PROJECT_MACHINE", "unsupported work offset"));
    }
    Ok(())
}

impl CamJob {
    /// Structural and supplied-value validation. Missing editable values are
    /// allowed here; they are reported by resolution before planning.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CAM_JOB_SCHEMA_VERSION {
            return Err(error(
                "CAM_JOB_SCHEMA_VERSION",
                "unsupported or missing CamJob schema version",
            ));
        }
        if self.name.trim().is_empty()
            || self.name.len() > 1000
            || self
                .source
                .as_ref()
                .is_some_and(|s| s.filename.len() > 1000)
        {
            return Err(error(
                "PROJECT_NAME",
                "job/source names must be short and the job name nonempty",
            ));
        }
        if self.source.is_none()
            && self
                .operations
                .iter()
                .any(|op| matches!(op.settings, OperationSettings::FlatVcarve(_)))
        {
            return Err(error(
                "PROJECT_OPERATION",
                "flat V-carve operations require an imported source",
            ));
        }
        let mut tool_ids = BTreeSet::new();
        for tool in &self.tools {
            if !preview::valid_id(&tool.id) || !tool_ids.insert(tool.id.as_str()) {
                return Err(error(
                    "PROJECT_TOOL_ID",
                    "job tool IDs must be valid and unique",
                ));
            }
            if let Some(geometry) = &tool.geometry {
                geometry.validate()?;
            }
        }
        let ctx = OperationContext::new(self);
        let mut operation_ids = BTreeSet::new();
        for (index, operation) in self.operations.iter().enumerate() {
            if !preview::valid_id(&operation.id) || !operation_ids.insert(operation.id.as_str()) {
                return Err(error(
                    "PROJECT_OPERATION_ID",
                    "operation IDs must be valid and unique",
                ));
            }
            if operation.name.trim().is_empty() || operation.name.len() > 1000 {
                return Err(error(
                    "PROJECT_OPERATION_ID",
                    "operation names must be nonempty and short",
                ));
            }
            let id = operation.id.as_str();
            let (top, bottom) = match &operation.settings {
                OperationSettings::FlatVcarve(settings) => {
                    validate_flat_vcarve(settings, &ctx, id)?;
                    (None, None)
                }
                OperationSettings::Face(settings) => {
                    validate_face(settings, &ctx, id)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
                OperationSettings::Profile(settings) => {
                    validate_profile(settings, &ctx, id)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
                OperationSettings::DragKnife(settings) => {
                    validate_drag_knife(settings, &ctx, id)?;
                    (Some(&settings.top), Some(&settings.bottom))
                }
            };
            if let Some(top) = top {
                ctx.validate_height_ref(top, "top", false, index, id)?;
            }
            if let Some(bottom) = bottom {
                ctx.validate_height_ref(bottom, "bottom", true, index, id)?;
            }
            // Legacy parity: an explicit flat V-carve depth must fit the stock.
            if let OperationSettings::FlatVcarve(settings) = &operation.settings
                && let Some(depth) = settings.max_depth_mm
                && self.setup.stock.thickness_mm.is_some_and(|h| depth > h)
            {
                return Err(error(
                    "PROJECT_STOCK_DEPTH",
                    format!("operation '{id}' carve depth exceeds stock thickness"),
                ));
            }
        }
        self.setup.validate()?;
        number(
            self.tolerances.motion_tolerance_mm,
            "tolerances.motion_tolerance_mm",
            true,
        )?;
        number(
            self.tolerances.verification_tolerance_mm,
            "tolerances.verification_tolerance_mm",
            true,
        )?;
        if let Some(profile) = &self.legacy_machine_profile {
            validate_machine_profile(profile)?;
        }
        Ok(())
    }

    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 64_000_000 {
            return Err(error(
                "PROJECT_RESOURCE_LIMIT",
                "job exceeds the 64 MB input limit",
            ));
        }
        let job: Self =
            serde_json::from_str(json).map_err(|e| error("PROJECT_JSON", e.to_string()))?;
        job.validate()?;
        Ok(job)
    }

    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map(|s| s + "\n")
            .map_err(|e| error("PROJECT_JSON", e.to_string()))
    }
}
