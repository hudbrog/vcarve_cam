use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FIELDS: [&str; 109] = [
    "Maximum depth",
    "Wall allowance",
    "Roughing feed",
    "Finishing feed",
    "Floor ridge",
    "Detail residual",
    "Stock thickness",
    "Clearance",
    "Stepdown",
    "Stepover",
    "Plunge feed",
    "Spindle speed",
    "Endmill diameter",
    "Cutting length",
    "Ramp angle",
    "Entry length",
    "V-bit angle",
    "Tip diameter",
    "Cutting diameter",
    "Cutting height",
    "Finish stepdown",
    "Finish plunge",
    "Finish spindle",
    "Motion tolerance",
    "Geometry tolerance",
    "Verification tolerance",
    "Origin X",
    "Origin Y",
    "Rotation",
    "Scale",
    "Start X",
    "Start Y",
    "Tool number",
    "Length offset",
    "Spinup delay",
    "Output precision",
    "Blend tolerance",
    "Recovery interval",
    "V-bit tool number",
    "V-bit length offset",
    "Stock minimum X",
    "Stock minimum Y",
    "Stock width",
    "Stock length",
    "Work zero X",
    "Work zero Y",
    "Finish stepover",
    "Top offset",
    "Rough layer limit",
    "Rough loop limit",
    "Rough motion limit",
    "Ramp feed",
    "Finish path limit",
    "Finish motion limit",
    "Curve segment limit",
    "Depth pass limit",
    "Cleanup iterations",
    "Quality sample spacing",
    "Quality sample limit",
    "Reachability cell limit",
    "Stock slices",
    "Blade offset",
    "Blade cutting depth",
    "Knife cutting feed",
    "Knife plunge feed",
    "Swivel feed",
    "Knife tool stepdown",
    "Knife pass stepdown",
    "Swivel depth",
    "Corner threshold",
    "Through-cut allowance",
    "Closure overlap",
    "Initial heading",
    "Knife top offset",
    "Knife bottom offset",
    "Face pass angle",
    "Face entry overrun",
    "Face exit overrun",
    "Face margin min X",
    "Face margin max X",
    "Face margin min Y",
    "Face margin max Y",
    "Face area min X",
    "Face area min Y",
    "Face area width",
    "Face area length",
    "Face top offset",
    "Face bottom offset",
    "Tool stepdown limit",
    "Profile top offset",
    "Profile bottom offset",
    "Finish allowance",
    "Finish feed",
    "Tab height",
    "Tab width",
    "Tab count",
    "Tab spacing",
    "Tab anchor fraction",
    "Entry ramp angle",
    "Entry ramp feed",
    "Lead-in length",
    "Lead-in feed",
    "Lead-in radius",
    "Lead-in sweep",
    "Lead-out length",
    "Lead-out feed",
    "Lead-out radius",
    "Lead-out sweep",
    "Start fraction",
];

/// Fields whose raw text belongs to one qualified geometry reference rather
/// than to the job: a profile start anchor and a manual tab anchor are
/// addressed by the contour they parameterize, so reselecting or reordering a
/// row never transfers its text to another anchor.
pub const ANCHOR_FIELDS: [usize; 2] = [97, 108];

pub fn is_placement(field: usize) -> bool {
    matches!(field, 26..=29)
}

pub fn is_anchor(field: usize) -> bool {
    ANCHOR_FIELDS.contains(&field)
}

/// Split a draft key into its scope, operation and field label. Every key the
/// store writes has exactly this three-part shape.
pub fn scope_of(key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = key.splitn(3, '/');
    let scope = parts.next()?;
    let operation = parts.next()?;
    let label = parts.next()?;
    (!scope.is_empty() && !label.is_empty()).then_some((scope, operation, label))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub schema: u32,
    pub artwork_item: String,
    pub operation: String,
    // String keys encode stable source/operation/field IDs, never row positions.
    pub raw: BTreeMap<String, String>,
}
impl Draft {
    pub fn for_job(job: &cam_core::project::v5::CamJobV5) -> Self {
        Self {
            schema: 3,
            artwork_item: job
                .artwork
                .first()
                .map(|i| i.id.0.clone())
                .unwrap_or_default(),
            operation: job
                .operations
                .first()
                .map(|o| o.id.clone())
                .unwrap_or_default(),
            raw: BTreeMap::new(),
        }
    }
    pub fn key(&self, field: usize) -> String {
        self.key_for(&self.artwork_item, field)
    }
    pub fn key_for(&self, item: &str, field: usize) -> String {
        format!(
            "{}/{}/{}",
            if matches!(field, 26..=29) {
                item
            } else {
                "job"
            },
            self.operation,
            FIELDS[field]
        )
    }
    /// Key of a field whose text belongs to one qualified geometry reference
    /// (a profile start or manual tab anchor) instead of the job.
    pub fn key_scoped(&self, scope: &str, field: usize) -> String {
        format!("{}/{}/{}", scope, self.operation, FIELDS[field])
    }
    pub fn validate_job(&self, job: &cam_core::project::v5::CamJobV5) -> Result<(), String> {
        // The selected operation is editor state: it must be a valid ID, and
        // keys may keep drafts for operations this document no longer carries
        // (a deleted operation's incomplete text stays recoverable until the
        // user discards it). Placement drafts are stricter: they must belong
        // to artwork that exists right now.
        if (!self.operation.is_empty() && !cam_core::preview::valid_id(&self.operation))
            || (!self.artwork_item.is_empty()
                && !job.artwork.iter().any(|i| i.id.0 == self.artwork_item))
        {
            return Err("Draft identity does not match the job".into());
        }
        for key in self.raw.keys() {
            let parts: Vec<_> = key.splitn(3, '/').collect();
            if parts.len() != 3 {
                return Err("Invalid draft key".into());
            }
            let Some(field) = FIELDS.iter().position(|name| *name == parts[2]) else {
                return Err("Invalid draft key".into());
            };
            if !parts[1].is_empty() && !cam_core::preview::valid_id(parts[1]) {
                return Err("Draft field belongs to another artwork item".into());
            }
            // Placement text belongs to one artwork item; anchor text belongs
            // to the qualified contour it parameterizes, whose owner must be a
            // live item; everything else is job-scoped.
            let owner = if is_placement(field) {
                Some(parts[0].to_string())
            } else if is_anchor(field) {
                cam_core::project::v5::artwork::parse_wire_id(parts[0])
                    .map(|pick| pick.artwork_item_id.0)
            } else {
                (parts[0] == "job").then(String::new)
            };
            let Some(owner) = owner else {
                return Err("Draft field belongs to another artwork item".into());
            };
            if !owner.is_empty() && !job.artwork.iter().any(|item| item.id.0 == owner) {
                return Err("Draft field belongs to another artwork item".into());
            }
        }
        Ok(())
    }
    pub fn parse(text: &str) -> Result<Option<f64>, &'static str> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.trim().ends_with('.') {
            return Err("Incomplete number — retained");
        }
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(Some)
            .ok_or("Invalid number — retained")
    }
    pub fn recover(text: &str) -> Result<Self, String> {
        if text.len() > 1_000_000 {
            return Err("Recovery exceeds 1 MB input budget".into());
        }
        let draft: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if draft.schema != 3
            || (!draft.artwork_item.is_empty() && !cam_core::preview::valid_id(&draft.artwork_item))
            || (!draft.operation.is_empty() && !cam_core::preview::valid_id(&draft.operation))
        {
            return Err("Unsupported recovery identity/schema".into());
        }
        for (key, text) in &draft.raw {
            let parts: Vec<_> = key.splitn(3, '/').collect();
            if parts.len() != 3
                || (!parts[1].is_empty() && !cam_core::preview::valid_id(parts[1]))
                || text.chars().count() > 4096
            {
                return Err("Unsupported recovery field identity or text limit".into());
            }
            let Some(field) = FIELDS.iter().position(|name| *name == parts[2]) else {
                return Err("Unsupported recovery field identity or text limit".into());
            };
            let scoped = is_placement(field) || is_anchor(field);
            if (is_anchor(field)
                && cam_core::project::v5::artwork::parse_wire_id(parts[0]).is_none())
                || (!scoped && parts[0] != "job")
                || (!is_anchor(field) && !cam_core::preview::valid_id(parts[0]))
            {
                return Err("Unsupported recovery field identity or text limit".into());
            }
        }
        Ok(draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_fields_use_real_document_identity_and_reject_foreign_keys() {
        let job = crate::session::open(crate::session::FLOWER).unwrap();
        let mut draft = Draft::for_job(&job);
        draft.raw.insert(draft.key(0), "-".into());
        let recovered = Draft::recover(&serde_json::to_string(&draft).unwrap()).unwrap();
        assert_eq!(recovered, draft);
        // A draft may outlive the operation it belongs to (deleting an
        // operation keeps its recoverable text), but never a malformed ID.
        draft.operation = "another-operation".into();
        assert!(Draft::recover(&serde_json::to_string(&draft).unwrap()).is_ok());
        draft.operation = "not a valid id".into();
        assert!(Draft::recover(&serde_json::to_string(&draft).unwrap()).is_err());
        for text in ["-", "1.", "invalid", "NaN", "inf"] {
            assert!(Draft::parse(text).is_err());
        }
        assert_eq!(Draft::parse(""), Ok(None));
        assert_eq!(Draft::parse("1.2"), Ok(Some(1.2)));
    }
}
