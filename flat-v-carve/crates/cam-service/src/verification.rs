//! Verification request DTOs. Reports are produced solely by the M5 verifier.
use cam_core::verification::VerificationOptions;
use serde::{Deserialize, Serialize};

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
