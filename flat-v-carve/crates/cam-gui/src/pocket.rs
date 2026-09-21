//! Pocket numeric bindings. Shared draft/undo/recovery owns all text editing.
use cam_core::project::{
    LeadSpec,
    v5::{CamJobV5, OperationSettingsV5, PocketEntry, PocketSettingsV5},
};

pub fn settings<'a>(job: &'a CamJobV5, id: &str) -> Option<&'a PocketSettingsV5> {
    match &crate::session::operation(job, id)?.settings {
        OperationSettingsV5::Pocket(s) => Some(s),
        _ => None,
    }
}
pub fn settings_mut<'a>(
    job: &'a mut CamJobV5,
    id: &str,
) -> Result<&'a mut PocketSettingsV5, String> {
    match &mut crate::session::operation_mut(job, id)
        .ok_or("Pocket operation missing")?
        .settings
    {
        OperationSettingsV5::Pocket(s) => Ok(s),
        _ => Err("Expected a pocket operation".into()),
    }
}
pub fn active(field: usize) -> bool {
    matches!(field, 1 | 2 | 6..=13 | 23 | 25..=36 | 40..=45 | 48..=50 | 92 | 98..=107 | 123..=125)
}
fn lead_value(s: &LeadSpec, field: usize) -> Option<f64> {
    match (s, field) {
        (LeadSpec::TangentLine { length_mm, .. }, 0) => *length_mm,
        (
            LeadSpec::TangentLine { feed_mm_min, .. } | LeadSpec::TangentArc { feed_mm_min, .. },
            1,
        ) => *feed_mm_min,
        (LeadSpec::TangentArc { radius_mm, .. }, 2) => *radius_mm,
        (LeadSpec::TangentArc { sweep_deg, .. }, 3) => *sweep_deg,
        _ => None,
    }
}
pub fn value(job: &CamJobV5, id: &str, field: usize) -> Option<f64> {
    let s = settings(job, id)?;
    match field {
        1 => s.wall_allowance_mm,
        2 => s.assignment.cutting_feed_mm_min,
        8 => s.assignment.max_stepdown_mm,
        9 => s.assignment.stepover_mm,
        10 => s.assignment.plunge_feed_mm_min,
        11 => s.assignment.spindle_rpm,
        48 => Some(s.limits.max_layers as f64),
        49 => Some(s.limits.max_loops_per_layer as f64),
        50 => Some(s.limits.max_motions as f64),
        92 => s.finish_feed_mm_min,
        98 => match s.entry {
            PocketEntry::Ramp { max_angle_deg, .. } | PocketEntry::Helix { max_angle_deg, .. } => {
                max_angle_deg
            }
            _ => None,
        },
        99 => match s.entry {
            PocketEntry::Ramp { feed_mm_min, .. } | PocketEntry::Helix { feed_mm_min, .. } => {
                feed_mm_min
            }
            _ => None,
        },
        100..=103 => lead_value(&s.lead_in, field - 100),
        104..=107 => lead_value(&s.lead_out, field - 104),
        123 => Some(s.top.offset_mm),
        124 => Some(s.bottom.offset_mm),
        125 => match s.entry {
            PocketEntry::Helix { radius_mm, .. } => radius_mm,
            _ => None,
        },
        _ => crate::authoring::value_in(job, id, field),
    }
}
fn set_lead(s: &mut LeadSpec, field: usize, v: Option<f64>) -> Result<(), String> {
    let slot = match (s, field) {
        (LeadSpec::TangentLine { length_mm, .. }, 0) => length_mm,
        (
            LeadSpec::TangentLine { feed_mm_min, .. } | LeadSpec::TangentArc { feed_mm_min, .. },
            1,
        ) => feed_mm_min,
        (LeadSpec::TangentArc { radius_mm, .. }, 2) => radius_mm,
        (LeadSpec::TangentArc { sweep_deg, .. }, 3) => sweep_deg,
        _ => return Err("Choose the corresponding lead shape first".into()),
    };
    *slot = v;
    Ok(())
}
pub fn set(job: &mut CamJobV5, id: &str, field: usize, v: Option<f64>) -> Result<(), String> {
    if field == 6 {
        job.setup.stock.thickness_mm = v;
        return Ok(());
    }
    if !matches!(field, 1 | 2 | 8..=11 | 48..=50 | 92 | 98..=107 | 123..=125) {
        return crate::authoring::set_in(job, id, field, v);
    }
    let s = settings_mut(job, id)?;
    match field {
        1 => s.wall_allowance_mm = v,
        2 => s.assignment.cutting_feed_mm_min = v,
        8 => s.assignment.max_stepdown_mm = v,
        9 => s.assignment.stepover_mm = v,
        10 => s.assignment.plunge_feed_mm_min = v,
        11 => s.assignment.spindle_rpm = v,
        48..=50 => {
            let n = v
                .filter(|v| v.is_finite() && *v >= 1. && *v <= 1_000_000. && v.fract() == 0.)
                .ok_or("Enter a positive whole-number limit")? as usize;
            match field {
                48 => s.limits.max_layers = n,
                49 => s.limits.max_loops_per_layer = n,
                _ => s.limits.max_motions = n,
            }
        }
        92 => s.finish_feed_mm_min = v,
        98 => match &mut s.entry {
            PocketEntry::Ramp { max_angle_deg, .. } | PocketEntry::Helix { max_angle_deg, .. } => {
                *max_angle_deg = v
            }
            _ => return Err("Choose ramp or helix first".into()),
        },
        99 => match &mut s.entry {
            PocketEntry::Ramp { feed_mm_min, .. } | PocketEntry::Helix { feed_mm_min, .. } => {
                *feed_mm_min = v
            }
            _ => return Err("Choose ramp or helix first".into()),
        },
        100..=103 => set_lead(&mut s.lead_in, field - 100, v)?,
        104..=107 => set_lead(&mut s.lead_out, field - 104, v)?,
        123 => s.top.offset_mm = v.ok_or("Enter a top offset")?,
        124 => s.bottom.offset_mm = v.ok_or("Enter a bottom offset")?,
        125 => match &mut s.entry {
            PocketEntry::Helix { radius_mm, .. } => *radius_mm = v,
            _ => return Err("Choose helix first".into()),
        },
        _ => unreachable!(),
    }
    Ok(())
}
