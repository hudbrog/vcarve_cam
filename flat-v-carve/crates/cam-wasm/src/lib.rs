//! Browser entry for the CAM engine: the same pure service logic the local
//! HTTP service uses, linked as WebAssembly and driven from a Web Worker.
//!
//! Every export is JSON-in/JSON-out with one envelope: `{"ok": …}` or
//! `{"error": …}`. Admission errors carry `{status, code, message}` like the
//! HTTP API; compute errors carry a UI diagnostic so task snapshots can hold
//! them directly. All functions are pure and run identically under `cargo
//! test` on the host.
use cam_core::{
    job::Job,
    pocket::plan_endmill,
    post::export_authenticated_plan,
    vcarve::{
        AuthenticatedPlan, VerificationReceipt, export_retained_plan, plan_combined_with_receipt,
        verify_retained_plan,
    },
    verification::verify_authenticated_plan,
};
use cam_service::{
    admission::{self, Failure},
    document::{self, DocumentRequest, ENGINE_VERSION},
    export,
    inspection::Inspection,
    summary,
    task::{self, Stage},
    verification,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use wasm_bindgen::prelude::*;

fn ok(value: Value) -> String {
    json!({ "ok": value }).to_string()
}
fn admission_error(failure: Failure) -> String {
    json!({ "error": { "status": failure.0, "code": failure.1, "message": failure.2 } }).to_string()
}
fn diagnostic_error(code: &str, stage: &str, message: impl Into<String>) -> String {
    json!({ "error": { "code": code, "severity": "error", "stage": stage, "message": message.into() } })
        .to_string()
}
fn engine_diagnostic(diagnostic: cam_core::geometry::Diagnostic) -> String {
    json!({ "error": document::UiDiagnostic::from(diagnostic) }).to_string()
}

/// Engine version reported by capabilities, envelopes, and task summaries.
#[wasm_bindgen]
pub fn engine_version() -> String {
    ENGINE_VERSION.into()
}

/// Default M5 verification options for capabilities reporting.
#[wasm_bindgen]
pub fn default_verification_options() -> String {
    ok(
        serde_json::to_value(cam_core::verification::VerificationOptions::default())
            .expect("options serialize"),
    )
}

/// Service limits mirrored from the shared crate; the UI builds one
/// capabilities object from these instead of duplicating constants.
#[wasm_bindgen]
pub fn limits() -> String {
    ok(json!({
        "svgBytes": cam_core::svg::MAX_SVG_BYTES,
        "jobBytes": document::JOB_BYTES,
        "requestBytes": document::REQUEST_BYTES,
        "pageMotions": task::PAGE_MOTIONS,
        "reportBytes": task::REPORT_BYTES,
        "profileBytes": export::PROFILE_BYTES,
        "programBytes": export::PROGRAM_BYTES,
        "sliceVertices": cam_service::inspection::SLICE_VERTICES,
        "inspectionVertices": cam_service::inspection::TOTAL_VERTICES,
    }))
}

/// Document operations: open, import, display, validate. Produces the same
/// envelope the HTTP service returns for the same request body.
#[wasm_bindgen]
pub fn document(request_json: &str) -> String {
    let request = match serde_json::from_str::<DocumentRequest>(request_json) {
        Ok(request) => request,
        Err(error) => {
            return admission_error(Failure::new(
                400,
                "REQUEST_JSON",
                &format!("The request body is not valid document JSON: {error}"),
            ));
        }
    };
    if request.api_version != document::API_VERSION {
        return admission_error(Failure::new(
            409,
            "API_VERSION",
            "UI and service API versions differ. Reload the page to get matching builds.",
        ));
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || request.revision > 9_007_199_254_740_991
    {
        return admission_error(Failure::new(
            400,
            "REQUEST_IDENTITY",
            "A short request ID and a safe integer revision are required.",
        ));
    }
    let mut envelope = json!({
        "apiVersion": document::API_VERSION, "engineVersion": ENGINE_VERSION,
        "requestId": request.request_id, "revision": request.revision
    });
    match document::execute(request.command) {
        Ok(data) => {
            envelope["data"] = data;
            ok(envelope)
        }
        Err(diagnostic) => {
            envelope["diagnostic"] = json!(diagnostic);
            ok(envelope)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanStart {
    api_version: String,
    instance_id: String,
    request_id: String,
    revision: u64,
    document_fingerprint: String,
    stage: Stage,
    job: Value,
}

/// Plan admission: identity, size, parse, and fingerprint receipt checks.
/// Returns the immutable request hash used for idempotent retries.
#[wasm_bindgen]
pub fn admit_plan(request_json: &str, instance_id: &str) -> String {
    let request = match serde_json::from_str::<PlanStart>(request_json) {
        Ok(request) => request,
        Err(error) => {
            return admission_error(Failure::new(
                400,
                "REQUEST_JSON",
                &format!("The plan request is not valid JSON: {error}"),
            ));
        }
    };
    if let Err(failure) = admission::validate_identity(
        &request.api_version,
        &request.instance_id,
        &request.request_id,
        request.revision,
        instance_id,
    ) {
        return admission_error(failure);
    }
    let raw = request.job.to_string();
    if raw.len() > document::JOB_BYTES {
        return admission_error(Failure::new(
            413,
            "JOB_RESOURCE_LIMIT",
            "Job exceeds 64 MB.",
        ));
    }
    let job = match Job::from_json(&raw) {
        Ok(job) => job,
        Err(diagnostic) => {
            return admission_error(Failure::new(
                422,
                "PLAN_JOB",
                &format!("{}: {}", diagnostic.code, diagnostic.message),
            ));
        }
    };
    let fingerprint = document::fingerprint(&job);
    if fingerprint != request.document_fingerprint {
        return admission_error(Failure::new(
            409,
            "STALE_DOCUMENT",
            "The submitted job differs from its Rust validation receipt.",
        ));
    }
    let request_hash = admission::digest(&("plan", request.revision, request.stage, &fingerprint));
    ok(json!({ "documentFingerprint": fingerprint, "requestHash": request_hash }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceSnapshot {
    stage: Stage,
    revision: u64,
    document_fingerprint: String,
    verification: Option<Value>,
    export: Option<Value>,
    summary: Value,
}

/// Verification admission against the retained source plan task.
#[wasm_bindgen]
pub fn admit_verification(request_json: &str, instance_id: &str, source_json: &str) -> String {
    let request = match serde_json::from_str::<verification::Start>(request_json) {
        Ok(request) => request,
        Err(error) => {
            return admission_error(Failure::new(
                400,
                "REQUEST_JSON",
                &format!("The verification request is not valid JSON: {error}"),
            ));
        }
    };
    let source = match serde_json::from_str::<SourceSnapshot>(source_json) {
        Ok(source) => source,
        Err(error) => {
            return admission_error(Failure::new(
                500,
                "VERIFICATION_SOURCE",
                &format!("The retained plan task could not be read: {error}"),
            ));
        }
    };
    if let Err(failure) = admission::validate_identity(
        &request.api_version,
        &request.instance_id,
        &request.request_id,
        request.revision,
        instance_id,
    ) {
        return admission_error(failure);
    }
    if let Err(diagnostic) = request.verification.options.validate() {
        return admission_error(Failure::new(
            422,
            "VERIFICATION_OPTIONS",
            &diagnostic.message,
        ));
    }
    let hash = admission::digest(&("verification", &request));
    if source.stage != Stage::Combined || source.verification.is_some() || source.export.is_some() {
        return admission_error(Failure::new(
            422,
            "VERIFICATION_STAGE",
            "M5 stock verification requires a combined endmill/V-bit plan.",
        ));
    }
    if source.revision != request.revision
        || source.document_fingerprint != request.document_fingerprint
        || source.summary["inputFingerprint"] != request.verification.input_fingerprint
        || source.summary["motionFingerprint"] != request.verification.motion_fingerprint
    {
        return admission_error(Failure::new(
            409,
            "VERIFICATION_PLAN_IDENTITY",
            "The requested plan differs from its accepted job or motion identity.",
        ));
    }
    ok(json!({ "requestHash": hash }))
}

/// Export admission against the retained source plan task.
#[wasm_bindgen]
pub fn admit_export(request_json: &str, instance_id: &str, source_json: &str) -> String {
    let request = match serde_json::from_str::<export::Start>(request_json) {
        Ok(request) => request,
        Err(error) => {
            return admission_error(Failure::new(
                400,
                "REQUEST_JSON",
                &format!("The export request is not valid JSON: {error}"),
            ));
        }
    };
    let source = match serde_json::from_str::<SourceSnapshot>(source_json) {
        Ok(source) => source,
        Err(error) => {
            return admission_error(Failure::new(
                500,
                "EXPORT_SOURCE",
                &format!("The retained plan task could not be read: {error}"),
            ));
        }
    };
    if let Err(failure) = admission::validate_identity(
        &request.api_version,
        &request.instance_id,
        &request.request_id,
        request.revision,
        instance_id,
    ) {
        return admission_error(failure);
    }
    if let Err(diagnostic) = request.export.options.validate() {
        return admission_error(Failure::new(422, "EXPORT_OPTIONS", &diagnostic.message));
    }
    if serde_json::to_vec(&request.export.profile).unwrap().len() > export::PROFILE_BYTES {
        return admission_error(Failure::new(
            413,
            "EXPORT_PROFILE_LIMIT",
            "Machine profile exceeds 64 KB.",
        ));
    }
    let hash = admission::digest(&("export", &request));
    if source.stage != Stage::Combined || source.verification.is_some() || source.export.is_some() {
        return admission_error(Failure::new(
            422,
            "EXPORT_STAGE",
            "LinuxCNC export requires a retained combined plan.",
        ));
    }
    if source.revision != request.revision
        || source.document_fingerprint != request.document_fingerprint
        || source.summary["inputFingerprint"] != request.export.input_fingerprint
        || source.summary["motionFingerprint"] != request.export.motion_fingerprint
    {
        return admission_error(Failure::new(
            409,
            "EXPORT_PLAN_IDENTITY",
            "The requested plan differs from its accepted job or motions.",
        ));
    }
    ok(json!({ "requestHash": hash }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanCompute {
    stage: Stage,
    job: String,
}

/// Planning compute for one disposable worker: full plan, summary, inspection,
/// retained plan JSON, and the planning receipt for later fast verification.
#[wasm_bindgen]
pub fn plan(input_json: &str) -> String {
    let input = match serde_json::from_str::<PlanCompute>(input_json) {
        Ok(input) => input,
        Err(error) => {
            return diagnostic_error(
                "PLAN_INPUT",
                "planning",
                format!("Invalid plan input: {error}"),
            );
        }
    };
    let job = match Job::from_json(&input.job) {
        Ok(job) => job,
        Err(diagnostic) => return engine_diagnostic(diagnostic),
    };
    match input.stage {
        Stage::Endmill => {
            let plan = match plan_endmill(&job) {
                Ok(plan) => plan,
                Err(diagnostic) => return engine_diagnostic(diagnostic),
            };
            let output = json!({
                "summary": summary::endmill(&plan),
                "motions": plan.motions,
                "inspection": Inspection::endmill(&plan),
                "verificationReceipt": Value::Null,
                "planJson": serde_json::to_string(&plan).expect("plans serialize"),
            });
            ok(output)
        }
        Stage::Combined => {
            let (plan, receipt) = match plan_combined_with_receipt(&job) {
                Ok(value) => value,
                Err(diagnostic) => return engine_diagnostic(diagnostic),
            };
            let output = json!({
                "summary": summary::combined(&plan),
                "motions": plan.endmill.motions.iter().chain(&plan.vbit_motions).collect::<Vec<_>>(),
                "inspection": Inspection::combined(&plan),
                "verificationReceipt": receipt,
                "planJson": serde_json::to_string(&plan).expect("plans serialize"),
            });
            ok(output)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReportCompute {
    plan_json: String,
    receipt: Option<VerificationReceipt>,
    identity: Value,
}

/// M5 verification compute over a retained in-memory plan.
#[wasm_bindgen]
pub fn verify(input_json: &str) -> String {
    let input = match serde_json::from_str::<ReportCompute>(input_json) {
        Ok(input) => input,
        Err(error) => {
            return diagnostic_error(
                "VERIFICATION_INPUT",
                "verification",
                format!("Invalid verification input: {error}"),
            );
        }
    };
    let identity = match serde_json::from_value::<verification::Identity>(input.identity.clone()) {
        Ok(identity) => identity,
        Err(error) => {
            return diagnostic_error(
                "VERIFICATION_INPUT",
                "verification",
                format!("Invalid verification identity: {error}"),
            );
        }
    };
    let reader = Cursor::new(input.plan_json.as_bytes());
    let report = if let Some(receipt) = &input.receipt {
        if !receipt.matches_plan(&identity.input_fingerprint, &identity.motion_fingerprint) {
            return diagnostic_error(
                "VERIFICATION_PLAN_IDENTITY",
                "verification",
                "Retained receipt does not match the accepted plan.",
            );
        }
        verify_retained_plan(reader, receipt, &identity.options)
    } else {
        match AuthenticatedPlan::from_reader(reader) {
            Ok(authenticated) => {
                let plan = authenticated.plan();
                if plan.input_fingerprint != identity.input_fingerprint
                    || plan.motion_fingerprint != identity.motion_fingerprint
                {
                    return diagnostic_error(
                        "VERIFICATION_PLAN_IDENTITY",
                        "verification",
                        "Worker artifact identity does not match the accepted plan.",
                    );
                }
                verify_authenticated_plan(&authenticated, &identity.options)
            }
            Err(diagnostic) => return engine_diagnostic(diagnostic),
        }
    };
    let report = match report {
        Ok(report) => report,
        Err(diagnostic) => return engine_diagnostic(diagnostic),
    };
    let artifact = serde_json::to_string(&report).expect("reports serialize");
    if artifact.len() > task::REPORT_BYTES {
        return diagnostic_error(
            "VERIFICATION_RESULT_LIMIT",
            "verification",
            "Verification report exceeds the 16 MB service limit. Reduce report detail and run again.",
        );
    }
    ok(json!({
        "summary": {
            "engineVersion": ENGINE_VERSION, "status": report.status,
            "verificationFingerprint": report.verification_fingerprint,
            "originalStatus": report.original.status,
            "roundedStatus": report.rounded.as_ref().map(|r| r.verification.status),
        },
        "reportJson": artifact,
    }))
}

/// LinuxCNC export compute over a retained in-memory plan.
#[wasm_bindgen]
pub fn export_linuxcnc(input_json: &str) -> String {
    let input = match serde_json::from_str::<ReportCompute>(input_json) {
        Ok(input) => input,
        Err(error) => {
            return diagnostic_error(
                "EXPORT_INPUT",
                "export",
                format!("Invalid export input: {error}"),
            );
        }
    };
    let identity = match serde_json::from_value::<export::Identity>(input.identity.clone()) {
        Ok(identity) => identity,
        Err(error) => {
            return diagnostic_error(
                "EXPORT_INPUT",
                "export",
                format!("Invalid export identity: {error}"),
            );
        }
    };
    let reader = Cursor::new(input.plan_json.as_bytes());
    let result = if let Some(receipt) = &input.receipt {
        if !receipt.matches_plan(&identity.input_fingerprint, &identity.motion_fingerprint) {
            return diagnostic_error(
                "EXPORT_PLAN_IDENTITY",
                "export",
                "Retained receipt does not match the accepted plan.",
            );
        }
        export_retained_plan(
            reader,
            receipt,
            &identity.profile,
            identity.layout,
            &identity.options,
        )
    } else {
        match AuthenticatedPlan::from_reader(reader) {
            Ok(authenticated) => {
                let plan = authenticated.plan();
                if plan.input_fingerprint != identity.input_fingerprint
                    || plan.motion_fingerprint != identity.motion_fingerprint
                {
                    return diagnostic_error(
                        "EXPORT_PLAN_IDENTITY",
                        "export",
                        "Worker artifact differs from the accepted plan.",
                    );
                }
                export_authenticated_plan(
                    &authenticated,
                    &identity.profile,
                    identity.layout,
                    &identity.options,
                )
            }
            Err(diagnostic) => return engine_diagnostic(diagnostic),
        }
    };
    let result = match result {
        Ok(result) => result,
        Err(diagnostic) => return engine_diagnostic(diagnostic),
    };
    let artifact = serde_json::to_string(&result.report).expect("reports serialize");
    if artifact.len() > task::REPORT_BYTES
        || result.programs.iter().map(|p| p.gcode.len()).sum::<usize>() > export::PROGRAM_BYTES
    {
        return diagnostic_error(
            "EXPORT_RESULT_LIMIT",
            "export",
            "Export exceeds the 16 MB report or 8 MB program limit. No partial program set is published.",
        );
    }
    ok(json!({
        "summary": {
            "engineVersion": ENGINE_VERSION, "status": result.report.status,
            "profileFingerprint": result.report.profile_fingerprint,
            "reportFingerprint": format!("{:x}", Sha256::digest(artifact.as_bytes())),
            "originalStatus": result.report.plan_verification.status,
            "emittedStatus": result.report.emitted_verification.as_ref().map(|v| v.status),
        },
        "reportJson": artifact,
        "programs": result.programs,
    }))
}

#[cfg(test)]
mod tests;
