//! Setup resolution: physical stock bounds and the work-zero output
//! transform (plan section 6). Setup coordinates are authoritative: mm,
//! right-handed XY, Z up, original stock top at Z=0. Changing work zero
//! changes the output transform only — never artwork placement or planned
//! removal.
use crate::{
    geometry::{Diagnostic, Result},
    project::{CamJob, WorkZeroXY, WorkZeroZ},
};
use serde::{Deserialize, Serialize};

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("setup")
}

/// The selected work-zero point in setup coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkZeroPoint {
    pub x_mm: f64,
    pub y_mm: f64,
    pub z_mm: f64,
}

impl WorkZeroPoint {
    /// The setup-to-output offset: `output = setup - offset` per axis
    /// (plan section 6.2). Z datum stock-bottom offsets by the original
    /// thickness, never a faced or milled surface.
    pub fn output_offset(&self) -> (f64, f64, f64) {
        (self.x_mm, self.y_mm, self.z_mm)
    }
}

/// Resolve the selected work zero. Anchor selections require physical stock
/// XY dimensions; legacy migrated jobs keep `setup origin`/`stock top`
/// coordinates that resolve without them.
pub fn resolve_work_zero(job: &CamJob) -> Result<WorkZeroPoint> {
    let z_mm = match job.setup.work_zero.z {
        WorkZeroZ::StockTop => 0.,
        WorkZeroZ::StockBottom => -job.setup.stock.thickness_mm.ok_or_else(|| {
            error(
                "SETUP_STOCK_THICKNESS_REQUIRED",
                "the stock-bottom datum requires the stock thickness",
            )
        })?,
    };
    let (x_mm, y_mm) = match &job.setup.work_zero.xy {
        WorkZeroXY::SetupOrigin => (0., 0.),
        WorkZeroXY::StockAnchor {
            x_fraction,
            y_fraction,
        } => {
            let rect = job
                .setup
                .stock
                .xy
                .ok_or_else(|| error(
                    "SETUP_STOCK_XY_REQUIRED",
                    "stock-anchor work zero requires physical stock XY dimensions; legacy jobs have none until supplied",
                ))?;
            (
                rect.min_x_mm + x_fraction.fraction() * rect.width_mm,
                rect.min_y_mm + y_fraction.fraction() * rect.length_mm,
            )
        }
        WorkZeroXY::CustomPoint { x_mm, y_mm } => (*x_mm, *y_mm),
    };
    for value in [x_mm, y_mm, z_mm] {
        if !value.is_finite() {
            return Err(error(
                "SETUP_PARAMETER",
                "work-zero coordinates must be finite",
            ));
        }
    }
    Ok(WorkZeroPoint { x_mm, y_mm, z_mm })
}
