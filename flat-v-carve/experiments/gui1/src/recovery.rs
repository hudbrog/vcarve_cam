//! Experimental recovery envelope. Persist editable inputs, never derived trust.
use crate::state::Draft;
use serde::{Deserialize, Serialize};
pub const MAX_BYTES: usize = 9_000_000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub draft: Draft,
    pub job: Option<String>,
}
impl Snapshot {
    pub fn new(draft: Draft, job: Option<String>) -> Self {
        Self {
            schema: 1,
            draft,
            job,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err("Unsupported session recovery schema".into());
        }
        Draft::recover(&serde_json::to_string(&self.draft).map_err(|e| e.to_string())?)?;
        if let Some(job) = &self.job {
            if job.len() > 8_000_000 {
                return Err("Recovery job exceeds 8 MB".into());
            }
            cam_core::job::Job::from_json(job).map_err(|e| format!("Invalid recovery job: {e}"))?;
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
