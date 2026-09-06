//! M6 output is retained only with the report for the exact emitted UTF-8 bytes.
//! The shared disposable worker reauthenticates and verifies the source plan.
use crate::{
    document::{API_VERSION, ENGINE_VERSION, UiDiagnostic},
    planning::{Failure, Planning, Snapshot, Status},
    planning_worker::{ARTIFACT_BYTES, Input, Output, Stage},
};
use cam_core::{
    post::{LinuxCncProfile, ProgramLayout, export_plan},
    vcarve::CombinedPlan,
    verification::VerificationOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const PROFILE_BYTES: usize = 64_000;
pub const PROGRAM_BYTES: usize = 8_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub plan_task_id: String,
    pub input_fingerprint: String,
    pub motion_fingerprint: String,
    pub profile: LinuxCncProfile,
    pub layout: ProgramLayout,
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
    pub export: Identity,
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
        .export
        .options
        .validate()
        .map_err(|d| Failure(422, "EXPORT_OPTIONS", d.message))?;
    if serde_json::to_vec(&request.export.profile).unwrap().len() > PROFILE_BYTES {
        return Err(Failure::new(
            413,
            "EXPORT_PROFILE_LIMIT",
            "Machine profile exceeds 64 KB.",
        ));
    }
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("export", &request)).unwrap())
    );
    if let Some(snapshot) = service.replay(&request.request_id, &hash)? {
        return Ok(snapshot);
    }
    let (source, result) = service.result(&request.export.plan_task_id)?;
    if source.stage != Stage::Combined || source.verification.is_some() || source.export.is_some() {
        return Err(Failure::new(
            422,
            "EXPORT_STAGE",
            "LinuxCNC export requires a retained combined plan.",
        ));
    }
    if source.revision != request.revision
        || source.document_fingerprint != request.document_fingerprint
        || result.summary["inputFingerprint"] != request.export.input_fingerprint
        || result.summary["motionFingerprint"] != request.export.motion_fingerprint
    {
        return Err(Failure::new(
            409,
            "EXPORT_PLAN_IDENTITY",
            "The requested plan differs from its accepted job or motions.",
        ));
    }
    let input = Input {
        stage: Stage::Combined,
        job: String::new(),
        verification: None,
        export: Some(Work {
            artifact: result.artifact.clone(),
            identity: request.export.clone(),
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
            verification: None,
            export: Some(request.export),
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
            json!({"code":"EXPORT_PLAN_IDENTITY","severity":"error","stage":"export","message":"Worker artifact differs from the accepted plan."}),
        );
    }
    let result = export_plan(
        &plan,
        &work.identity.profile,
        work.identity.layout,
        &work.identity.options,
    )
    .map_err(convert)?;
    let artifact = serde_json::to_string(&result.report).unwrap();
    if artifact.len() > ARTIFACT_BYTES
        || result.programs.iter().map(|p| p.gcode.len()).sum::<usize>() > PROGRAM_BYTES
    {
        return Err(
            json!({"code":"EXPORT_RESULT_LIMIT","severity":"error","stage":"export","message":"Export exceeds the 16 MB report or 8 MB program limit. No partial program set is published."}),
        );
    }
    Ok(Output {
        summary: json!({"engineVersion":ENGINE_VERSION,"status":result.report.status,
            "profileFingerprint":result.report.profile_fingerprint,"reportFingerprint":format!("{:x}",Sha256::digest(artifact.as_bytes())),
            "originalStatus":result.report.plan_verification.status,"emittedStatus":result.report.emitted_verification.as_ref().map(|v|v.status)}),
        artifact,
        programs: result.programs,
        motions: vec![],
        inspection: Default::default(),
    })
}
