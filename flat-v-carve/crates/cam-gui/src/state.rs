use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FIELDS: [&str; 47] = [
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
];

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
            schema: 2,
            artwork_item: job.artwork[0].id.0.clone(),
            operation: job.operations[0].id.clone(),
            raw: BTreeMap::new(),
        }
    }
    pub fn key(&self, field: usize) -> String {
        format!("{}/{}/{}", self.artwork_item, self.operation, FIELDS[field])
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
        if draft.schema != 2
            || !cam_core::preview::valid_id(&draft.artwork_item)
            || !cam_core::preview::valid_id(&draft.operation)
        {
            return Err("Unsupported recovery identity/schema".into());
        }
        for (key, text) in &draft.raw {
            let parts: Vec<_> = key.splitn(3, '/').collect();
            if parts.len() != 3
                || parts[0] != draft.artwork_item
                || parts[1] != draft.operation
                || !FIELDS.contains(&parts[2])
                || text.chars().count() > 4096
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
        draft.operation = "another-operation".into();
        assert!(Draft::recover(&serde_json::to_string(&draft).unwrap()).is_err());
        for text in ["-", "1.", "invalid", "NaN", "inf"] {
            assert!(Draft::parse(text).is_err());
        }
        assert_eq!(Draft::parse(""), Ok(None));
        assert_eq!(Draft::parse("1.2"), Ok(Some(1.2)));
    }
}
