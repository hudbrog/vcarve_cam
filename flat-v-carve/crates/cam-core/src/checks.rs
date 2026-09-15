//! Basic automatic plan checks (plan section 14.2).
//!
//! These are bounded structural/numeric checks over the produced motions and
//! execution items — deliberately separate from detailed stock-quality
//! analysis (M5), which stays optional and operation-specific. A plan that
//! fails a basic check cannot be exported; an unrun detailed analysis cannot
//! block generation, and a passing one cannot authorize a broken plan.
use crate::{
    geometry::{Diagnostic, Result},
    sequence::{
        ExecutionItem, ExecutionStage, GenerationStatus, OperationPlan, OperationResult,
        ProcessSpindle, StageRole,
    },
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use serde::{Deserialize, Serialize};

/// Numerical reserve for the sampled cleared-space test, in millimeters. A
/// sample may land exactly on the boundary of a swept region rather than
/// inside it.
const SAMPLE_RESERVE_MM: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CheckFinding {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BasicCheckReport {
    pub status: CheckStatus,
    pub findings: Vec<CheckFinding>,
    /// True when export may proceed once process state is prepared.
    pub export_ready: bool,
}

fn finding(code: &str, message: impl Into<String>) -> CheckFinding {
    CheckFinding {
        code: code.into(),
        message: message.into(),
        operation_id: None,
        stage_id: None,
    }
}

/// Distance from a point to a segment, in the plane.
fn segment_distance(ax: f64, ay: f64, bx: f64, by: f64, px: f64, py: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let length2 = dx * dx + dy * dy;
    if length2 <= f64::EPSILON {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / length2).clamp(0., 1.);
    (px - (ax + t * dx)).hypot(py - (ay + t * dy))
}

impl BasicCheckReport {
    /// First blocking finding when the report failed.
    pub fn first_failure(&self) -> Option<&CheckFinding> {
        if self.status == CheckStatus::Failed {
            self.findings.first()
        } else {
            None
        }
    }
}

/// One expected execution step during the ordered walk.
#[derive(Debug)]
enum Expected<'a> {
    ToolChange(&'a str, &'a ExecutionStage),
    Intent(&'a ExecutionStage),
    Run(&'a ExecutionStage),
}

/// Run the automatic basic checks over a finished schema-4 plan.
pub fn check_plan(plan: &OperationPlan) -> Result<BasicCheckReport> {
    let safety = Safety::of_job(&plan.job_snapshot);
    check_assembly(
        &plan.operation_results,
        &plan.stages,
        &plan.motions,
        &plan.execution,
        &safety,
    )
}

/// The same automatic checks over a schema-5 collection plan (H3): the
/// assembly invariants are identical; only the embedded document differs.
pub fn check_plan_v5(plan: &crate::sequence::OperationPlanV5) -> Result<BasicCheckReport> {
    let safety = Safety::of_job_v5(&plan.job_snapshot);
    check_assembly(
        &plan.operation_results,
        &plan.stages,
        &plan.motions,
        &plan.execution,
        &safety,
    )
}

/// Physical stock and cutter facts the entry-safety pass needs. The plan's own
/// job snapshot supplies both, so the pass never depends on the caller and a
/// hand-built plan cannot dodge it.
struct Safety {
    stock: Option<crate::project::RectXY>,
    tools: Vec<(String, CutterSafety)>,
}

#[derive(Clone, Copy, Debug)]
struct CutterSafety {
    /// Cutter radius at the stock top: half the diameter for an endmill, half
    /// the tip diameter for a V-bit.
    tip_radius_mm: Option<f64>,
    /// Radius gained per millimeter of depth (the V-bit half-angle tangent).
    slope: Option<f64>,
    /// Largest radius the cutter can present, when its geometry bounds one.
    cutting_radius_mm: Option<f64>,
    plunge_capable: Option<bool>,
    ramp_capable: Option<bool>,
}

impl CutterSafety {
    fn of(
        geometry: Option<&crate::project::ToolGeometry>,
        capabilities: &crate::project::ToolCapabilities,
    ) -> Self {
        use crate::project::ToolGeometry;
        let (tip_radius_mm, slope, cutting_radius_mm) = match geometry {
            Some(ToolGeometry::Endmill(g)) => {
                (Some(g.diameter_mm / 2.), Some(0.), Some(g.diameter_mm / 2.))
            }
            Some(ToolGeometry::Vbit(v)) => {
                let slope = (v.included_angle_deg / 2.).to_radians().tan();
                (
                    Some(v.tip_diameter_mm / 2.),
                    slope.is_finite().then_some(slope),
                    Some(v.max_cutting_diameter_mm / 2.),
                )
            }
            // A passive knife is pressed into its material by design and its
            // stage role is exempt from this pass; unknown geometry cannot
            // prove anything about clearance.
            Some(ToolGeometry::DragKnife(_)) | None => (None, None, None),
        };
        Self {
            tip_radius_mm,
            slope,
            cutting_radius_mm,
            plunge_capable: capabilities.plunge_capable,
            ramp_capable: capabilities.ramp_capable,
        }
    }

    /// Cutter radius where it reaches `depth_mm` below the stock top.
    fn radius_at(&self, depth_mm: f64) -> Option<f64> {
        let radius = self.tip_radius_mm? + self.slope.unwrap_or(0.) * depth_mm.max(0.);
        Some(match self.cutting_radius_mm {
            Some(cap) => radius.min(cap),
            None => radius,
        })
    }
}

impl Safety {
    fn of_job(job: &crate::project::CamJob) -> Self {
        Self {
            stock: job.setup.stock.xy,
            tools: job
                .tools
                .iter()
                .map(|tool| {
                    (
                        tool.id.clone(),
                        CutterSafety::of(tool.geometry.as_ref(), &tool.capabilities),
                    )
                })
                .collect(),
        }
    }

    fn of_job_v5(job: &crate::project::v5::CamJobV5) -> Self {
        Self {
            stock: job.setup.stock.xy,
            tools: job
                .tools
                .iter()
                .map(|tool| {
                    (
                        tool.id.clone(),
                        CutterSafety::of(tool.geometry.as_ref(), &tool.capabilities),
                    )
                })
                .collect(),
        }
    }

    fn cutter(&self, tool_id: &str) -> Option<&CutterSafety> {
        self.tools
            .iter()
            .find(|(id, _)| id == tool_id)
            .map(|(_, cutter)| cutter)
    }

    /// Distance from a point to the stock rectangle, or `None` without stock.
    fn distance(&self, x: f64, y: f64) -> Option<f64> {
        let rect = self.stock?;
        let max_x = rect.min_x_mm + rect.width_mm;
        let max_y = rect.min_y_mm + rect.length_mm;
        let dx = if x < rect.min_x_mm {
            rect.min_x_mm - x
        } else if x > max_x {
            x - max_x
        } else {
            0.
        };
        let dy = if y < rect.min_y_mm {
            rect.min_y_mm - y
        } else if y > max_y {
            y - max_y
        } else {
            0.
        };
        Some(dx.hypot(dy))
    }

    /// Whether a descent between two points can reach the stock at all.
    fn descent_clear(
        &self,
        start: crate::motion::Position,
        end: crate::motion::Position,
        radius: f64,
    ) -> bool {
        // Tangent contact is not material engagement: a cutter that only
        // touches the stock boundary has zero overlap with the material, so
        // descending there is a side entry rather than a plunge.
        let clear = |p: crate::motion::Position| {
            self.distance(p.x, p.y)
                .is_some_and(|d| d + SAMPLE_RESERVE_MM >= radius)
        };
        if !clear(start) || !clear(end) {
            return false;
        }
        // The distance to a convex rectangle is convex along the move, so an
        // endpoint-only test would miss a span that dips across a corner.
        !self.crosses_stock(start, end)
    }

    /// Whether the part of a descending cutter's footprint that lies inside
    /// the stock was already cut to this depth or deeper by the given earlier
    /// cutting motions of the same stage — a deeper layer of one loop
    /// re-enters the corridor its own earlier passes opened.
    ///
    /// Sampled at the centre, half radius and full radius. This is a bound,
    /// not a heightfield: the basic checks have to stay cheap, and the planner
    /// remains the authority on where material is. A missed sample can only
    /// make this pass stricter, never looser.
    fn footprint_cleared(
        &self,
        cleared: &[&PlannedMotion],
        surface: f64,
        descent: &PlannedMotion,
        radius: f64,
    ) -> bool {
        if self.stock.is_none() || cleared.is_empty() {
            return false;
        }
        let target = descent.end.z;
        let inside = |x: f64, y: f64| {
            self.stock.is_some_and(|rect| {
                x >= rect.min_x_mm
                    && x <= rect.min_x_mm + rect.width_mm
                    && y >= rect.min_y_mm
                    && y <= rect.min_y_mm + rect.length_mm
            })
        };
        let cut = |x: f64, y: f64| {
            // Most recent first: a deeper layer's covered space was cut by the
            // passes immediately before it.
            cleared.iter().rev().any(|prior| {
                let prior_cut = prior.start.z.min(prior.end.z);
                if prior_cut > target + 1e-9 {
                    return false;
                }
                let Some(prior_radius) = self
                    .cutter(&prior.tool_id)
                    .and_then(|cutter| cutter.radius_at((surface - prior_cut).max(0.)))
                else {
                    return false;
                };
                // The sample sits exactly on a swept boundary when a layer
                // re-enters its own corridor; compare with a numerical
                // reserve rather than deciding the boundary by one ulp.
                segment_distance(prior.start.x, prior.start.y, prior.end.x, prior.end.y, x, y)
                    <= prior_radius + SAMPLE_RESERVE_MM
            })
        };
        // Unit offsets scaled by the radius: centre, cardinal and diagonal
        // points on the full circle, and the cardinal points at half radius.
        const UNIT: f64 = std::f64::consts::FRAC_1_SQRT_2;
        let offsets = [
            (0., 0.),
            (1., 0.),
            (-1., 0.),
            (0., 1.),
            (0., -1.),
            (0.5, 0.),
            (-0.5, 0.),
            (0., 0.5),
            (0., -0.5),
            (UNIT, UNIT),
            (-UNIT, UNIT),
            (UNIT, -UNIT),
            (-UNIT, -UNIT),
        ];
        offsets.iter().all(|(dx, dy)| {
            let x = descent.end.x + dx * radius;
            let y = descent.end.y + dy * radius;
            !inside(x, y) || cut(x, y)
        })
    }

    /// Whether the XY span of a move intersects the stock rectangle.
    fn crosses_stock(&self, start: crate::motion::Position, end: crate::motion::Position) -> bool {
        let Some(rect) = self.stock else {
            return false;
        };
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let mut low = 0.0_f64;
        let mut high = 1.0_f64;
        for (origin, delta, min, max) in [
            (start.x, dx, rect.min_x_mm, rect.min_x_mm + rect.width_mm),
            (start.y, dy, rect.min_y_mm, rect.min_y_mm + rect.length_mm),
        ] {
            if delta.abs() <= f64::EPSILON {
                if origin < min || origin > max {
                    return false;
                }
                continue;
            }
            let (t0, t1) = ((min - origin) / delta, (max - origin) / delta);
            let (t0, t1) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
            low = low.max(t0);
            high = high.min(t1);
            if low > high {
                return false;
            }
        }
        true
    }
}

fn check_assembly(
    operation_results: &[OperationResult],
    stages: &[ExecutionStage],
    motions: &[PlannedMotion],
    execution: &[ExecutionItem],
    safety: &Safety,
) -> Result<BasicCheckReport> {
    let mut findings = vec![];
    let mut failed = false;
    macro_rules! fail {
        ($code:expr, $message:expr) => {{
            findings.push(finding($code, $message));
            failed = true;
        }};
    }

    // Generation completeness: incomplete or inconclusive operations block
    // export of this sequence but the report itself stays deterministic.
    for result in operation_results {
        match result.generation_status {
            GenerationStatus::Complete | GenerationStatus::Empty => {}
            GenerationStatus::Incomplete | GenerationStatus::Inconclusive => {
                let mut f = finding(
                    "PLAN_GENERATION_INCOMPLETE",
                    format!(
                        "operation '{}' did not generate completely; export is unavailable for this sequence",
                        result.operation_id
                    ),
                );
                f.operation_id = Some(result.operation_id.clone());
                findings.push(f);
                failed = true;
            }
        }
    }

    // Stage/motion ownership: dense motion ids, exactly one stage each,
    // stages ordered and contiguous.
    for (expected_id, motion) in motions.iter().enumerate() {
        if motion.id != expected_id {
            fail!(
                "PLAN_MOTION_IDS",
                format!(
                    "motion id {} breaks the dense global ordering at {}",
                    motion.id, expected_id
                )
            );
            break;
        }
    }
    for (index, stage) in stages.iter().enumerate() {
        if stage.motion_range.0 >= stage.motion_range.1 {
            let mut f = finding(
                "PLAN_STAGE_RANGE",
                format!(
                    "stage '{}' has an empty or inverted motion range",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            f.operation_id = Some(stage.operation_id.clone());
            findings.push(f);
            failed = true;
            continue;
        }
        if stage.motion_range.1 > motions.len() {
            let mut f = finding(
                "PLAN_STAGE_RANGE",
                format!("stage '{}' extends past the motion array", stage.stage_id),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
            continue;
        }
        if index > 0 && stages[index - 1].motion_range.1 != stage.motion_range.0 {
            let mut f = finding(
                "PLAN_STAGE_ORDER",
                format!(
                    "stage '{}' does not continue where the previous stage ended",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
        }
        for motion in &motions[stage.motion_range.0..stage.motion_range.1] {
            if motion.stage_id != stage.stage_id || motion.tool_id != stage.tool_id {
                let mut f = finding(
                    "PLAN_STAGE_OWNERSHIP",
                    format!(
                        "motion {} does not belong to stage '{}'",
                        motion.id, stage.stage_id
                    ),
                );
                f.stage_id = Some(stage.stage_id.clone());
                findings.push(f);
                failed = true;
                break;
            }
        }
    }
    let covered: usize = stages
        .iter()
        .map(|s| s.motion_range.1 - s.motion_range.0)
        .sum();
    if covered != motions.len() {
        fail!(
            "PLAN_STAGE_COVERAGE",
            format!(
                "{} of {} motions are covered by stages",
                covered,
                motions.len()
            )
        );
    }

    // Motion semantics: finite coordinates, positive feeds on linear feeds,
    // no feed on rapids, milling sweeps only with a milling stage role.
    for motion in motions {
        if !motion.start.finite() || !motion.end.finite() {
            let mut f = finding(
                "PLAN_MOTION_NUMERIC",
                format!("motion {} has non-finite coordinates", motion.id),
            );
            f.operation_id = Some(motion.operation_id.clone());
            findings.push(f);
            failed = true;
        }
        match motion.interpolation {
            Interpolation::LinearFeed => {
                if !motion.feed_mm_min.is_some_and(|f| f.is_finite() && f > 0.) {
                    let mut f = finding(
                        "PLAN_FEED_REQUIRED",
                        format!("linear feed motion {} lacks a positive feed", motion.id),
                    );
                    f.operation_id = Some(motion.operation_id.clone());
                    findings.push(f);
                    failed = true;
                }
            }
            Interpolation::Rapid => {
                if motion.feed_mm_min.is_some() {
                    let mut f = finding(
                        "PLAN_FEED_INVALID",
                        format!("rapid motion {} carries a feed", motion.id),
                    );
                    f.operation_id = Some(motion.operation_id.clone());
                    findings.push(f);
                    failed = true;
                }
            }
        }
        let role = stage_role(stages, &motion.stage_id);
        let knife_stage = role == Some(StageRole::Knife);
        if motion.effect == MotionEffect::MillingSweep
            && !matches!(
                role,
                Some(
                    StageRole::VcarveRough
                        | StageRole::VcarveFinish
                        | StageRole::Face
                        | StageRole::ProfileRough
                        | StageRole::ProfileFinish
                )
            )
        {
            let mut f = finding(
                "PLAN_EFFECT_ROLE_MISMATCH",
                format!("motion {} removes stock outside a milling stage", motion.id),
            );
            f.stage_id = Some(motion.stage_id.clone());
            findings.push(f);
            failed = true;
        }
        // Knife semantics: blade traces and knife purposes belong to knife
        // stages only, and every knife motion carries the modeled blade
        // headings the pivot/tip display contract needs (plan sections 12.1
        // and 15.3). Milling motions never carry headings.
        if motion.effect == MotionEffect::KnifeTrace && !knife_stage {
            let mut f = finding(
                "PLAN_EFFECT_ROLE_MISMATCH",
                format!(
                    "motion {} traces knife material outside a knife stage",
                    motion.id
                ),
            );
            f.stage_id = Some(motion.stage_id.clone());
            findings.push(f);
            failed = true;
        }
        if matches!(
            motion.purpose,
            MotionPurpose::KnifeCut | MotionPurpose::KnifeAlign | MotionPurpose::KnifeSwivel
        ) && !knife_stage
        {
            let mut f = finding(
                "PLAN_EFFECT_ROLE_MISMATCH",
                format!(
                    "motion {} uses a knife purpose outside a knife stage",
                    motion.id
                ),
            );
            f.stage_id = Some(motion.stage_id.clone());
            findings.push(f);
            failed = true;
        }
        match (&motion.blade_heading_deg, knife_stage) {
            (None, true) => {
                let mut f = finding(
                    "PLAN_KNIFE_HEADING",
                    format!(
                        "knife motion {} lacks the modeled blade heading the tip display needs",
                        motion.id
                    ),
                );
                f.stage_id = Some(motion.stage_id.clone());
                findings.push(f);
                failed = true;
            }
            (Some(_), false) => {
                let mut f = finding(
                    "PLAN_KNIFE_HEADING",
                    format!(
                        "motion {} carries blade headings outside a knife stage",
                        motion.id
                    ),
                );
                f.stage_id = Some(motion.stage_id.clone());
                findings.push(f);
                failed = true;
            }
            _ => {}
        }
        if let Some((start, end)) = motion.blade_heading_deg
            && (!start.is_finite() || !end.is_finite())
        {
            let mut f = finding(
                "PLAN_MOTION_NUMERIC",
                format!("motion {} has non-finite blade headings", motion.id),
            );
            f.operation_id = Some(motion.operation_id.clone());
            findings.push(f);
            failed = true;
        }
    }

    // The surface each stage cuts from: the highest Z at which that stage's
    // own cutting motions start. A stage that follows a facing pass starts at
    // the faced plane, so measuring a descent from the original stock top
    // would report material an earlier operation already removed.
    let mut stage_tops: Vec<(&str, f64)> = vec![];
    for stage in stages {
        let top = motions
            .get(stage.motion_range.0..stage.motion_range.1)
            .unwrap_or(&[])
            .iter()
            .filter(|motion| motion.effect == MotionEffect::MillingSweep)
            .map(|motion| motion.start.z)
            .fold(f64::NEG_INFINITY, f64::max);
        if top.is_finite() {
            stage_tops.push((stage.stage_id.as_str(), top));
        }
    }
    let stage_top = |stage_id: &str| {
        stage_tops
            .iter()
            .find(|(id, _)| *id == stage_id)
            .map_or(0., |(_, top)| *top)
    };

    // Entry safety (field-test finding 1.1): a motion that descends below the
    // surface its stage cuts from may only do so where the cutter is clear of
    // the stock, where this stage already cleared that space, or with a
    // capability that authorizes the descent. A rapid is never authorized, so
    // no G0 can be emitted into material. This keeps a cutter that cannot
    // plunge out of the stock whatever planner produced the motions.
    //
    // A stage whose chain skips a position would defeat this: the post emits
    // one block per motion endpoint, so the machine travels from where it
    // really is to the next endpoint, and a descent can hide behind a claimed
    // start. Continuity is checked first, and every descent is measured from
    // the position the machine is actually at.
    for stage in stages {
        let stage_motions = &motions[stage.motion_range.0..stage.motion_range.1];
        for (index, motion) in stage_motions.iter().enumerate() {
            if index == 0 {
                continue;
            }
            let previous = stage_motions[index - 1].end;
            let gap = (motion.start.x - previous.x)
                .abs()
                .max((motion.start.y - previous.y).abs())
                .max((motion.start.z - previous.z).abs());
            // A micrometre is the smallest gap the rounded output could even
            // express; anything above it is a move the machine would make and
            // the plan would not describe.
            if gap > 1e-6 {
                let mut f = finding(
                    "PLAN_MOTION_DISCONTINUITY",
                    format!(
                        "motion {} starts {gap:.3} mm from where motion {} ends, so the emitted program would travel there without the plan describing it",
                        motion.id,
                        stage_motions[index - 1].id
                    ),
                );
                f.operation_id = Some(motion.operation_id.clone());
                f.stage_id = Some(motion.stage_id.clone());
                findings.push(f);
                failed = true;
            }
        }
    }
    for stage in stages {
        // A passive knife is pressed into its material by design.
        if stage.role == StageRole::Knife {
            continue;
        }
        // Never lower the reference *above* the original stock top: an
        // operation that starts in the air (a positive top offset) descends
        // through nothing, and material exists only from Z = 0 downward.
        let surface = stage_top(&stage.stage_id).min(0.);
        // Cutting motions of this stage that already ran, for the cleared
        // space test below.
        let mut cleared: Vec<&PlannedMotion> = vec![];
        // Where the machine really is before this motion: its start, unless the
        // plan left a gap, in which case the previous motion's end is the
        // truth the control will interpolate from.
        let mut real_previous: Option<crate::motion::Position> = None;
        for motion in &motions[stage.motion_range.0..stage.motion_range.1] {
            let from = real_previous.unwrap_or(motion.start);
            if from.z > motion.end.z {
                let depth = surface - motion.end.z;
                if depth > 0. {
                    let cutter = safety.cutter(&motion.tool_id);
                    let radius = cutter.and_then(|cutter| cutter.radius_at(depth));
                    let in_air = radius.is_some_and(|radius| {
                        safety.descent_clear(from, motion.end, radius)
                    });
                    let axial = (motion.end.x - from.x).abs() <= 1e-9
                        && (motion.end.y - from.y).abs() <= 1e-9;
                    // A rapid is never a licensed entry. A feed descent needs a
                    // tool that may make it: declared plunge capability, or
                    // ramp capability for a non-axial entry. An undeclared
                    // capability is tolerated here because the planners that
                    // depend on the answer (facing, flat v-carve) report the
                    // missing field themselves, and treating silence as a
                    // refusal would stop every saved profile job that never
                    // declared it. Requiring the declaration for profile and
                    // pocket entries is the recorded follow-up.
                    let authorized = motion.interpolation == Interpolation::LinearFeed
                        && cutter.is_some_and(|cutter| match cutter.plunge_capable {
                            Some(true) => true,
                            Some(false) => !axial && cutter.ramp_capable == Some(true),
                            None => true,
                        });
                    // A deeper layer of the same loop re-enters the corridor
                    // its own earlier passes already opened; only ask when the
                    // cheaper answers have not settled it.
                    let allowed = in_air
                        || authorized
                        || radius.is_some_and(|radius| {
                            safety.footprint_cleared(&cleared, surface, motion, radius)
                        });
                    if !allowed {
                        let mut f = finding(
                            "PLAN_ENTRY_UNVERIFIED",
                            format!(
                                "motion {} descends {depth:.3} mm into the material at ({:.3}, {:.3}) where the cutter is neither clear of the stock nor entering space this stage already cut, and tool '{}' is not declared able to enter the material there",
                                motion.id, motion.end.x, motion.end.y, motion.tool_id
                            ),
                        );
                        f.stage_id = Some(motion.stage_id.clone());
                        f.operation_id = Some(motion.operation_id.clone());
                        findings.push(f);
                        failed = true;
                    }
                }
            }
            if motion.effect == MotionEffect::MillingSweep {
                cleared.push(motion);
            }
            real_previous = Some(motion.end);
        }
    }

    // Execution accounting: every stage runs exactly once in order under its
    // own process intent. A tool change appears exactly when the selected
    // tool changes — consecutive same-tool stages continue without one, and
    // recurring tools are re-selected rather than regrouped.
    let mut expected: Vec<Expected<'_>> = vec![];
    let mut selected: Option<&str> = None;
    for stage in stages {
        if selected != Some(stage.tool_id.as_str()) {
            expected.push(Expected::ToolChange(&stage.tool_id, stage));
            selected = Some(stage.tool_id.as_str());
        }
        expected.push(Expected::Intent(stage));
        expected.push(Expected::Run(stage));
    }
    let mut actual = execution.iter();
    for want in &expected {
        let Some(got) = actual.next() else {
            let mut f = finding(
                "PLAN_EXECUTION_INCOMPLETE",
                format!(
                    "execution ends before the ordered steps of stage '{}'",
                    stage_of(want).stage_id
                ),
            );
            f.stage_id = Some(stage_of(want).stage_id.clone());
            findings.push(f);
            failed = true;
            break;
        };
        let ok = match (want, got) {
            (Expected::ToolChange(tool, _), ExecutionItem::ToolChange { tool_id }) => {
                tool == tool_id
            }
            (Expected::Intent(stage), ExecutionItem::SetProcessIntent { intent }) => {
                if matches!(intent.spindle, ProcessSpindle::Off) && stage.role != StageRole::Knife {
                    let mut f = finding(
                        "PROCESS_SPINDLE_STATE",
                        format!("milling stage '{}' requests spindle off", stage.stage_id),
                    );
                    f.stage_id = Some(stage.stage_id.clone());
                    findings.push(f);
                    failed = true;
                }
                true
            }
            (Expected::Run(stage), ExecutionItem::RunStage { stage_id }) => {
                &stage.stage_id == stage_id
            }
            _ => false,
        };
        if !ok {
            let stage = stage_of(want);
            let mut f = finding(
                "PLAN_EXECUTION_ORDER",
                format!(
                    "execution step for stage '{}' does not match the ordered plan",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
            break;
        }
    }
    if actual.next().is_some() {
        fail!(
            "PLAN_EXECUTION_EXTRA",
            "execution contains items after the last stage ran".to_string()
        );
    }

    let status = if failed {
        CheckStatus::Failed
    } else {
        CheckStatus::Passed
    };
    Ok(BasicCheckReport {
        status,
        findings,
        export_ready: status == CheckStatus::Passed,
    })
}

fn stage_of<'a>(expected: &'a Expected<'_>) -> &'a ExecutionStage {
    match expected {
        Expected::ToolChange(_, stage) | Expected::Intent(stage) | Expected::Run(stage) => stage,
    }
}

fn stage_role(stages: &[ExecutionStage], stage_id: &str) -> Option<StageRole> {
    stages
        .iter()
        .find(|s| s.stage_id == stage_id)
        .map(|s| s.role)
}

/// Convenience wrapper used by export preparation: pass or surface the report.
pub fn require_pass(report: &BasicCheckReport) -> Result<()> {
    match report.status {
        CheckStatus::Passed => Ok(()),
        CheckStatus::Failed | CheckStatus::Inconclusive => Err(report
            .findings
            .first()
            .map(|f| Diagnostic::new("PLAN_BASIC_CHECKS", f.message.clone()).at_stage("checks"))
            .unwrap_or_else(|| {
                Diagnostic::new("PLAN_BASIC_CHECKS", "basic checks did not pass").at_stage("checks")
            })),
    }
}
