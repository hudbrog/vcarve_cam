//! Face-operation editor helpers (GUI7a coverage settings and GUI7b controls).
//!
//! The planner owns every machining decision: this module only binds the
//! editor's typed fields to the canonical [`FaceSettingsV5`] values and back.
//! Empty text stays unset, so a half-configured face never looks ready.
use cam_core::project::v5::CamJobV5;
use cam_core::project::{FaceArea, v5};

/// Fields this editor binds for a Face operation. IDs 2/8/9/10/11 are the
/// operation's own assignment values, exactly as they are for a Flat V-carve.
pub fn active(field: usize) -> bool {
    matches!(
        field,
        2 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 23 | 25 | 26..=36 | 40..=45 | 75..=88
    )
}

/// Face area fields form one group when the requested area is a rectangle.
pub fn group(field: usize) -> &'static [usize] {
    match field {
        82..=85 => &[82, 83, 84, 85],
        _ => &[],
    }
}

pub fn value(job: &CamJobV5, operation_id: &str, field: usize) -> Option<f64> {
    let s = crate::session::face(job, operation_id);
    match field {
        2 => s?.assignment.cutting_feed_mm_min,
        8 => s?.stepdown_mm,
        9 => s?.stepover_mm,
        10 => s?.assignment.plunge_feed_mm_min,
        11 => s?.assignment.spindle_rpm,
        75 => s?.pass_angle_deg,
        76 => s?.entry_overrun_mm,
        77 => s?.exit_overrun_mm,
        78 => s?.margins.min_x_mm,
        79 => s?.margins.max_x_mm,
        80 => s?.margins.min_y_mm,
        81 => s?.margins.max_y_mm,
        82..=85 => match &s?.area {
            FaceArea::Rectangle { rect } => Some(match field {
                82 => rect.min_x_mm,
                83 => rect.min_y_mm,
                84 => rect.width_mm,
                _ => rect.length_mm,
            }),
            FaceArea::EntireStock => None,
        },
        86 => Some(s?.top.offset_mm),
        87 => Some(s?.bottom.offset_mm),
        88 => s?.assignment.max_stepdown_mm,
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
    if !matches!(field, 2 | 8 | 9 | 10 | 11 | 75..=88) {
        return crate::authoring::set_in(job, operation_id, field, v);
    }
    let s = crate::session::face_mut(job, operation_id).ok_or("Expected a Face operation")?;
    match field {
        2 => s.assignment.cutting_feed_mm_min = v,
        8 => s.stepdown_mm = v,
        9 => s.stepover_mm = v,
        10 => s.assignment.plunge_feed_mm_min = v,
        11 => s.assignment.spindle_rpm = v,
        75 => s.pass_angle_deg = v,
        76 => s.entry_overrun_mm = v,
        77 => s.exit_overrun_mm = v,
        78 => s.margins.min_x_mm = v,
        79 => s.margins.max_x_mm = v,
        80 => s.margins.min_y_mm = v,
        81 => s.margins.max_y_mm = v,
        82..=85 => {
            let FaceArea::Rectangle { rect } = &mut s.area else {
                return Err("Choose a rectangle face area before setting its corners".into());
            };
            let n = v.ok_or("Rectangle face area values cannot be unset")?;
            match field {
                82 => rect.min_x_mm = n,
                83 => rect.min_y_mm = n,
                84 => rect.width_mm = n,
                _ => rect.length_mm = n,
            }
        }
        86 => s.top.offset_mm = v.ok_or("Top offset cannot be unset")?,
        87 => s.bottom.offset_mm = v.ok_or("Bottom offset cannot be unset")?,
        _ => s.assignment.max_stepdown_mm = v,
    }
    Ok(())
}

/// Set the requested area mode. Switching to a rectangle starts from the
/// physical stock rectangle when it is known, otherwise from the entered
/// corners only; the planner still validates coverage and rejection.
pub fn set_area(job: &mut CamJobV5, operation_id: &str, area: FaceArea) -> Result<(), String> {
    let stock_xy = job.setup.stock.xy;
    let s = crate::session::face_mut(job, operation_id).ok_or("Expected a Face operation")?;
    s.area = match area {
        FaceArea::EntireStock => FaceArea::EntireStock,
        FaceArea::Rectangle { rect } => {
            let rect = if rect.width_mm > 0. && rect.length_mm > 0. {
                rect
            } else {
                stock_xy.unwrap_or(rect)
            };
            FaceArea::Rectangle { rect }
        }
    };
    Ok(())
}

pub fn area(job: &CamJobV5, operation_id: &str) -> Option<FaceArea> {
    Some(crate::session::face(job, operation_id)?.area.clone())
}

pub fn pattern(job: &CamJobV5, operation_id: &str) -> Option<cam_core::project::FacePattern> {
    Some(crate::session::face(job, operation_id)?.pattern)
}

pub fn set_pattern(
    job: &mut CamJobV5,
    operation_id: &str,
    pattern: cam_core::project::FacePattern,
) -> Result<(), String> {
    crate::session::face_mut(job, operation_id)
        .ok_or("Expected a Face operation")?
        .pattern = pattern;
    Ok(())
}

/// Spindle rotation of the face assignment. Required before checked export.
pub fn set_spindle_direction(
    job: &mut CamJobV5,
    operation_id: &str,
    direction: Option<cam_core::project::SpindleDirection>,
) -> Result<(), String> {
    crate::session::face_mut(job, operation_id)
        .ok_or("Expected a Face operation")?
        .assignment
        .spindle_direction = direction;
    Ok(())
}

/// Group commit for the rectangle area corners (field 82…85). All four values
/// arrive together, exactly like the other multi-field groups.
pub fn set_group(
    job: &mut CamJobV5,
    operation_id: &str,
    field: usize,
    values: &[f64],
) -> Result<(), String> {
    if !matches!(field, 82..=85) {
        return Err("Unknown face field group".into());
    }
    if values.len() != 4 {
        return Err("A rectangle face area needs four values".into());
    }
    let s = crate::session::face_mut(job, operation_id).ok_or("Expected a Face operation")?;
    s.area = FaceArea::Rectangle {
        rect: cam_core::project::RectXY {
            min_x_mm: values[0],
            min_y_mm: values[1],
            width_mm: values[2],
            length_mm: values[3],
        },
    };
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(())
}

/// Height reference selection for the top or bottom of a face operation.
pub fn set_height_reference(
    job: &mut CamJobV5,
    operation_id: &str,
    bottom: bool,
    reference: cam_core::project::HeightReference,
) -> Result<(), String> {
    let s = crate::session::face_mut(job, operation_id).ok_or("Expected a Face operation")?;
    if bottom {
        s.bottom.reference = reference;
    } else {
        s.top.reference = reference;
    }
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(())
}

/// Every face plane an earlier operation published, in document order, for the
/// height-reference choices of a later operation.
pub fn published_faces(job: &CamJobV5, before: &str) -> Vec<String> {
    let mut out = vec![];
    for operation in &job.operations {
        if operation.id == before {
            break;
        }
        if matches!(operation.settings, v5::OperationSettingsV5::Face(_)) {
            out.push(operation.id.clone());
        }
    }
    out
}
