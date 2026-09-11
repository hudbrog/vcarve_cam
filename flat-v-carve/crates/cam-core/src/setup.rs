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

/// Resolved concrete heights of one operation, in setup coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedHeights {
    pub top_z: f64,
    pub bottom_z: f64,
}

/// Resolve a height reference (plan section 6.3). `published_planes` carries
/// the face planes established by preceding face operations; B2 accepts an
/// empty map because the face planner ships in C1, and an unresolvable
/// reference stays an explicit error rather than defaulting to stock top.
pub fn resolve_heights(
    job: &CamJob,
    _operation_index: usize,
    top: &crate::project::HeightRef,
    bottom: &crate::project::HeightRef,
    published_planes: &std::collections::BTreeMap<String, f64>,
) -> Result<ResolvedHeights> {
    resolve_heights_values(job.setup.stock.thickness_mm, top, bottom, published_planes)
}

/// Thickness-driven [`resolve_heights`] shared by the schema-4 and schema-5
/// planner paths; only the stock thickness comes from the job.
pub fn resolve_heights_values(
    thickness: Option<f64>,
    top: &crate::project::HeightRef,
    bottom: &crate::project::HeightRef,
    published_planes: &std::collections::BTreeMap<String, f64>,
) -> Result<ResolvedHeights> {
    let thickness = thickness.ok_or_else(|| {
        error(
            "SETUP_STOCK_THICKNESS_REQUIRED",
            "height references require the stock thickness",
        )
    })?;
    let stock_bottom = -thickness;
    let resolve = |height: &crate::project::HeightRef, is_bottom: bool| -> Result<f64> {
        let base = match &height.reference {
            crate::project::HeightReference::StockTop => 0.,
            crate::project::HeightReference::StockBottom => stock_bottom,
            crate::project::HeightReference::OperationTop => {
                // Legal only for bottoms (document validation enforces the
                // placement); it resolves to this operation's own top.
                if !is_bottom {
                    return Err(error(
                        "HEIGHT_REFERENCE_INVALID",
                        "operation_top is legal only for bottom heights",
                    ));
                }
                return Ok(f64::NAN); // replaced by the resolved top plus the offset below
            }
            crate::project::HeightReference::FaceResult { operation_id } => {
                published_planes.get(operation_id).copied().ok_or_else(|| {
                    error(
                        "HEIGHT_REFERENCE_UNRESOLVED",
                        format!(
                            "the face plane of operation '{operation_id}' is not established; face planning ships with the face milestone"
                        ),
                    )
                })?
            }
        };
        Ok(base + height.offset_mm)
    };
    let top_z = resolve(top, false)?;
    let mut bottom_z = resolve(bottom, true)?;
    if bottom_z.is_nan() {
        bottom_z = top_z + bottom.offset_mm;
    }
    if !top_z.is_finite() || !bottom_z.is_finite() {
        return Err(error("SETUP_PARAMETER", "resolved heights must be finite"));
    }
    if bottom_z >= top_z {
        return Err(error(
            "HEIGHT_RANGE_INVALID",
            format!(
                "the resolved bottom ({bottom_z:.4}) must lie below the resolved top ({top_z:.4}) for positive-depth operations"
            ),
        ));
    }
    Ok(ResolvedHeights { top_z, bottom_z })
}

/// Resolve a single height reference to a concrete Z (no range check).
/// FaceResult resolves against published face planes; OperationTop is
/// meaningless without an operation's own top and stays an error here.
pub fn resolve_top(
    job: &CamJob,
    height: &crate::project::HeightRef,
    published_planes: &std::collections::BTreeMap<String, f64>,
) -> Result<f64> {
    resolve_top_values(job.setup.stock.thickness_mm, height, published_planes)
}

/// Thickness-driven [`resolve_top`] shared by the schema-4 and schema-5
/// planner paths.
pub fn resolve_top_values(
    thickness: Option<f64>,
    height: &crate::project::HeightRef,
    published_planes: &std::collections::BTreeMap<String, f64>,
) -> Result<f64> {
    let thickness = thickness.ok_or_else(|| {
        error(
            "SETUP_STOCK_THICKNESS_REQUIRED",
            "height references require the stock thickness",
        )
    })?;
    let base = match &height.reference {
        crate::project::HeightReference::StockTop => 0.,
        crate::project::HeightReference::StockBottom => -thickness,
        crate::project::HeightReference::OperationTop => {
            return Err(error(
                "HEIGHT_REFERENCE_INVALID",
                "operation_top resolves only as an operation bottom",
            ));
        }
        crate::project::HeightReference::FaceResult { operation_id } => {
            published_planes.get(operation_id).copied().ok_or_else(|| {
                error(
                    "HEIGHT_REFERENCE_UNRESOLVED",
                    format!("the face plane of operation '{operation_id}' is not established"),
                )
            })?
        }
    };
    let z = base + height.offset_mm;
    if !z.is_finite() {
        return Err(error("SETUP_PARAMETER", "resolved heights must be finite"));
    }
    Ok(z)
}
