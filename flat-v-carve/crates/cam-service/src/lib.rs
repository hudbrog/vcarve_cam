//! Pure service DTOs and projections shared by the command-line tool, the
//! local HTTP service and the WebAssembly build. No filesystem, async runtime,
//! or UI access lives here.
pub mod admission;
pub mod collection;
pub mod export;
pub mod retained;
pub mod task;
pub mod verification;

/// The largest job document a command accepts.
pub const JOB_BYTES: usize = 64_000_000;
/// Service API version, reported by every projection.
pub const API_VERSION: &str = "ui-9";
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One engine or document diagnostic as the service reports it: no machining
/// rules live here, only the projection of [`cam_core::geometry::Diagnostic`].
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDiagnostic {
    pub code: String,
    pub severity: cam_core::geometry::Severity,
    pub stage: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}
impl From<cam_core::geometry::Diagnostic> for UiDiagnostic {
    fn from(d: cam_core::geometry::Diagnostic) -> Self {
        Self {
            code: d.code,
            severity: d.severity,
            stage: d.stage.into(),
            message: d.message,
            source_id: d.source_id,
        }
    }
}
