//! Emitted-output knife evidence (plan section 22.10, F3b): the independent
//! no-slip replay run against the **decoded, rounded output coordinates** —
//! never the planner's own coordinates again — together with the trusted
//! plan's intended geometry, and a renderer-neutral bounded report of the
//! intended tip, replayed tip, holder pivot and headings.
//!
//! Intended tips are reconstructed from the trusted plan's modeled motions
//! (`tip = q + d*u(theta)` for the pivot/heading pair each motion carries),
//! never from the decoded points, output comments or client-supplied
//! headings. Knife chords plant their tip at the arc center, which the
//! reconstruction recovers exactly because the pivot-to-tip vector of a
//! chord endpoint lands on that center.
use crate::{
    geometry::{Diagnostic, Point, Result},
    operations::drag_knife::replay::{
        IntendedTip, ReplayOutcome, ReplaySample, ReplayStatus, replay_traced,
    },
    post::sequence::{DecodedProgram, PreparedExecution},
    sequence::{SequencePlan, StageRole},
    toolpath::{MotionEffect, MotionPurpose, PlannedMotion},
};
use serde::{Deserialize, Serialize};

pub const KNIFE_EVIDENCE_ARTIFACT_KIND: &str = "knife_evidence";
pub const KNIFE_EVIDENCE_SCHEMA_VERSION: u32 = 1;
/// Upper bound on retained evidence samples (plan section 22.10: page
/// detail under an explicit budget; the bound can limit detail, never turn
/// a required replay failure into success).
pub const KNIFE_EVIDENCE_MAX_SAMPLES: usize = 20_000;
/// Integration step budget for one emitted replay; exhaustion is
/// inconclusive and blocks output rather than truncating it.
pub const EMITTED_REPLAY_STEP_BUDGET: usize = 2_000_000;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("knife-evidence")
}

/// One emitted replay: aggregate outcome, the geometry assumptions it ran
/// under, and its bounded boundary samples (empty when no sample budget was
/// requested).
pub struct EmittedReplay {
    pub outcome: ReplayOutcome,
    pub blade_offset_mm: f64,
    pub initial_heading_deg: f64,
    pub tip_budget_mm: f64,
    pub heading_tolerance_deg: f64,
    pub total_samples: usize,
    pub truncated: bool,
    pub samples: Vec<ReplaySample>,
}

/// The intended tip one trusted plan motion defines, reconstructed from the
/// modeled pivot/heading pair. Lifted moves have none.
fn intended_from_plan(motion: &PlannedMotion, blade_offset_mm: f64) -> IntendedTip {
    if motion.effect != MotionEffect::KnifeTrace {
        return IntendedTip::Lifted;
    }
    // check_plan guarantees knife motions carry their modeled headings.
    let (h0, h1) = motion.blade_heading_deg.unwrap_or((0., 0.));
    let a = crate::toolpath::knife_tip(motion.start.xy(), h0, blade_offset_mm);
    let b = crate::toolpath::knife_tip(motion.end.xy(), h1, blade_offset_mm);
    if a.distance(b) <= 1e-12 {
        IntendedTip::Planted(a)
    } else {
        IntendedTip::Segment(a, b)
    }
}

fn setup_position(machine: crate::motion::Position, offset: [f64; 3]) -> crate::motion::Position {
    crate::motion::Position {
        x: machine.x + offset[0],
        y: machine.y + offset[1],
        z: machine.z + offset[2],
    }
}

/// Replay the decoded output bytes of a plan's knife stages without
/// retaining samples. Used by export, where only the pass/fail outcome
/// gates the bytes.
pub fn replay_emitted(
    plan: &dyn SequencePlan,
    prepared: &PreparedExecution,
    decoded: &DecodedProgram,
    step_budget: usize,
) -> Result<EmittedReplay> {
    replay_emitted_bounded(plan, prepared, decoded, step_budget, 0)
}

/// Run the emitted replay, optionally recording up to `sample_budget`
/// boundary samples. The decoded machine coordinates are transformed back
/// into setup coordinates (the exact inverse of the output transform —
/// offsets are exactly representable at output precision); effect,
/// headings, intended tips and the initial-heading assumption come from
/// the trusted plan and nothing else.
pub(crate) fn replay_emitted_bounded(
    plan: &dyn SequencePlan,
    prepared: &PreparedExecution,
    decoded: &DecodedProgram,
    step_budget: usize,
    sample_budget: usize,
) -> Result<EmittedReplay> {
    if decoded.motions.len() != plan.motions().len() {
        return Err(error(
            "KNIFE_EVIDENCE_SOURCE",
            "decoded motions do not correspond one-to-one with the plan; \
             run output comparison before replay",
        ));
    }
    let knife_stage_ids: Vec<&str> = plan
        .stages()
        .iter()
        .filter(|stage| stage.role == StageRole::Knife)
        .map(|stage| stage.stage_id.as_str())
        .collect();
    if knife_stage_ids.is_empty() {
        return Err(error(
            "KNIFE_EVIDENCE_SOURCE",
            "the plan contains no knife stages",
        ));
    }
    // One blade offset for the whole replay: knife stages share one knife
    // tool contract in this release; distinct knives need distinct reports.
    let mut blade_offset = None;
    for motion in plan.motions() {
        if knife_stage_ids.contains(&motion.stage_id.as_str()) {
            let offset = plan
                .tool_geometry(&motion.tool_id)
                .and_then(|geometry| match geometry {
                    crate::project::ToolGeometry::DragKnife(spec) => Some(spec.blade_offset_mm),
                    _ => None,
                })
                .ok_or_else(|| {
                    error(
                        "KNIFE_EVIDENCE_SOURCE",
                        format!(
                            "knife motion's tool '{}' has no drag-knife geometry",
                            motion.tool_id
                        ),
                    )
                })?;
            match blade_offset {
                None => blade_offset = Some(offset),
                Some(seen) if seen != offset => {
                    return Err(error(
                        "KNIFE_EVIDENCE_SOURCE",
                        "the plan mixes knife tools with different blade offsets; \
                         emit one report per tool",
                    ));
                }
                _ => {}
            }
        }
    }
    let blade_offset = blade_offset.expect("knife stages present");
    let tip_budget = plan.tolerances().motion_tolerance_mm.ok_or_else(|| {
        error(
            "KNIFE_EVIDENCE_SOURCE",
            "the plan lacks the motion tolerance that bounds tip error",
        )
    })?;
    // Decoded machine coordinates back into setup coordinates. Missing
    // starts (the machine-owned M6 boundary) fall back to the trusted
    // plan's start; the ends are pinned by output comparison anyway.
    let offset = prepared.machine_offset_mm;
    let mut motions: Vec<PlannedMotion> = vec![];
    let mut intended: Vec<IntendedTip> = vec![];
    for (planned, read) in plan.motions().iter().zip(&decoded.motions) {
        let mut replayed = planned.clone();
        replayed.start = match read.start {
            Some(machine) => setup_position(machine, offset),
            None => planned.start,
        };
        replayed.end = setup_position(read.end, offset);
        motions.push(replayed);
        intended.push(intended_from_plan(planned, blade_offset));
    }
    let (outcome, recording) = replay_traced(
        &motions,
        blade_offset,
        &intended,
        tip_budget,
        step_budget,
        sample_budget,
    );
    let initial_heading = plan
        .motions()
        .iter()
        .find(|motion| {
            motion.effect == MotionEffect::KnifeTrace && motion.blade_heading_deg.is_some()
        })
        .and_then(|motion| motion.blade_heading_deg)
        .map_or(0., |(start, _)| start);
    Ok(EmittedReplay {
        outcome,
        blade_offset_mm: blade_offset,
        initial_heading_deg: initial_heading,
        tip_budget_mm: tip_budget,
        heading_tolerance_deg: ((2. * tip_budget / blade_offset).to_degrees()).max(0.01),
        total_samples: recording.total,
        truncated: recording.total > recording.samples.len(),
        samples: recording.samples,
    })
}

/// Status of one evidence report: `within` budgets, a proven `exceeded`
/// deviation/heading error, or `inconclusive` when the integration budget
/// was exhausted (which blocks output; it never becomes success).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Within,
    Exceeded,
    Inconclusive,
}

/// One bounded evidence sample: holder pivot, intended tip and replayed tip
/// at a motion boundary, with modeled and replayed headings and ownership.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct KnifeEvidenceSample {
    /// Global plan motion index and whether the sample sits at its start.
    pub motion_index: usize,
    pub at_start: bool,
    pub operation_id: String,
    pub stage_id: String,
    pub pass_id: usize,
    pub layer: usize,
    pub purpose: MotionPurpose,
    /// Setup-space holder pivot decoded from the emitted bytes.
    pub pivot_mm: Point,
    /// Intended tip from the trusted plan: the planted point, or the two
    /// ends of the intended segment. Lifted moves carry none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intended_tip_mm: Option<(Point, Option<Point>)>,
    pub replayed_tip_mm: Point,
    /// Modeled heading from the plan and replayed heading integrated from
    /// the decoded output. Degrees CCW from +X pointing from the pivot
    /// toward the tip (the implemented wire convention).
    pub modeled_heading_deg: Option<f64>,
    pub replayed_heading_deg: Option<f64>,
    pub deviation_mm: Option<f64>,
}

/// Renderer-neutral bounded knife evidence bound to one exact program
/// (plan section 22.10 F3b). Samples are a bounded projection; the status
/// and maxima reflect every sampled deviation, so truncation cannot turn a
/// required failure into success.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct KnifeEvidenceReport {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    /// Identity binding: execution fingerprint, work-zero transform, output
    /// precision and the exact emitted bytes this evidence was decoded from.
    pub execution_fingerprint: String,
    pub program_sha256: String,
    pub output_decimal_places: usize,
    pub machine_offset_mm: [f64; 3],
    pub blade_offset_mm: f64,
    pub initial_heading_deg: f64,
    pub tip_budget_mm: f64,
    pub heading_tolerance_deg: f64,
    pub status: EvidenceStatus,
    pub max_tip_deviation_mm: f64,
    pub max_heading_error_deg: f64,
    pub total_samples: usize,
    pub sample_budget: usize,
    pub truncated: bool,
    pub samples: Vec<KnifeEvidenceSample>,
}

/// Build the bounded evidence report for a decoded program. The caller
/// supplies the exact program SHA-256; the report is meaningless against
/// any other bytes.
pub fn build_evidence(
    plan: &dyn SequencePlan,
    prepared: &PreparedExecution,
    decoded: &DecodedProgram,
    program_sha256: &str,
    sample_budget: usize,
) -> Result<KnifeEvidenceReport> {
    let sample_budget = sample_budget.clamp(1, KNIFE_EVIDENCE_MAX_SAMPLES);
    let replay = replay_emitted_bounded(
        plan,
        prepared,
        decoded,
        EMITTED_REPLAY_STEP_BUDGET,
        sample_budget,
    )?;
    let status = match replay.outcome.status {
        ReplayStatus::Within => EvidenceStatus::Within,
        ReplayStatus::Exceeded => EvidenceStatus::Exceeded,
        ReplayStatus::BudgetExhausted => EvidenceStatus::Inconclusive,
    };
    let samples = replay
        .samples
        .iter()
        .filter_map(|sample| {
            let motion = plan.motions().get(sample.motion)?;
            let intended = match intended_from_plan(motion, replay.blade_offset_mm) {
                IntendedTip::Lifted => None,
                IntendedTip::Planted(p) => Some((p, None)),
                IntendedTip::Segment(a, b) => Some((a, Some(b))),
            };
            Some(KnifeEvidenceSample {
                motion_index: sample.motion,
                at_start: sample.at_start,
                operation_id: motion.operation_id.clone(),
                stage_id: motion.stage_id.clone(),
                pass_id: motion.pass_id,
                layer: motion.layer,
                purpose: motion.purpose,
                pivot_mm: sample.pivot,
                intended_tip_mm: intended,
                replayed_tip_mm: sample.tip,
                modeled_heading_deg: motion
                    .blade_heading_deg
                    .map(|(start, end)| if sample.at_start { start } else { end }),
                replayed_heading_deg: sample.heading_deg,
                deviation_mm: sample.deviation_mm,
            })
        })
        .collect();
    Ok(KnifeEvidenceReport {
        artifact_kind: KNIFE_EVIDENCE_ARTIFACT_KIND.into(),
        schema_version: KNIFE_EVIDENCE_SCHEMA_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        execution_fingerprint: plan.execution_fingerprint().to_string(),
        program_sha256: program_sha256.to_string(),
        output_decimal_places: prepared.output_decimal_places,
        machine_offset_mm: prepared.machine_offset_mm,
        blade_offset_mm: replay.blade_offset_mm,
        initial_heading_deg: replay.initial_heading_deg,
        tip_budget_mm: replay.tip_budget_mm,
        heading_tolerance_deg: replay.heading_tolerance_deg,
        status,
        max_tip_deviation_mm: replay.outcome.max_tip_deviation_mm,
        max_heading_error_deg: replay.outcome.max_heading_error_deg,
        total_samples: replay.total_samples,
        sample_budget,
        truncated: replay.truncated,
        samples,
    })
}
