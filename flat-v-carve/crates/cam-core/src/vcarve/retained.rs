//! Reuse planning evidence only for a plan retained by its generating service.
//! Portable/untrusted plans continue to use AuthenticatedPlan reconstruction.
use super::*;
use crate::verification::{
    VerificationOptions, VerificationReport, bind_plan_report, verify_motions,
};

/// A small receipt produced alongside a freshly generated plan. It records only
/// established plan identity/completeness, never a geometric verification pass.
/// The service must retain it separately from user-controlled request/artifact
/// data. Deserialization exists for private parent/worker IPC, not plan imports.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReceipt {
    engine_version: String,
    input_fingerprint: String,
    motion_fingerprint: String,
    incomplete: bool,
}
impl VerificationReceipt {
    pub fn matches_plan(&self, input_fingerprint: &str, motion_fingerprint: &str) -> bool {
        self.input_fingerprint == input_fingerprint && self.motion_fingerprint == motion_fingerprint
    }
}

pub fn plan_combined_with_receipt(job: &Job) -> Result<(CombinedPlan, VerificationReceipt)> {
    let plan = plan_combined(job)?;
    let receipt = VerificationReceipt {
        engine_version: env!("CARGO_PKG_VERSION").into(),
        input_fingerprint: plan.input_fingerprint.clone(),
        motion_fingerprint: plan.motion_fingerprint.clone(),
        incomplete: plan.analysis.finish_paths_expected != plan.analysis.finish_paths_executed
            || !plan.generation_issues.is_empty(),
    };
    Ok((plan, receipt))
}

/// Independently check actual cutting sweeps without regenerating planning
/// previews/candidates. The receipt MUST come from the trusted task ledger;
/// accepting a receipt supplied with an imported plan would defeat that binding.
pub fn verify_retained_plan(
    reader: impl std::io::Read,
    receipt: &VerificationReceipt,
    options: &VerificationOptions,
) -> Result<VerificationReport> {
    let mut timing = crate::timing::Timer::new("M5 retained plan");
    options.validate()?;
    let e: Envelope =
        serde_json::from_reader(reader).map_err(|e| error("PLAN_JSON", e.to_string()))?;
    if e.artifact_kind != "combined_plan"
        || e.schema_version != 1
        || e.engine_version != env!("CARGO_PKG_VERSION")
        || receipt.engine_version != e.engine_version
    {
        return Err(error(
            "PLAN_VERSION",
            "unsupported retained plan schema or engine",
        ));
    }
    e.endmill.check_identity()?;
    if e.input_fingerprint != receipt.input_fingerprint
        || e.motion_fingerprint != receipt.motion_fingerprint
        || e.input_fingerprint != identity_for_endmill(&e.endmill.input_fingerprint)?
        || e.motion_fingerprint
            != hash(&(
                &e.input_fingerprint,
                &e.endmill.motion_fingerprint,
                &e.transition,
                &e.vbit_motions,
                &e.executions,
                &e.generation_issues,
            ))?
    {
        return Err(error(
            "STALE_PLAN",
            "retained settings, motions or execution records changed",
        ));
    }
    timing.lap("decode and authenticate");
    let mut report = verify_motions(&e.endmill.job, &e.endmill.motions, &e.vbit_motions, options)?;
    bind_plan_report(
        &mut report,
        &e.input_fingerprint,
        &e.motion_fingerprint,
        receipt.incomplete,
    )?;
    Ok(report)
}
