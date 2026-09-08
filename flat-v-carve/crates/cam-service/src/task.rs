//! Task vocabulary shared by every service adapter.
use serde::{Deserialize, Serialize};

/// Endmill-only or the combined endmill/V-bit pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Endmill,
    Combined,
}

/// Motions per preview page; the first result page uses the same bound.
pub const PAGE_MOTIONS: usize = 20_000;

/// Retained report JSON bound shared by verification and export adapters.
pub const REPORT_BYTES: usize = 16_000_000;
