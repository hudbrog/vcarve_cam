//! Recovery envelope. Persist editable inputs, never derived trust.
use crate::state::Draft;
use serde::{Deserialize, Serialize};
pub const MAX_BYTES: usize = 9_000_000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub inspector: usize,
    pub inspector_width: f32,
    pub scroll: [f32; 6],
    pub search: String,
    pub simulate: bool,
    pub view: crate::viewport::ViewSettings,
    pub plan_fingerprint: Option<String>,
    pub saved_job_hash: Option<String>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            inspector: 2,
            inspector_width: 325.,
            scroll: [0.; 6],
            search: String::new(),
            simulate: false,
            view: Default::default(),
            plan_fingerprint: None,
            saved_job_hash: None,
        }
    }
}
impl Workspace {
    pub fn validate(&self) -> Result<(), String> {
        if self.inspector > 5
            || !self.inspector_width.is_finite()
            || !(240. ..=600.).contains(&self.inspector_width)
            || self
                .scroll
                .iter()
                .any(|n| !n.is_finite() || *n < 0. || *n > 100000.)
            || self.search.len() > 512
            || self
                .saved_job_hash
                .as_ref()
                .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            || self
                .plan_fingerprint
                .as_ref()
                .is_some_and(|s| s.len() > 128)
        {
            return Err("Invalid saved workspace".into());
        }
        self.view.validate()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub draft: Draft,
    pub job: Option<String>,
    pub finish_draft: Option<cam_core::vcarve::VBitPlanningSettings>,
    pub workspace: Workspace,
    pub undo: Vec<crate::app::Document>,
    pub redo: Vec<crate::app::Document>,
}
impl Snapshot {
    pub fn new(draft: Draft, job: Option<String>) -> Self {
        Self {
            schema: 3,
            draft,
            job,
            finish_draft: None,
            workspace: Default::default(),
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 3 {
            return Err("Unsupported session recovery schema".into());
        }
        Draft::recover(&serde_json::to_string(&self.draft).map_err(|e| e.to_string())?)?;
        if let Some(job) = &self.job {
            if job.len() > 8_000_000 {
                return Err("Recovery job exceeds 8 MB".into());
            }
            let job = crate::session::open(job)?;
            if self.draft.artwork_item != job.artwork[0].id.0
                || self.draft.operation != job.operations[0].id
            {
                return Err("Recovery fields belong to another document".into());
            }
        }
        self.workspace.validate()?;
        if let Some(finish) = &self.finish_draft {
            finish.validate().map_err(|e| e.to_string())?;
        }
        if self.undo.len() + self.redo.len() > 32 {
            return Err("Recovery history exceeds 32 transactions".into());
        }
        for doc in self.undo.iter().chain(&self.redo) {
            doc.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    pub revision: u64,
    pub snapshot: Snapshot,
}
impl Stored {
    pub fn decode(text: &str) -> Result<Self, String> {
        if text.len() > MAX_BYTES {
            return Err("Recovery exceeds 9 MB".into());
        }
        let record: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if record.revision == 0 || record.revision > 9_007_199_254_740_991 {
            return Err("Invalid recovery revision".into());
        }
        record.snapshot.validate()?;
        Ok(record)
    }
}

#[derive(Default)]
pub struct Tracker {
    pub enabled: bool,
    pub ready: bool,
    pub revision: Option<u64>,
    pub edit: u64,
    pub saved_edit: u64,
    pub pending: Option<u64>,
    pub last_edit: f64,
    pub failed: bool,
    pub status: String,
    pub offered: Option<Stored>,
}
impl Tracker {
    pub fn changed(&mut self, now: f64) {
        self.edit += 1;
        self.last_edit = now;
    }
    pub fn due(&self, now: f64) -> bool {
        self.enabled
            && self.ready
            && !self.failed
            && self.offered.is_none()
            && self.pending.is_none()
            && self.edit != self.saved_edit
            && now - self.last_edit >= 0.75
    }
    pub fn written(&mut self, edit: u64, result: Result<u64, String>) {
        if self.pending != Some(edit) {
            return;
        }
        self.pending = None;
        match result {
            Ok(revision) => {
                self.revision = Some(revision);
                self.saved_edit = edit;
                self.status = format!("Recovery saved · revision {revision}");
            }
            Err(error) => {
                self.failed = true;
                self.status = format!("Recovery stopped: {error}. Current edits are retained.");
            }
        }
    }
}
