//! Reviewable starting values; geometry is not a material cutting profile.
use cam_core::project::{ToolGeometry, v5::CamJobV5};

pub struct Suggestion {
    pub field: usize,
    pub value: f64,
    pub reason: &'static str,
}

pub fn suggestions(job: &CamJobV5) -> Vec<Suggestion> {
    let Some(s) = crate::knife::settings(job) else {
        return vec![];
    };
    let Some(ToolGeometry::DragKnife(tool)) = job
        .tools
        .iter()
        .find(|t| t.id == s.assignment.tool_id)
        .and_then(|t| t.geometry.as_ref())
    else {
        return vec![];
    };
    let limit = tool.max_cut_depth_mm.min(
        s.assignment
            .max_stepdown_mm
            .unwrap_or(tool.max_cut_depth_mm),
    );
    let pass = s.stepdown_mm.unwrap_or(limit).min(limit);
    [
        (
            66,
            limit,
            "Knife capacity; confirm the material's per-pass limit",
        ),
        (
            67,
            limit,
            "Smaller of knife capacity and the cutting profile's stepdown limit",
        ),
        (
            68,
            pass * 0.1,
            "Starting suggestion: 10% of the effective pass depth; adjust for the material",
        ),
        (
            69,
            20.,
            "Starting suggestion: lift for turns of 20° or more; adjust for the material",
        ),
    ]
    .into_iter()
    .filter(|(field, _, _)| crate::knife::value(job, *field).is_none())
    .map(|(field, value, reason)| Suggestion {
        field,
        value,
        reason,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suggestions_use_assigned_knife_and_profile_without_changing_explicit_values() {
        let mut job =
            crate::session::open(include_str!("../../../fixtures/gui6/knife.job.json")).unwrap();
        crate::knife::set(&mut job, 67, None).unwrap();
        crate::knife::set(&mut job, 68, None).unwrap();
        crate::knife::set(&mut job, 69, None).unwrap();
        let values = suggestions(&job);
        assert_eq!(
            values
                .iter()
                .map(|v| (v.field, v.value))
                .collect::<Vec<_>>(),
            vec![(67, 1.), (68, 0.1), (69, 20.)]
        );
        crate::knife::set(&mut job, 67, Some(0.3)).unwrap();
        assert_eq!(suggestions(&job)[0].value, 0.03);
        crate::knife::set(&mut job, 68, Some(0.05)).unwrap();
        crate::knife::set(&mut job, 69, Some(35.)).unwrap();
        assert!(suggestions(&job).is_empty());
        assert_eq!(
            crate::knife::settings(&job)
                .unwrap()
                .alignment
                .initial_heading_deg,
            Some(180.)
        );
    }
}
