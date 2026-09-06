use crate::{
    geometry::{Diagnostic, Point, Result},
    job::{Job, ToolGeometry},
    model::{Depth, Endmill, VBit},
    target::Target,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearingStrategy {
    DepthDependent,
    DeepestRegion,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntryStrategy {
    Plunge,
    Ramp {
        max_angle_deg: f64,
        feed_mm_min: f64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndmillPlanningSettings {
    pub clearance_z_mm: f64,
    pub start_xy_mm: Point,
    pub strategy: ClearingStrategy,
    pub entry: EntryStrategy,
    pub max_layers: usize,
    pub max_loops_per_layer: usize,
    pub max_motions: usize,
}
impl EndmillPlanningSettings {
    pub fn validate(&self) -> Result<()> {
        if !self.clearance_z_mm.is_finite()
            || self.clearance_z_mm <= 0.
            || !self.start_xy_mm.finite()
        {
            return Err(error(
                "PLANNING_SETTINGS",
                "positive clearance Z and finite start XY are required",
            ));
        }
        if !(1..=256).contains(&self.max_layers)
            || !(1..=1024).contains(&self.max_loops_per_layer)
            || !(1..=100_000).contains(&self.max_motions)
        {
            return Err(error(
                "PLANNING_LIMIT",
                "limits must be 1..256 layers, 1..1024 loops/layer, and 1..100000 motions",
            ));
        }
        if let EntryStrategy::Ramp {
            max_angle_deg,
            feed_mm_min,
        } = self.entry
            && (!max_angle_deg.is_finite()
                || max_angle_deg <= 0.
                || max_angle_deg >= 90.
                || !feed_mm_min.is_finite()
                || feed_mm_min <= 0.)
        {
            return Err(error(
                "RAMP_SETTINGS",
                "ramp angle must be between 0 and 90 degrees, with an explicit positive feed",
            ));
        }
        Ok(())
    }
}
pub(super) struct Context {
    pub target: Target,
    pub mill: Endmill,
    pub settings: EndmillPlanningSettings,
    pub allowance: f64,
    pub stepdown: f64,
    pub stepover: f64,
    pub feed: f64,
    pub entry_feed: f64,
    pub spindle: f64,
    pub coverage_tolerance: f64,
    pub guard: f64,
    pub tool_id: String,
    pub operation_id: String,
}
pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("pocket")
}
fn required(v: Option<f64>, name: &str) -> Result<f64> {
    v.ok_or_else(|| {
        error(
            "MISSING_MACHINING_SETTING",
            format!("set {name} before endmill planning"),
        )
    })
}
impl Context {
    pub fn new(job: &Job) -> Result<Self> {
        job.validate_settings()?;
        let settings = job.endmill_planning.clone().ok_or_else(|| {
            error(
                "MISSING_PLANNING_SETTINGS",
                "configure endmill_planning before generating cutting moves",
            )
        })?;
        settings.validate()?;
        let geometry = job.inspect()?.geometry;
        if geometry.selected.rings().is_empty() {
            return Err(error(
                "EMPTY_SELECTION",
                "select at least one region before planning",
            ));
        }
        let depth = required(job.operation.max_depth_mm, "operation.max_depth_mm")?;
        required(job.stock.thickness_mm, "stock.thickness_mm")?;
        let allowance = required(
            job.operation.wall_allowance_mm,
            "operation.wall_allowance_mm",
        )?;
        let tool = job
            .tools
            .iter()
            .find(|t| t.id == job.operation.endmill_id)
            .unwrap();
        let Some(ToolGeometry::Endmill(spec)) = &tool.geometry else {
            return Err(error("MISSING_MACHINING_SETTING", "set endmill dimensions"));
        };
        let mill = Endmill::try_from(spec.clone())?;
        let vbit = job
            .tools
            .iter()
            .find(|t| t.id == job.operation.vbit_id)
            .unwrap();
        let Some(ToolGeometry::Vbit(spec)) = &vbit.geometry else {
            return Err(error(
                "MISSING_MACHINING_SETTING",
                "set V-bit dimensions to define the nominal target angle",
            ));
        };
        let vbit = VBit::try_from(spec.clone())?;
        vbit.validate_depth(Depth::new(depth)?)?;
        let target = Target::for_planning(geometry.selected, Depth::new(depth)?, vbit.angle())?;
        let stepdown = required(tool.max_stepdown_mm, "endmill.max_stepdown_mm")?;
        let stepover = required(tool.stepover_mm, "endmill.stepover_mm")?;
        let feed = required(tool.cutting_feed_mm_min, "endmill.cutting_feed_mm_min")?;
        let spindle = required(tool.spindle_rpm, "endmill.spindle_rpm")?;
        let motion_tolerance = required(
            job.tolerances.motion_tolerance_mm,
            "tolerances.motion_tolerance_mm",
        )?;
        let coverage_tolerance = required(
            job.tolerances.verification_tolerance_mm,
            "tolerances.verification_tolerance_mm",
        )?;
        let e = target.region().grid().tolerance_mm();
        if motion_tolerance
            < target.region().grid().arc_tolerance_mm() + target.region().grid().snap_bound_mm()
        {
            return Err(error(
                "MOTION_PRECISION",
                "motion tolerance must cover the offset arc and grid-snap budgets; refine geometry precision",
            ));
        }
        if coverage_tolerance < 8. * e {
            return Err(error(
                "VERIFICATION_PRECISION",
                "M3 slice coverage tolerance must be at least eight times geometry tolerance",
            ));
        }
        if stepover > mill.radius().mm() || stepover <= 4. * e {
            return Err(error(
                "STEPOVER_RANGE",
                "M3 stepover must exceed four geometry tolerances and be at most half the tool diameter",
            ));
        }
        let entry_feed = match settings.entry {
            EntryStrategy::Plunge => {
                required(tool.plunge_feed_mm_min, "endmill.plunge_feed_mm_min")?
            }
            EntryStrategy::Ramp { feed_mm_min, .. } => feed_mm_min,
        };
        target.boundary().sample(settings.start_xy_mm)?;
        Ok(Self {
            target,
            mill,
            settings,
            allowance,
            stepdown,
            stepover,
            feed,
            entry_feed,
            spindle,
            coverage_tolerance,
            guard: 2. * e,
            tool_id: tool.id.clone(),
            operation_id: job.operation.id.clone(),
        })
    }
    pub fn entry_supported(&self, job: &Job) -> bool {
        match self.settings.entry {
            EntryStrategy::Plunge => self.mill.plunge_capable(),
            EntryStrategy::Ramp { .. } => {
                job.tools
                    .iter()
                    .find(|t| t.id == self.tool_id)
                    .unwrap()
                    .ramp_capable
                    == Some(true)
            }
        }
    }
    pub fn required_clearance(&self, depth: f64) -> f64 {
        depth * self.target.angle().slope() + self.mill.radius().mm() + self.allowance
    }
}
