//! Verification binds a retained combined plan and explicit options to the shared
//! cancellable worker queue. Reports are produced solely by the M5 verifier.
use crate::{
    document::{API_VERSION, ENGINE_VERSION, UiDiagnostic},
    planning::{Failure, Planning, Snapshot, Status},
    planning_worker::{Input, Output, REPORT_BYTES, Stage},
};
use cam_core::{
    vcarve::{AuthenticatedPlan, VerificationReceipt, verify_retained_plan},
    verification::{VerificationOptions, verify_authenticated_plan},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

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
    pub artifact: PathBuf,
    pub identity: Identity,
    pub receipt: Option<VerificationReceipt>,
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
    if source.stage != Stage::Combined || source.verification.is_some() || source.export.is_some() {
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
    let artifact = result.plan_artifact.clone().ok_or_else(|| {
        Failure::new(
            410,
            "PLAN_RESULT_UNAVAILABLE",
            "The retained plan file is unavailable. Replan the job.",
        )
    })?;
    let input = Input {
        stage: Stage::Combined,
        job: String::new(),
        export: None,
        verification: Some(Work {
            artifact: artifact.path().to_owned(),
            identity: request.verification.clone(),
            receipt: result.verification_receipt.clone(),
        }),
        output_path: None,
        motion_output_path: None,
        source_artifact: Some(artifact),
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
            export: None,
        },
        hash,
        input,
    )
}
pub fn calculate(work: Work) -> Result<Output, Value> {
    let convert = |d| json!(UiDiagnostic::from(d));
    let file = File::open(&work.artifact).map_err(|e| json!({"code":"VERIFICATION_ARTIFACT_IO", "severity":"error", "stage":"verification", "message":format!("Could not read the local plan artifact: {e}")}))?;
    let report = if let Some(receipt) = &work.receipt {
        if !receipt.matches_plan(
            &work.identity.input_fingerprint,
            &work.identity.motion_fingerprint,
        ) {
            return Err(
                json!({"code":"VERIFICATION_PLAN_IDENTITY", "severity":"error", "stage":"verification", "message":"Retained receipt does not match the accepted plan."}),
            );
        }
        verify_retained_plan(
            BufReader::with_capacity(1024 * 1024, file),
            receipt,
            &work.identity.options,
        )
        .map_err(convert)?
    } else {
        let authenticated =
            AuthenticatedPlan::from_reader(BufReader::with_capacity(1024 * 1024, file))
                .map_err(convert)?;
        let plan = authenticated.plan();
        if plan.input_fingerprint != work.identity.input_fingerprint
            || plan.motion_fingerprint != work.identity.motion_fingerprint
        {
            return Err(
                json!({"code":"VERIFICATION_PLAN_IDENTITY", "severity":"error", "stage":"verification", "message":"Worker artifact identity does not match the accepted plan."}),
            );
        }
        verify_authenticated_plan(&authenticated, &work.identity.options).map_err(convert)?
    };
    let artifact = serde_json::to_string(&report).unwrap();
    if artifact.len() > REPORT_BYTES {
        return Err(
            json!({"code":"VERIFICATION_RESULT_LIMIT", "severity":"error", "stage":"verification", "message":"Verification report exceeds the 16 MB service limit. Reduce report detail and run again."}),
        );
    }
    Ok(Output {
        verification_receipt: None,
        summary: json!({"engineVersion": ENGINE_VERSION, "status": report.status,
            "verificationFingerprint": report.verification_fingerprint,
            "originalStatus": report.original.status, "roundedStatus": report.rounded.as_ref().map(|r| r.verification.status),
        }),
        artifact,
        plan_artifact: None,
        motion_artifact: None,
        motion_pages: vec![],
        motions: vec![],
        programs: vec![],
        inspection: Default::default(),
    })
}
