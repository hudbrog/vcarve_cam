//! Starting a project from one SVG.
//!
//! The GUI and the command line share this constructor so a file imported
//! either way is the same document: one artwork item, its page-sized stock and
//! the planning tolerances. The operation list and the job tools stay empty —
//! an import is artwork, not a machining step — so Face, Flat V-carve, Profile
//! and drag knife are only ever added by an explicit user action.
use super::{ArtworkContent, ArtworkItem, ArtworkItemId, CamJobV5, SvgInterpretation, artwork};
use crate::{
    geometry::{Point, Result},
    job::{PlanningTolerances, SourceSnapshot},
    project::{SetupSettings, StockSetup, error},
};

/// The whole SVG page, including its placement, as a stock rectangle. Page
/// capture is a page measurement, not the bounds of the drawn paths.
pub fn page_stock(item: &ArtworkItem) -> Result<crate::project::RectXY> {
    let ArtworkContent::Svg(source) = &item.content;
    let geometry = crate::svg::import_svg(&source.svg, &item.import_options(), None)?;
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for (x, y) in [
        (0., 0.),
        (geometry.page_width_mm, 0.),
        (0., geometry.page_height_mm),
        (geometry.page_width_mm, geometry.page_height_mm),
    ] {
        let p = item.placement.to_setup(Point::new(x, y))?;
        min[0] = min[0].min(p.x);
        min[1] = min[1].min(p.y);
        max[0] = max[0].max(p.x);
        max[1] = max[1].max(p.y);
    }
    Ok(crate::project::RectXY {
        min_x_mm: min[0],
        min_y_mm: min[1],
        width_mm: max[0] - min[0],
        length_mm: max[1] - min[1],
    })
}

/// A new schema-5 document holding exactly this artwork.
pub fn from_svg(filename: String, svg: String, geometry_tolerance_mm: f64) -> Result<CamJobV5> {
    if !geometry_tolerance_mm.is_finite() || geometry_tolerance_mm <= 0. {
        return Err(error(
            "PROJECT_PARAMETER",
            "geometry_tolerance_mm must be finite and positive",
        ));
    }
    let item = ArtworkItem {
        id: ArtworkItemId("artwork-1".into()),
        name: filename.clone(),
        content: ArtworkContent::Svg(SourceSnapshot {
            filename: filename.clone(),
            svg,
        }),
        import_settings: SvgInterpretation {
            geometry_tolerance_mm,
            ticks_per_mm: None,
        },
        placement: Default::default(),
    };
    // Import admission belongs to the one SVG importer: units, geometry
    // interpretation and unsupported content are resolved there, so a source
    // without usable geometry is refused by the importer rather than by a gate
    // shaped around one operation kind.
    let catalogue = artwork::resolve_artwork_item(&item)?;
    if let Some(message) = catalogue.import_error {
        return Err(error("SVG_IMPORT", message));
    }
    let job = CamJobV5 {
        schema_version: 5,
        name: filename,
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(18.),
                xy: Some(page_stock(&item)?),
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
            ..Default::default()
        },
        artwork: vec![item],
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    };
    job.validate_structure()?;
    Ok(job)
}
