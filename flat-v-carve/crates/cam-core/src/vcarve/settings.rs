use crate::{
    geometry::{Diagnostic, Result},
    job::{Job, ToolGeometry},
    model::{Depth, Endmill, VBit},
    target::Target,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VBitPlanningSettings {
    pub max_paths: usize,
    pub max_motions: usize,
    pub max_curve_segments: usize,
    pub max_depth_passes: usize,
    pub max_cleanup_iterations: usize,
    pub quality_sample_spacing_mm: f64,
    pub max_quality_samples: usize,
    pub reachability_max_cells: usize,
    pub stock_slices: usize,
}
impl VBitPlanningSettings {
    pub fn validate(&self) -> Result<()> {
        if !(1..=65_536).contains(&self.max_paths)
            || !(1..=1_000_000).contains(&self.max_motions)
            || !(1..=1_000_000).contains(&self.max_curve_segments)
            || !(1..=256).contains(&self.max_depth_passes)
            || self.max_cleanup_iterations > 8
            || !(1..=1_000_000).contains(&self.max_quality_samples)
            || !(1..=100_000).contains(&self.reachability_max_cells)
            || !(1..=32).contains(&self.stock_slices)
        {
            return Err(error(
                "VBIT_RESOURCE_SETTINGS",
                "V-bit resource limits are outside supported ranges",
            ));
        }
        if !self.quality_sample_spacing_mm.is_finite() || self.quality_sample_spacing_mm <= 0. {
            return Err(error(
                "VBIT_SAMPLE_SPACING",
                "positive finite quality sample spacing required",
            ));
        }
        Ok(())
    }
}
pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("vcarve")
}
fn required(v: Option<f64>, name: &str) -> Result<f64> {
    v.ok_or_else(|| {
        error(
            "MISSING_VBIT_SETTING",
            format!("set {name} before combined planning"),
        )
    })
}
pub(super) struct Context {
    pub target: Target,
    pub mill: Endmill,
    pub tool: VBit,
    pub settings: VBitPlanningSettings,
    pub guard: f64,
    pub tolerance: f64,
    pub motion_tolerance: f64,
    pub ridge: f64,
    pub detail: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    pub spindle: f64,
    pub stepdown: f64,
    pub stepover: f64,
    pub plunge_capable: bool,
    pub tool_id: String,
    pub operation_id: String,
    pub clearance: f64,
}
impl Context {
    pub fn new(job: &Job) -> Result<Self> {
        job.validate_settings()?;
        let settings = job.vbit_planning.clone().ok_or_else(|| {
            error(
                "MISSING_VBIT_SETTINGS",
                "configure vbit_planning for the combined stage",
            )
        })?;
        let geometry = job.inspect()?.geometry;
        let slot = job
            .tools
            .iter()
            .find(|t| t.id == job.operation.vbit_id)
            .unwrap();
        let Some(ToolGeometry::Vbit(spec)) = &slot.geometry else {
            return Err(error("MISSING_VBIT_SETTING", "V-bit geometry is required"));
        };
        let tool = VBit::try_from(spec.clone())?;
        let mill = job
            .tools
            .iter()
            .find(|t| t.id == job.operation.endmill_id)
            .unwrap();
        let Some(ToolGeometry::Endmill(spec)) = &mill.geometry else {
            return Err(error(
                "MISSING_VBIT_SETTING",
                "endmill geometry is required",
            ));
        };
        let mill = Endmill::try_from(spec.clone())?;
        let target = Target::new(
            geometry.selected,
            Depth::new(required(
                job.operation.max_depth_mm,
                "operation.max_depth_mm",
            )?)?,
            tool.angle(),
        )?;
        let e = target.region().grid().tolerance_mm();
        let tolerance = required(
            job.tolerances.verification_tolerance_mm,
            "verification_tolerance_mm",
        )?;
        let motion_tolerance = required(job.tolerances.motion_tolerance_mm, "motion_tolerance_mm")?;
        if tolerance < 8. * e / target.angle().slope().min(1.) {
            return Err(error(
                "VBIT_PRECISION",
                "verification tolerance must cover eight geometry tolerances in both XY and depth",
            ));
        }
        let stepdown = required(slot.max_stepdown_mm, "V-bit max_stepdown_mm")?;
        if (target.depth_cap().mm() / stepdown).ceil() > settings.max_depth_passes as f64 {
            return Err(error(
                "VBIT_PASS_LIMIT",
                "V-bit depth passes exceed the configured budget",
            ));
        }
        let clearance = job
            .endmill_planning
            .as_ref()
            .ok_or_else(|| {
                error(
                    "MISSING_PLANNING_SETTINGS",
                    "endmill_planning establishes the common clearance plane",
                )
            })?
            .clearance_z_mm;
        Ok(Self {
            target,
            mill,
            tool,
            settings,
            guard: 2. * e,
            tolerance,
            motion_tolerance,
            ridge: required(
                job.operation.max_floor_ridge_mm,
                "operation.max_floor_ridge_mm",
            )?,
            detail: required(
                job.operation.max_detail_residual_mm,
                "operation.max_detail_residual_mm",
            )?,
            feed: required(slot.cutting_feed_mm_min, "V-bit cutting_feed_mm_min")?,
            plunge_feed: required(slot.plunge_feed_mm_min, "V-bit plunge_feed_mm_min")?,
            spindle: required(slot.spindle_rpm, "V-bit spindle_rpm")?,
            stepdown,
            stepover: required(slot.stepover_mm, "V-bit stepover_mm")?,
            plunge_capable: slot.plunge_capable.ok_or_else(|| {
                error(
                    "MISSING_VBIT_ENTRY",
                    "explicit V-bit plunge_capable is required",
                )
            })?,
            tool_id: slot.id.clone(),
            operation_id: job.operation.id.clone(),
            clearance,
        })
    }
    pub fn safe_depth(&self, p: crate::geometry::Point) -> Result<f64> {
        let sample = self.target.boundary().sample(p)?;
        let available = sample.signed_distance_mm - self.tool.tip_radius().mm() - self.guard;
        // Threshold intersections can leave a few ulps of apparent positive
        // depth. Such a plunge has no established clearance and collapses when
        // formatted; retain the endpoint at stock top instead.
        if available <= sample.numerical_reserve_mm {
            return Ok(0.);
        }
        Ok((available / self.tool.angle().slope()).clamp(0., self.target.depth_cap().mm()))
    }
}
