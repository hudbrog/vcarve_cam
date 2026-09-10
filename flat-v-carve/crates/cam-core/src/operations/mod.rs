//! Typed operation dispatch. Each planner receives the canonical job and one
//! enabled operation, and returns locally-indexed stages, motions and
//! diagnostics. Only planners that ship in a slice are advertised here.
use crate::{
    geometry::{Diagnostic, Result},
    project::{CamJob, Operation},
    sequence::PlannedOperation,
};

pub mod face;
pub mod flat_vcarve;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocatedDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    /// e.g. `operations[op-id].endmill.spindle_rpm`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
}
impl LocatedDiagnostic {
    pub fn missing(operation_id: &str, field_path: &str, message: impl Into<String>) -> Self {
        Self {
            code: "MISSING_MACHINING_SETTING".into(),
            message: message.into(),
            operation_id: Some(operation_id.into()),
            tool_id: None,
            field_path: Some(field_path.into()),
        }
    }
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(&self.code, self.message.clone()).at_stage("operations")
    }
}

/// A face plane published by a preceding face operation (plan section 6.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PublishedFace {
    pub z_mm: f64,
    pub covered: crate::project::RectXY,
}

pub(crate) fn plan_operation(
    job: &CamJob,
    operation: &Operation,
    published_faces: &std::collections::BTreeMap<String, PublishedFace>,
    prior_motions: &[crate::toolpath::PlannedMotion],
) -> Result<PlannedOperation> {
    match &operation.settings {
        crate::project::OperationSettings::FlatVcarve(settings) => {
            flat_vcarve::plan(job, &operation.id, settings, published_faces, prior_motions)
        }
        // The dispatcher never silently skips an unsupported operation; the
        // sequence planner rejects them with OPERATION_PLANNER_UNAVAILABLE.
        crate::project::OperationSettings::Face(settings) => {
            face::plan(job, &operation.id, settings)
        }
        crate::project::OperationSettings::Profile(_)
        | crate::project::OperationSettings::DragKnife(_) => Err(Diagnostic::new(
            "OPERATION_PLANNER_UNAVAILABLE",
            "operation planner ships in a later slice",
        )
        .at_stage("operations")),
    }
}
