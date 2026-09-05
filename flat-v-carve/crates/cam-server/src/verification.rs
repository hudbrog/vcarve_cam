//! Verification binds a retained combined plan and explicit options to the shared
//! cancellable worker queue. Reports are produced solely by the M5 verifier.
use crate::{
    document::{API_VERSION, ENGINE_VERSION, UiDiagnostic},
    planning::{Failure, Planning, Snapshot, Status},
    planning_worker::{ARTIFACT_BYTES, Input, Output, Stage},
};
use cam_core::{
    vcarve::CombinedPlan,
    verification::{VerificationOptions, verify_plan},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub plan_task_id: String,
    pub input_fingerprint: String,
    pub motion_fingerprint: String,
    pub options: VerificationOptions,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
    pub api_version: String,
    pub instance_id: String,
    pub request_id: String,
    pub revision: u64,
    pub document_fingerprint: String,
    pub verification: Identity,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    pub artifact: String,
    pub identity: Identity,
}
pub fn start(service: &Arc<Planning>, request: Start) -> Result<Snapshot, Failure> {
    service.validate_identity(
        &request.api_version,
        &request.instance_id,
        &request.request_id,
        request.revision,
    )?;
    request
        .verification
        .options
        .validate()
        .map_err(|d| Failure(422, "VERIFICATION_OPTIONS", d.message))?;
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("verification", &request)).unwrap())
    );
    // A completed retry is still idempotent if the source artifact was evicted.
    if let Some(snapshot) = service.replay(&request.request_id, &hash)? {
        return Ok(snapshot);
    }
    let (source, result) = service.result(&request.verification.plan_task_id)?;
    if source.stage != Stage::Combined || source.verification.is_some() {
        return Err(Failure::new(
            422,
            "VERIFICATION_STAGE",
            "M5 stock verification requires a combined endmill/V-bit plan.",
        ));
    }
    if source.revision != request.revision
        || source.document_fingerprint != request.document_fingerprint
        || result.summary["inputFingerprint"] != request.verification.input_fingerprint
        || result.summary["motionFingerprint"] != request.verification.motion_fingerprint
    {
        return Err(Failure::new(
            409,
            "VERIFICATION_PLAN_IDENTITY",
            "The requested plan differs from its accepted job or motion identity.",
        ));
    }
    let input = Input {
        stage: Stage::Combined,
        job: String::new(),
        verification: Some(Work {
            artifact: result.artifact.clone(),
            identity: request.verification.clone(),
        }),
    };
    service.enqueue(
        Snapshot {
            api_version: API_VERSION,
            engine_version: ENGINE_VERSION,
            instance_id: service.instance_id.clone(),
            task_id: request.request_id,
            revision: source.revision,
            document_fingerprint: source.document_fingerprint,
            stage: Stage::Combined,
            sequence: 1,
            state: Status::Queued,
            diagnostic: None,
            summary: None,
            result_available: false,
            verification: Some(request.verification),
        },
        hash,
        input,
    )
}
pub fn calculate(work: Work) -> Result<Output, Value> {
    let convert = |d| json!(UiDiagnostic::from(d));
    let plan = CombinedPlan::from_json(&work.artifact).map_err(convert)?;
    if plan.input_fingerprint != work.identity.input_fingerprint
        || plan.motion_fingerprint != work.identity.motion_fingerprint
    {
        return Err(
            json!({"code":"VERIFICATION_PLAN_IDENTITY", "severity":"error", "stage":"verification", "message":"Worker artifact identity does not match the accepted plan."}),
        );
    }
    let report = verify_plan(&plan, &work.identity.options).map_err(convert)?;
    let artifact = serde_json::to_string(&report).unwrap();
    if artifact.len() > ARTIFACT_BYTES {
        return Err(
            json!({"code":"VERIFICATION_RESULT_LIMIT", "severity":"error", "stage":"verification", "message":"Verification report exceeds the 16 MB service limit. Reduce report detail and run again."}),
        );
    }
    Ok(Output {
        summary: json!({"engineVersion": ENGINE_VERSION, "status": report.status,
            "verificationFingerprint": report.verification_fingerprint,
            "originalStatus": report.original.status, "roundedStatus": report.rounded.as_ref().map(|r| r.verification.status),
        }),
        artifact,
        motions: vec![],
        inspection: Default::default(),
    })
}
