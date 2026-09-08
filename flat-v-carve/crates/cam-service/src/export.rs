//! LinuxCNC export request DTOs. Output is retained only with the report for
//! the exact emitted UTF-8 bytes.
use cam_core::{
    post::{LinuxCncProfile, ProgramLayout},
    verification::VerificationOptions,
};
use serde::{Deserialize, Serialize};

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

/// Retained machine-profile bound.
pub const PROFILE_BYTES: usize = 64_000;
/// Total retained LinuxCNC program bytes.
pub const PROGRAM_BYTES: usize = 8_000_000;
