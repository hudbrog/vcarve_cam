//! Operation lifecycle for the ordered workspace.
//!
//! The edits themselves live in the shared schema-5 document commands
//! ([`cam_core::project::v5::commands`]), so the UI applies operation
//! ordering, enablement and creation exactly like any other document command
//! and never keeps a second validator. This module only binds the GUI's
//! intent (which kind to add, under which stable ID) to those commands.
use cam_core::project::v5::{self, CamJobV5};
use serde::{Deserialize, Serialize};

/// Bound on the ordered operation list this workspace edits. The planner has
/// its own stage/motion budget; this limit keeps the navigator and one
/// generated prefix inside the reviewed display envelope.
pub const MAX_OPERATIONS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Face,
    FlatVcarve,
    DragKnife,
}

impl Kind {
    pub fn new_operation_kind(self) -> v5::commands::NewOperationKind {
        match self {
            Self::Face => v5::commands::NewOperationKind::Face,
            Self::FlatVcarve => v5::commands::NewOperationKind::FlatVcarve,
            Self::DragKnife => v5::commands::NewOperationKind::DragKnife,
        }
    }
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::FlatVcarve => "carving",
            Self::DragKnife => "knife",
        }
    }
    pub fn default_name(self) -> &'static str {
        self.new_operation_kind().default_name()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Add {
        kind: Kind,
        operation_id: String,
        name: String,
    },
    Delete {
        operation_id: String,
    },
    SetEnabled {
        operation_id: String,
        enabled: bool,
    },
    Move {
        operation_id: String,
        to_index: usize,
    },
    Rename {
        operation_id: String,
        name: String,
    },
}

pub fn empty_job() -> CamJobV5 {
    CamJobV5 {
        schema_version: 5,
        name: "New job".into(),
        setup: Default::default(),
        artwork: vec![],
        tools: vec![],
        operations: vec![],
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    }
}

/// A stable, unused operation ID for one new operation.
pub fn next_id(job: &CamJobV5, kind: Kind) -> String {
    let prefix = kind.id_prefix();
    let mut index = 1;
    loop {
        let candidate = format!("{prefix}-{index}");
        if !job
            .operations
            .iter()
            .any(|operation| operation.id == candidate)
        {
            return candidate;
        }
        index += 1;
    }
}

/// The add action for one kind, with the next free ID and the canonical name.
pub fn add(kind: Kind, job: &CamJobV5) -> Action {
    Action::Add {
        kind,
        operation_id: next_id(job, kind),
        name: kind.default_name().into(),
    }
}

/// Apply one operation-list edit. Every returned document is structurally
/// valid; a rejected edit leaves the caller's document untouched.
pub fn apply(job: &CamJobV5, action: Action) -> Result<CamJobV5, String> {
    fn message(error: cam_core::geometry::Diagnostic) -> String {
        error.to_string()
    }
    let outcome = match action {
        Action::Add {
            kind,
            operation_id,
            name,
        } => {
            if job.operations.len() >= MAX_OPERATIONS {
                return Err(format!(
                    "This workspace orders up to {MAX_OPERATIONS} operations; delete one before adding another"
                ));
            }
            v5::commands::add_operation(job, kind.new_operation_kind(), &operation_id, &name)
                .map_err(message)?
        }
        Action::Delete { operation_id } => {
            v5::commands::remove_operation(job, &operation_id).map_err(message)?
        }
        Action::SetEnabled {
            operation_id,
            enabled,
        } => v5::commands::set_operation_enabled(job, &operation_id, enabled).map_err(message)?,
        Action::Move {
            operation_id,
            to_index,
        } => v5::commands::move_operation(job, &operation_id, to_index).map_err(message)?,
        Action::Rename { operation_id, name } => {
            v5::commands::rename_operation(job, &operation_id, &name).map_err(message)?
        }
    };
    Ok(outcome.job)
}
