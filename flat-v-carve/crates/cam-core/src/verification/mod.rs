//! M5: bounded, continuous height-field comparison, independent of polygon unions.
//! Every successful cell covers its entire rectangle, including islands/outside P.
mod adaptive;
mod motion;

use crate::{
    geometry::{Diagnostic, Point, Result},
    job::{Job, ToolGeometry},
    model::{Depth, Endmill, Length, VBit},
    motion::Motion,
    svg::Bounds,
    target::Target,
    vcarve::CombinedPlan,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationOptions {
    pub max_cells: usize,
    pub max_depth: usize,
    pub reachability_max_cells: usize,
    pub max_depth_bands: usize,
    pub max_findings: usize,
    /// Absent means that only the original coordinates are checked.
    pub decimal_places: Option<usize>,
}
impl Default for VerificationOptions {
    fn default() -> Self {
        Self {
            max_cells: 1_000_000,
            max_depth: 24,
            reachability_max_cells: 4096,
            max_depth_bands: 512,
            max_findings: 64,
            decimal_places: None,
        }
    }
}
impl VerificationOptions {
    pub fn validate(&self) -> Result<()> {
        if !(1..=2_000_000).contains(&self.max_cells)
            || !(1..=40).contains(&self.max_depth)
            || !(1..=1_000_000).contains(&self.reachability_max_cells)
            || !(1..=4096).contains(&self.max_depth_bands)
            || !(1..=4096).contains(&self.max_findings)
            || self.decimal_places.is_some_and(|n| n > 9)
        {
            return Err(error(
                "VERIFICATION_OPTIONS",
                "invalid verification resource limits or decimal precision (expected 0..=9)",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Interval {
    pub lower: f64,
    pub upper: f64,
}
impl Interval {
    fn positive(lower: f64, upper: f64) -> Self {
        Self {
            lower: lower.max(0.),
            upper: upper.max(lower).max(0.),
        }
    }
    fn maximum(&mut self, value: Self) {
        self.lower = self.lower.max(value.lower);
        self.upper = self.upper.max(value.upper);
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: String,
    pub status: VerificationStatus,
    pub message: String,
    pub location: Point,
    pub cell: Option<Bounds>,
    pub motion_id: Option<usize>,
    pub measured_mm: Option<Interval>,
    pub limit_mm: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ErrorBounds {
    pub overcut_mm: Interval,
    pub floor_ridge_mm: Interval,
    pub unreachable_detail_mm: Interval,
    pub other_reachable_residual_mm: Interval,
    pub total_residual_mm: Interval,
    pub residual_volume_mm3: Interval,
    pub overcut_volume_mm3: Interval,
}

/// Area bounds valid for every horizontal slice in the entire closed depth band.
#[derive(Clone, Debug, Serialize)]
pub struct DepthBand {
    pub from_depth_mm: f64,
    pub to_depth_mm: f64,
    pub nominal_area_mm2: Interval,
    pub removed_area_mm2: Interval,
    pub residual_area_mm2: Interval,
    pub overcut_area_mm2: Interval,
}

#[derive(Clone, Debug, Serialize)]
pub struct StockVerification {
    pub status: VerificationStatus,
    pub domain: Bounds,
    pub verification_tolerance_mm: f64,
    pub floor_ridge_limit_mm: f64,
    pub detail_residual_limit_mm: f64,
    pub arithmetic_reserve_mm: f64,
    pub source_geometry_depth_error_mm: f64,
    pub checked_motion_count: usize,
    pub analytically_clear_motion_count: usize,
    pub evaluated_cells: usize,
    pub terminal_cells: usize,
    pub unresolved_cells: usize,
    pub maximum_refinement_depth: usize,
    pub reachability_cells: usize,
    pub maximum_error_uncertainty_mm: f64,
    pub bounds: ErrorBounds,
    pub depth_bands: Vec<DepthBand>,
    pub findings: Vec<Finding>,
    pub omitted_findings: usize,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoundedVerification {
    pub decimal_places: usize,
    pub coordinate_quantum_mm: f64,
    pub maximum_coordinate_change_mm: f64,
    pub motion_fingerprint: String,
    pub verification: StockVerification,
}

#[derive(Clone, Debug, Serialize)]
pub struct VerificationReport {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub status: VerificationStatus,
    pub input_fingerprint: String,
    pub motion_fingerprint: String,
    pub verification_fingerprint: String,
    pub authenticated_plan_fingerprint: Option<String>,
    pub options: VerificationOptions,
    pub original: StockVerification,
    pub rounded: Option<RoundedVerification>,
}

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("verification")
}
fn fingerprint(value: &impl Serialize) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(value).map_err(|e| error("VERIFICATION_JSON", e.to_string()))?
        )
    ))
}
fn status(a: VerificationStatus, b: VerificationStatus) -> VerificationStatus {
    use VerificationStatus::*;
    if a == Failed || b == Failed {
        Failed
    } else if a == Inconclusive || b == Inconclusive {
        Inconclusive
    } else {
        Passed
    }
}

struct Context<'a> {
    job: &'a Job,
    target: Target,
    mill: Endmill,
    vbit: VBit,
    allowance: Length,
    tolerance: f64,
    ridge: f64,
    detail: f64,
    reserve: f64,
    source_error: f64,
}
impl<'a> Context<'a> {
    fn new(job: &'a Job) -> Result<Self> {
        let inspection = job.inspect()?;
        let required = |v: Option<f64>, name: &str| {
            v.ok_or_else(|| {
                error(
                    "VERIFICATION_SETTING",
                    format!("set {name} before verification"),
                )
            })
        };
        let geometry = |id: &str| {
            job.tools
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.geometry.as_ref())
        };
        let Some(ToolGeometry::Endmill(e)) = geometry(&job.operation.endmill_id) else {
            return Err(error("VERIFICATION_SETTING", "endmill geometry required"));
        };
        let Some(ToolGeometry::Vbit(v)) = geometry(&job.operation.vbit_id) else {
            return Err(error("VERIFICATION_SETTING", "V-bit geometry required"));
        };
        let mill = Endmill::try_from(e.clone())?;
        let vbit = VBit::try_from(v.clone())?;
        let depth = Depth::new(required(job.operation.max_depth_mm, "max_depth_mm")?)?;
        mill.validate_depth(depth)?;
        vbit.validate_depth(depth)?;
        let tolerance = required(
            job.tolerances.verification_tolerance_mm,
            "verification_tolerance_mm",
        )?;
        required(job.stock.thickness_mm, "stock.thickness_mm")?;
        if job.endmill_planning.is_none() {
            return Err(error(
                "VERIFICATION_SETTING",
                "endmill_planning is required for entry and clearance rules",
            ));
        }
        let source_error = (inspection.geometry.flattening_bound_mm
            + inspection.geometry.source_snap_bound_mm
            + inspection.geometry.grid.snap_bound_mm())
            / vbit.angle().slope();
        let target = Target::new(inspection.geometry.selected, depth, vbit.angle())?;
        let bounds = Bounds::of(target.region()).unwrap();
        let magnitude = [
            bounds.min.x.abs(),
            bounds.min.y.abs(),
            bounds.max.x.abs(),
            bounds.max.y.abs(),
            depth.mm(),
            mill.radius().mm(),
            vbit.max_cutting_radius().mm(),
        ]
        .into_iter()
        .fold(1., f64::max);
        let reserve = 8192. * f64::EPSILON * magnitude / vbit.angle().slope().min(1.);
        Ok(Self {
            job,
            target,
            mill,
            vbit,
            tolerance,
            reserve,
            source_error,
            allowance: Length::new(required(
                job.operation.wall_allowance_mm,
                "wall_allowance_mm",
            )?)?,
            ridge: required(job.operation.max_floor_ridge_mm, "max_floor_ridge_mm")?,
            detail: required(
                job.operation.max_detail_residual_mm,
                "max_detail_residual_mm",
            )?,
        })
    }
}

/// Checks raw motion lists, useful for challenging the verifier independently of
/// generation records. This does not authenticate a saved plan's path families.
pub fn verify_motions(
    job: &Job,
    endmill: &[Motion],
    vbit: &[Motion],
    options: &VerificationOptions,
) -> Result<VerificationReport> {
    options.validate()?;
    let ctx = Context::new(job)?;
    let input_fingerprint = fingerprint(&(env!("CARGO_PKG_VERSION"), job))?;
    let motion_fingerprint = fingerprint(&(endmill, vbit))?;
    let verification_fingerprint =
        fingerprint(&(&input_fingerprint, &motion_fingerprint, options))?;
    let original = adaptive::verify(&ctx, endmill, vbit, options, None, vec![])?;
    let rounded = if let Some(places) = options.decimal_places {
        let (e, v, change, findings) = motion::rounded(endmill, vbit, places);
        Some(RoundedVerification {
            decimal_places: places,
            coordinate_quantum_mm: 10f64.powi(-(places as i32)),
            maximum_coordinate_change_mm: change,
            motion_fingerprint: fingerprint(&(&e, &v))?,
            verification: adaptive::verify(&ctx, &e, &v, options, Some(places), findings)?,
        })
    } else {
        None
    };
    Ok(VerificationReport {
        artifact_kind: "verification_report".into(),
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        status: rounded.as_ref().map_or(original.status, |r| {
            status(original.status, r.verification.status)
        }),
        input_fingerprint,
        motion_fingerprint,
        verification_fingerprint,
        authenticated_plan_fingerprint: None,
        options: options.clone(),
        original,
        rounded,
    })
}

/// Saved/typed plans are authenticated and their execution records are replayed.
/// Cached M4 or M5 report fields are never acceptance evidence.
pub fn verify_plan(
    plan: &CombinedPlan,
    options: &VerificationOptions,
) -> Result<VerificationReport> {
    let plan = CombinedPlan::from_json(&plan.to_json()?)?;
    let mut report = verify_motions(
        &plan.endmill.job,
        &plan.endmill.motions,
        &plan.vbit_motions,
        options,
    )?;
    report.authenticated_plan_fingerprint = Some(fingerprint(&(
        &plan.input_fingerprint,
        &plan.motion_fingerprint,
    ))?);
    report.verification_fingerprint = fingerprint(&(
        &report.verification_fingerprint,
        &report.authenticated_plan_fingerprint,
    ))?;
    if plan.analysis.finish_paths_expected != plan.analysis.finish_paths_executed
        || !plan.generation_issues.is_empty()
    {
        let finding = Finding {
            code: "INCOMPLETE_EXECUTION".into(),
            status: VerificationStatus::Inconclusive,
            message:
                "generation limits or an unfinished required path family prevent plan acceptance"
                    .into(),
            location: report.original.domain.min,
            cell: None,
            motion_id: None,
            measured_mm: None,
            limit_mm: None,
        };
        report.original.status = status(report.original.status, finding.status);
        report.original.findings.push(finding);
        report.status = status(report.status, VerificationStatus::Inconclusive);
    }
    Ok(report)
}
