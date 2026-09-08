//! Request admission shared by the HTTP service and the WebAssembly worker.
use serde::Serialize;
use sha2::{Digest, Sha256};

/// An admitted-or-rejected request: HTTP status, stable code, message.
#[derive(Debug)]
pub struct Failure(pub u16, pub &'static str, pub String);
impl Failure {
    pub fn new(status: u16, code: &'static str, message: &str) -> Self {
        Self(status, code, message.into())
    }
}

/// Stable request identity: same API, same service instance, safe task key.
pub fn validate_identity(
    api_version: &str,
    instance_id: &str,
    request_id: &str,
    revision: u64,
    expected_instance: &str,
) -> Result<(), Failure> {
    if api_version != crate::document::API_VERSION || instance_id != expected_instance {
        return Err(Failure::new(
            409,
            "TASK_INSTANCE",
            "The service changed. Reconnect; previous tasks are not replayed.",
        ));
    }
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || revision > 9_007_199_254_740_991
    {
        return Err(Failure::new(
            400,
            "REQUEST_IDENTITY",
            "A short request ID and a safe revision are required.",
        ));
    }
    Ok(())
}

/// Idempotency hash over an immutable accepted request.
pub fn digest(value: &impl Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("accepted requests serialize"))
    )
}
