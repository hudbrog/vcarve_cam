//! Typed operation dispatch. Each planner receives the canonical job and one
//! enabled operation, and returns locally-indexed stages, motions and
//! diagnostics. Only planners that ship in a slice are advertised here.
use crate::{
    contours::ContourCatalogue,
    geometry::{Diagnostic, Result},
    project::{CamJob, Operation},
    sequence::PlannedOperation,
};

pub mod drag_knife;
pub mod face;
pub mod flat_vcarve;
pub mod profile;

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

/// Face planes established by preceding face operations, keyed by operation
/// ID; later height references resolve against them during one planning run.
pub(crate) type PublishedFaceMap = std::collections::BTreeMap<String, PublishedFace>;

/// One job tool as planners see it: pure geometry and capabilities. Schema-4
/// `JobTool` and schema-5 `JobToolV5` project onto this view, so a planner
/// body cannot tell (and never needs to know) which document owns the tools.
pub(crate) struct PlanTool<'a> {
    pub id: &'a str,
    pub geometry: Option<&'a crate::project::ToolGeometry>,
    pub capabilities: &'a crate::project::ToolCapabilities,
}

/// The non-geometric job inputs every planner shares: setup, tolerances and
/// the job tools (plan section 22.4). The schema-4 dispatch and the H3
/// collection planner each construct one view; planner bodies run unchanged
/// over either.
pub(crate) struct PlanContext<'a> {
    pub setup: &'a crate::project::SetupSettings,
    pub tolerances: &'a crate::job::PlanningTolerances,
    pub tools: Vec<PlanTool<'a>>,
    /// Whether any artwork exists to select from: an attached source in
    /// schema 4, at least one artwork item in schema 5. An empty selection
    /// is a per-operation missing value either way.
    pub has_artwork: bool,
}

impl<'a> PlanContext<'a> {
    pub(crate) fn from_v4(job: &'a CamJob) -> Self {
        Self {
            setup: &job.setup,
            tolerances: &job.tolerances,
            tools: job
                .tools
                .iter()
                .map(|tool| PlanTool {
                    id: tool.id.as_str(),
                    geometry: tool.geometry.as_ref(),
                    capabilities: &tool.capabilities,
                })
                .collect(),
            has_artwork: job.source.is_some(),
        }
    }

    pub(crate) fn from_v5(job: &'a crate::project::v5::CamJobV5) -> Self {
        Self {
            setup: &job.setup,
            tolerances: &job.tolerances,
            tools: job
                .tools
                .iter()
                .map(|tool| PlanTool {
                    id: tool.id.as_str(),
                    geometry: tool.geometry.as_ref(),
                    capabilities: &tool.capabilities,
                })
                .collect(),
            has_artwork: !job.artwork.is_empty(),
        }
    }

    pub(crate) fn tool(&self, id: &str) -> Option<&PlanTool<'a>> {
        self.tools.iter().find(|tool| tool.id == id)
    }
}

/// How a planner obtains its selected geometry: import the single attached
/// source (schema 4) or consume the assembled owner-qualified catalogue
/// (schema 5 collection). Resolution happens inside the planner after its
/// cheap missing-field checks, exactly where the catalogue was built before.
pub(crate) enum PlannerGeometry<'a> {
    SourceJob(&'a CamJob),
    Catalogue(&'a ContourCatalogue),
}

impl PlannerGeometry<'_> {
    pub(crate) fn catalogue(&self) -> Result<ContourCatalogue> {
        match self {
            PlannerGeometry::SourceJob(job) => ContourCatalogue::build(job),
            PlannerGeometry::Catalogue(catalogue) => Ok((*catalogue).clone()),
        }
    }
}

pub(crate) fn plan_operation(
    job: &CamJob,
    operation: &Operation,
    published_faces: &std::collections::BTreeMap<String, PublishedFace>,
    prior_motions: &[crate::toolpath::PlannedMotion],
) -> Result<PlannedOperation> {
    let ctx = PlanContext::from_v4(job);
    match &operation.settings {
        crate::project::OperationSettings::FlatVcarve(settings) => flat_vcarve::plan(
            job,
            &ctx,
            &operation.id,
            settings,
            published_faces,
            prior_motions,
        ),
        // The dispatcher never silently skips an unsupported operation; the
        // sequence planner rejects them with OPERATION_PLANNER_UNAVAILABLE.
        crate::project::OperationSettings::Face(settings) => {
            face::plan(&ctx, &operation.id, settings)
        }
        crate::project::OperationSettings::Profile(settings) => profile::plan(
            &ctx,
            &operation.id,
            settings,
            published_faces,
            &PlannerGeometry::SourceJob(job),
        ),
        crate::project::OperationSettings::DragKnife(settings) => drag_knife::plan(
            &ctx,
            &operation.id,
            settings,
            published_faces,
            prior_motions,
            &PlannerGeometry::SourceJob(job),
        ),
    }
}
