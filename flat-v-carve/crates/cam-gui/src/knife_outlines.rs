//! Explicit filled-boundary derivative using the core's imported geometry.
use cam_core::project::v5::*;
use std::fmt::Write;

pub fn create(job: &CamJobV5, id: &ArtworkItemId) -> Result<CamJobV5, String> {
    crate::knife::settings(job).ok_or("Add a knife operation first")?;
    let source = job
        .artwork
        .iter()
        .find(|i| &i.id == id)
        .ok_or("Choose artwork first")?;
    let mut unplaced = source.clone();
    unplaced.placement = Default::default();
    let catalogue = artwork::item_catalogue(source).map_err(|e| e.to_string())?;
    if catalogue.contours.is_empty() {
        return Err(
            "This artwork has no filled boundaries. Select its existing stroked paths instead."
                .into(),
        );
    }
    let page = crate::authoring::svg_page_stock(&unplaced)?;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\">\n",
        page.width_mm, page.length_mm, page.width_mm, page.length_mm
    );
    for (index, contour) in catalogue.contours.iter().enumerate() {
        write!(
            svg,
            "<path id=\"outline-{}\" fill=\"none\" stroke=\"black\" stroke-width=\"0.1\" d=\"",
            index + 1
        )
        .unwrap();
        for (i, point) in contour.vertices.iter().enumerate() {
            let p = source
                .placement
                .to_page(*point)
                .map_err(|e| e.to_string())?;
            write!(
                svg,
                "{}{} {} ",
                if i == 0 { "M" } else { "L" },
                p.x,
                page.length_mm - p.y
            )
            .unwrap();
        }
        svg.push_str("Z\"/>\n");
        if svg.len() > 8_000_000 {
            return Err("Knife outline copy exceeds the 8 MB source limit".into());
        }
    }
    svg.push_str("</svg>\n");
    let mut interpretation = source.import_settings.clone();
    interpretation.mode = cam_core::svg::ImportMode::Centerline;
    let added = commands::add_artwork(
        job,
        vec![commands::ArtworkInput {
            filename: format!("{}-knife-outlines.svg", source.id.0),
            svg,
            interpretation,
            placement: source.placement.clone(),
            name: Some(format!("{} · knife outlines", source.name)),
        }],
    )
    .map_err(|e| e.to_string())?;
    if let Some(rejection) = added.rejected.first() {
        return Err(rejection.error.to_string());
    }
    let job = added.outcome.job;
    crate::session::open(&job.to_json().map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_preserves_outer_and_hole_coordinates_placement_and_original_source() {
        let mut job =
            crate::session::open(include_str!("../../../fixtures/gui4/lettering.job.json"))
                .unwrap();
        job = crate::operation_authoring::apply(
            &crate::operation_authoring::apply(&job, crate::operation_authoring::Action::Delete)
                .unwrap(),
            crate::operation_authoring::Action::AddKnife,
        )
        .unwrap();
        job.artwork[0].placement.rotation_deg = 30.;
        job.artwork[0].placement.scale = 1.5;
        job.artwork[0].placement.origin_mm.x = 2.;
        let catalogue = artwork::item_catalogue(&job.artwork[0]).unwrap();
        let copy = create(&job, &job.artwork[0].id).unwrap();
        assert_eq!(&copy.artwork[..job.artwork.len()], job.artwork);
        assert_eq!(copy.setup, job.setup);
        let chains = crate::knife::chains(&copy).unwrap();
        assert_eq!(chains.len(), catalogue.contours.len());
        for (chain, contour) in chains.iter().zip(&catalogue.contours) {
            assert!(chain.closed);
            for point in &contour.vertices {
                let distance = chain
                    .vertices
                    .iter()
                    .map(|p| (p[0] - point.x).hypot(p[1] - point.y))
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    distance < 1e-6,
                    "distance={distance}, chain={:?}, point={point:?}",
                    chain.vertices
                );
            }
        }
        assert!(crate::knife::settings(&copy).unwrap().chains.is_empty());
        let chosen = crate::knife::select(&copy, &[chains[0].reference.clone()]).unwrap();
        assert_eq!(crate::knife::settings(&chosen).unwrap().chains.len(), 1);
        crate::app::Document::new(chosen)
            .snapshot()
            .validate()
            .unwrap();
    }
}
