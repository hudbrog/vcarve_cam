use cam_core::project::v5::{CamJobV5, OperationSettingsV5, PlotSettingsV5};

pub(crate) fn settings<'a>(job: &'a CamJobV5, id: &str) -> Option<&'a PlotSettingsV5> {
    let OperationSettingsV5::CenterlinePlot(s) = &crate::session::operation(job, id)?.settings
    else {
        return None;
    };
    Some(s)
}
pub(crate) fn settings_mut<'a>(
    job: &'a mut CamJobV5,
    id: &str,
) -> Result<&'a mut PlotSettingsV5, String> {
    let Some(OperationSettingsV5::CenterlinePlot(s)) =
        crate::session::operation_mut(job, id).map(|op| &mut op.settings)
    else {
        return Err("Select a Centerline Plot operation".into());
    };
    Ok(s)
}
pub(crate) fn value(job: &CamJobV5, id: &str, field: usize) -> Option<f64> {
    let s = settings(job, id)?;
    match field {
        2 => s.draw_feed_mm_min,
        10 => s.lower_feed_mm_min,
        126 => Some(s.compression_mm),
        127 => Some(s.closure_gap_mm),
        6 => job.setup.stock.thickness_mm,
        _ => crate::authoring::value_in(job, id, field),
    }
}
pub(crate) fn set(
    job: &mut CamJobV5,
    id: &str,
    field: usize,
    value: Option<f64>,
) -> Result<(), String> {
    match field {
        2 => settings_mut(job, id)?.draw_feed_mm_min = value,
        10 => settings_mut(job, id)?.lower_feed_mm_min = value,
        126 => settings_mut(job, id)?.compression_mm = value.unwrap_or(0.),
        127 => settings_mut(job, id)?.closure_gap_mm = value.unwrap_or(0.),
        6 => job.setup.stock.thickness_mm = value,
        _ => return crate::authoring::set_in(job, id, field, value),
    }
    Ok(())
}
