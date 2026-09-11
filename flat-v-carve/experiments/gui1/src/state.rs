use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FIELDS: [&str; 40] = [
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
    "Optional probe",
    "Tool name / IME probe",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub schema: u32,
    pub sources: Vec<u64>,
    pub selected_source: u64,
    pub operation: u64,
    // String keys encode stable source/operation/field IDs, never row positions.
    pub raw: BTreeMap<String, String>,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            schema: 1,
            sources: vec![101, 202],
            selected_source: 101,
            operation: 1,
            raw: BTreeMap::new(),
        }
    }
}
impl Draft {
    pub fn key(&self, field: usize) -> String {
        format!(
            "{}/{}/{}",
            self.selected_source, self.operation, FIELDS[field]
        )
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
            return Err("Recovery exceeds 1 MB spike budget".into());
        }
        let draft: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if draft.schema != 1
            || draft.sources.len() != 2
            || !draft.sources.contains(&101)
            || !draft.sources.contains(&202)
            || !draft.sources.contains(&draft.selected_source)
            || !(1..=1000).contains(&draft.operation)
        {
            return Err("Unsupported recovery identity/schema".into());
        }
        for (key, text) in &draft.raw {
            let parts: Vec<_> = key.splitn(3, '/').collect();
            if parts.len() != 3
                || !matches!(parts[0], "101" | "202")
                || parts[1]
                    .parse::<u64>()
                    .ok()
                    .filter(|v| (1..=1000).contains(v))
                    .is_none()
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
    fn partial_text_survives_reorder_navigation_and_recovery() {
        let mut d = Draft::default();
        let key = d.key(0);
        d.raw.insert(key.clone(), "-".into());
        d.sources.reverse();
        d.operation = 2;
        d.selected_source = 202;
        assert!(!d.raw.contains_key(&d.key(0)));
        let mut restored = Draft::recover(&serde_json::to_string(&d).unwrap()).unwrap();
        restored.operation = 1;
        restored.selected_source = 101;
        assert_eq!(restored.raw[&restored.key(0)], "-");
        for text in ["-", "1.", "paste invalid", "NaN", "inf"] {
            assert!(Draft::parse(text).is_err());
        }
        assert_eq!(Draft::parse(""), Ok(None));
        assert_eq!(Draft::parse("1.2"), Ok(Some(1.2)));
    }
}
