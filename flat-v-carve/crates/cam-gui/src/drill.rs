//! Drill-operation editor binding: the typed fields between the panel and
//! the canonical [`DrillSettingsV5`]. The planner owns every machining
//! decision; this module only reads and writes document fields, so a
//! half-configured drill never looks ready. Shared assignment ids (2 = cutting
//! feed, 10 = plunge feed, 11 = spindle speed) bind exactly as they do for the
//! other milling kinds — a drill's cutting feed is its plunge feed.
use cam_core::project::v5::{CamJobV5, DrillSettingsV5};
use cam_core::project::{
    DrillDepthReference, DrillHoleOrder, DrillPeckMode, DrillPeckSettings, HeightReference,
};

/// Fields this editor binds for a Drill operation.
pub fn active(field: usize) -> bool {
    matches!(field, 2 | 6 | 7 | 10 | 11 | 23 | 25..=36 | 40..=45 | 111..=122)
}

pub fn value(job: &CamJobV5, operation_id: &str, field: usize) -> Option<f64> {
    let s = crate::session::drill(job, operation_id);
    match field {
        2 | 10 => s?.assignment.plunge_feed_mm_min,
        11 => s?.assignment.spindle_rpm,
        120..=122 => match &crate::authoring::tool_in(job, operation_id, false)?.geometry {
            Some(cam_core::project::ToolGeometry::Drill(g)) => Some(match field {
                120 => g.diameter_mm,
                121 => g.cutting_length_mm,
                _ => g.tip_angle_deg,
            }),
            _ => None,
        },
        111 => Some(s?.top.offset_mm),
        112 => Some(s?.bottom.offset_mm),
        113 => Some(s?.retract_height.offset_mm),
        114 => s?.breakthrough_extra_mm,
        115 => s?.dwell_at_bottom_s,
        116 => s?.peck.as_ref().map(|p| p.depth_mm),
        117 => s?.peck.as_ref().map(|p| p.reduction_mm),
        118 => s?.peck.as_ref().map(|p| p.min_depth_mm),
        119 => s?.peck.as_ref().and_then(|p| p.retract_mm),
        _ => crate::authoring::value_in(job, operation_id, field),
    }
}

pub fn set(
    job: &mut CamJobV5,
    operation_id: &str,
    field: usize,
    v: Option<f64>,
) -> Result<(), String> {
    if field == 6 {
        job.setup.stock.thickness_mm = v;
        return Ok(());
    }
    if !matches!(field, 2 | 10 | 11 | 111..=119) {
        return crate::authoring::set_in(job, operation_id, field, v);
    }
    let s = drill_mut(job, operation_id).ok_or("Expected a Drill operation")?;
    match field {
        2 | 10 => s.assignment.plunge_feed_mm_min = v,
        11 => s.assignment.spindle_rpm = v,
        111 => s.top.offset_mm = v.ok_or("Top offset cannot be unset")?,
        112 => s.bottom.offset_mm = v.ok_or("Bottom offset cannot be unset")?,
        113 => s.retract_height.offset_mm = v.ok_or("Retract offset cannot be unset")?,
        114 => s.breakthrough_extra_mm = v,
        115 => s.dwell_at_bottom_s = v,
        116 => peck_mut(s).depth_mm = v.ok_or("Peck depth cannot be unset")?,
        117 => peck_mut(s).reduction_mm = v.ok_or("Peck reduction cannot be unset")?,
        118 => peck_mut(s).min_depth_mm = v.ok_or("Minimum peck depth cannot be unset")?,
        119 => peck_mut(s).retract_mm = v,
        _ => return Err("Unknown drill field".into()),
    }
    Ok(())
}

/// The peck settings, creating them with the panel's defaults when the first
/// peck field is edited (FreeCAD's default peck depth is 0.75× the drill
/// diameter; the retract matches LinuxCNC's fixed G73 clearance).
fn peck_mut(settings: &mut DrillSettingsV5) -> &mut DrillPeckSettings {
    settings.peck.get_or_insert(DrillPeckSettings {
        mode: DrillPeckMode::FullRetract,
        depth_mm: 3.,
        reduction_mm: 0.,
        min_depth_mm: 1.,
        retract_mm: Some(0.254),
    })
}

pub fn drill_mut<'a>(job: &'a mut CamJobV5, operation_id: &str) -> Option<&'a mut DrillSettingsV5> {
    match &mut crate::session::operation_mut(job, operation_id)?.settings {
        cam_core::project::v5::OperationSettingsV5::Drill(settings) => Some(settings),
        _ => None,
    }
}

/// Commit all drill dimensions together so partial input cannot replace the
/// tool with another geometry kind or silently invent a point angle.
pub fn set_geometry(job: &mut CamJobV5, operation_id: &str, values: &[f64]) -> Result<(), String> {
    let [diameter_mm, cutting_length_mm, tip_angle_deg] = values else {
        return Err("Complete the drill bit diameter, cutting length and point angle".into());
    };
    crate::session::drill(job, operation_id).ok_or("Expected a Drill operation")?;
    crate::authoring::tool_mut_in(job, operation_id, false)?.geometry = Some(
        cam_core::project::ToolGeometry::Drill(cam_core::model::DrillSpec {
            diameter_mm: *diameter_mm,
            cutting_length_mm: *cutting_length_mm,
            tip_angle_deg: *tip_angle_deg,
        }),
    );
    Ok(())
}

/// Height reference selection for the top, bottom or retract plane.
pub fn set_height_reference(
    job: &mut CamJobV5,
    operation_id: &str,
    which: usize,
    reference: HeightReference,
) -> Result<(), String> {
    let s = drill_mut(job, operation_id).ok_or("Expected a Drill operation")?;
    match which {
        0 => s.top.reference = reference,
        1 => s.bottom.reference = reference,
        _ => s.retract_height.reference = reference,
    }
    job.validate_structure().map_err(|e| e.to_string())
}

/// What the resolved bottom describes: the tip point, or the full diameter.
pub fn set_depth_reference(
    job: &mut CamJobV5,
    operation_id: &str,
    reference: DrillDepthReference,
) -> Result<(), String> {
    drill_mut(job, operation_id)
        .ok_or("Expected a Drill operation")?
        .depth_reference = reference;
    Ok(())
}

pub fn set_hole_order(
    job: &mut CamJobV5,
    operation_id: &str,
    order: DrillHoleOrder,
) -> Result<(), String> {
    drill_mut(job, operation_id)
        .ok_or("Expected a Drill operation")?
        .hole_order = order;
    Ok(())
}

/// `None` disables pecking (one plunge per hole).
pub fn set_peck_mode(
    job: &mut CamJobV5,
    operation_id: &str,
    mode: Option<DrillPeckMode>,
) -> Result<(), String> {
    let s = drill_mut(job, operation_id).ok_or("Expected a Drill operation")?;
    match mode {
        None => s.peck = None,
        Some(mode) => {
            peck_mut(s).mode = mode;
        }
    }
    Ok(())
}

pub fn set_warn_drill_exceeds_marker(
    job: &mut CamJobV5,
    operation_id: &str,
    warn: bool,
) -> Result<(), String> {
    drill_mut(job, operation_id)
        .ok_or("Expected a Drill operation")?
        .warn_drill_exceeds_marker = warn;
    Ok(())
}

/// Spindle rotation of the drill assignment. Required before checked export.
pub fn set_spindle_direction(
    job: &mut CamJobV5,
    operation_id: &str,
    direction: Option<cam_core::project::SpindleDirection>,
) -> Result<(), String> {
    drill_mut(job, operation_id)
        .ok_or("Expected a Drill operation")?
        .assignment
        .spindle_direction = direction;
    Ok(())
}
