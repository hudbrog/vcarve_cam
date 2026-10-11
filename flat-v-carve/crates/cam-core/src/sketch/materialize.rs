//! Verified geometry through the shared SVG flattener and precision machinery.
//! Generated SVG is an implementation detail, never the saved sketch model.
use super::*;
use crate::{
    contours::{ContourCatalogue, ContourRole},
    geometry::{Grid, Region},
    project::v5::GeometryRefKind,
    svg::{self, Bounds, ImportOptions, MappedComponent, NormalizedGeometry, SourceComponent},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

/// Coordinate-independent continuity evidence. Components include their holes;
/// contours include ancestry; chains include ordered edge membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub kind: GeometryRefKind,
    pub local_id: String,
    pub wires: Vec<(String, Vec<WireEdge>, bool)>,
    pub entity_kinds: Vec<(String, String)>,
    pub ancestry: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct MaterializedArtwork {
    pub geometry: NormalizedGeometry,
    pub catalogue: ContourCatalogue,
    /// SVG retains conservative revision semantics without continuity evidence.
    pub provenance: Vec<Provenance>,
}
impl MaterializedArtwork {
    pub fn provenance(&self, kind: GeometryRefKind, id: &str) -> Option<&Provenance> {
        self.provenance
            .iter()
            .find(|p| p.kind == kind && p.local_id == id)
    }
}

fn wire_svg(
    doc: &SketchDocument,
    w: &Wire,
    join_tolerance: f64,
    flatten_tolerance: f64,
) -> Result<String> {
    let f = &doc.frame;
    let xy = |p: Point| Point::new(p.x - f.min.x, f.height_mm - (p.y - f.min.y));
    if w.edges.is_empty() {
        return Err(located(
            "SKETCH_WIRE",
            &w.id,
            "wire must have at least one edge",
        ));
    }
    if w.edges.len() == 1 {
        let e = doc.entity(&w.edges[0].entity_id)?;
        if let SketchGeometry::Circle { center, radius_mm } = &e.geometry {
            if !w.closed {
                return Err(located("SKETCH_WIRE", &w.id, "circle wire must be closed"));
            }
            let p = xy(*center);
            return Ok(format!(
                "<circle id=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
                w.id, p.x, p.y, radius_mm
            ));
        }
    }
    let mut path = String::new();
    let mut first = None;
    let mut previous = None;
    for (edge_index, edge) in w.edges.iter().enumerate() {
        let g = &doc.entity(&edge.entity_id)?.geometry;
        let (Some(mut start), Some(mut end)) = (g.point(PointRole::Start), g.point(PointRole::End))
        else {
            return Err(located(
                "SKETCH_WIRE",
                &w.id,
                "only a standalone closed wire can contain a circle",
            ));
        };
        if edge.reversed {
            std::mem::swap(&mut start, &mut end);
        }
        if let Some(p) = previous {
            if start.distance(p) > join_tolerance {
                return Err(located(
                    "SKETCH_OPEN_JOIN",
                    &w.id,
                    "ordered edges do not share endpoints within the join budget",
                ));
            }
        } else {
            let p = xy(start);
            write!(path, "M {} {} ", p.x, p.y).expect("string");
            first = Some(start);
        }
        // Close to the persistent first endpoint within the declared residual
        // budget, avoiding a microscopic extra edge that collapses on the grid.
        let output_end = if w.closed && edge_index + 1 == w.edges.len() {
            first.expect("first endpoint")
        } else {
            end
        };
        let p = xy(output_end);
        match g {
            SketchGeometry::Line { .. } => write!(path, "L {} {} ", p.x, p.y),
            SketchGeometry::Arc {
                center,
                radius_mm,
                start_angle_rad,
                sweep_angle_rad,
                ..
            } => {
                let sweep = if edge.reversed {
                    -sweep_angle_rad
                } else {
                    *sweep_angle_rad
                };
                let angle = start_angle_rad + if edge.reversed { *sweep_angle_rad } else { 0. };
                let points = svg::flatten_circular_arc(
                    *center,
                    *radius_mm,
                    angle,
                    sweep,
                    flatten_tolerance,
                )?;
                for (i, p) in points.iter().enumerate().skip(1) {
                    let p = xy(if i + 1 == points.len() {
                        output_end
                    } else {
                        *p
                    });
                    write!(path, "L {} {} ", p.x, p.y).expect("string");
                    if path.len() > svg::MAX_SVG_BYTES {
                        return Err(located(
                            "SKETCH_LIMIT",
                            &w.id,
                            "flattened wire exceeds the geometry bridge budget",
                        ));
                    }
                }
                Ok(())
            }
            _ => unreachable!("validated wire geometry"),
        }
        .expect("string");
        previous = Some(end);
    }
    if w.closed {
        if first
            .expect("nonempty")
            .distance(previous.expect("nonempty"))
            > join_tolerance
        {
            return Err(located(
                "SKETCH_OPEN_JOIN",
                &w.id,
                "closed wire has an open final join",
            ));
        }
        path.push('Z');
    } else if first
        .expect("nonempty")
        .distance(previous.expect("nonempty"))
        <= join_tolerance
    {
        return Err(located(
            "SKETCH_WIRE",
            &w.id,
            "coincident wire endpoints require an explicit closed wire",
        ));
    }
    Ok(format!("<path id=\"{}\" d=\"{path}\"/>", w.id))
}

pub fn materialize(
    doc: &SketchDocument,
    options: &ImportOptions,
    selection: Option<&[String]>,
) -> Result<MaterializedArtwork> {
    doc.require_valid_solution()?;
    Grid::new(options.geometry_tolerance_mm, 0.)?;
    let mut options = options.clone();
    // Page XY = local XY - frame.min. Compensate placement exactly once.
    options.placement.origin_mm.x -= doc.frame.min.x;
    options.placement.origin_mm.y -= doc.frame.min.y;
    options.placement.to_setup(Point::new(0., 0.))?;
    let join_tolerance =
        RESIDUAL_TOLERANCE_MM.min(options.geometry_tolerance_mm / (16. * options.placement.scale));
    let mut used = BTreeSet::new();
    let mut active = BTreeMap::new();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\"><g fill=\"none\" stroke=\"black\">",
        doc.frame.width_mm, doc.frame.height_mm, doc.frame.width_mm, doc.frame.height_mm
    );
    for w in &doc.wires {
        let mut construction = None;
        for edge in &w.edges {
            let e = doc.entity(&edge.entity_id)?;
            if !used.insert(&e.id) {
                return Err(located(
                    "SKETCH_WIRE",
                    &e.id,
                    "an edge may belong to exactly one wire",
                ));
            }
            if construction.is_some_and(|c| c != e.construction) {
                return Err(located(
                    "SKETCH_WIRE",
                    &w.id,
                    "construction and published edges cannot share a wire",
                ));
            }
            construction = Some(e.construction);
        }
        if construction == Some(true) {
            continue;
        }
        let element = wire_svg(
            doc,
            w,
            join_tolerance,
            options.geometry_tolerance_mm / (4. * options.placement.scale),
        )?;
        if svg.len() + element.len() > svg::MAX_SVG_BYTES {
            return Err(error(
                "SKETCH_LIMIT",
                "flattened sketch exceeds the geometry bridge budget",
            ));
        }
        svg.push_str(&element);
        active.insert(w.id.clone(), w);
    }
    for e in &doc.entities {
        if !e.construction
            && !matches!(e.geometry, SketchGeometry::Point { .. })
            && !used.contains(&e.id)
        {
            return Err(located(
                "SKETCH_WIRE",
                &e.id,
                "published geometry must belong to an explicit wire",
            ));
        }
    }
    svg.push_str("</g></svg>");
    if active.is_empty() {
        if selection.is_some_and(|s| !s.is_empty()) {
            return Err(error("SKETCH_SELECTION", "unknown selected region"));
        }
        let grid = match options.ticks_per_mm {
            Some(s) => Grid::with_scale(options.geometry_tolerance_mm, 0., s)?,
            None => Grid::new(options.geometry_tolerance_mm, 0.)?,
        };
        return Ok(MaterializedArtwork {
            geometry: NormalizedGeometry {
                page_width_mm: doc.frame.width_mm,
                page_height_mm: doc.frame.height_mm,
                flattening_bound_mm: 0.,
                source_grid: grid,
                source_snap_bound_mm: 0.,
                grid,
                sources: vec![],
                chains: vec![],
                points: vec![],
                selected_region_ids: vec![],
                selected: Region::from_rings(grid, &[])?,
                components: vec![],
                bounds: None,
                diagnostics: vec![],
            },
            catalogue: ContourCatalogue {
                contours: vec![],
                open_chains: vec![],
                points: vec![],
            },
            provenance: vec![],
        });
    }
    let mut geometry = svg::import_svg(&svg, &options, None)?;
    // Strict topology precedes fill unions. Region::from_rings preserves input
    // order, associating ancestry by wire identity rather than proximity.
    let mut ring_ids = vec![];
    let mut rings = vec![];
    for chain in &mut geometry.chains {
        chain.label = active[&chain.source_id].label.clone();
        if chain.closed {
            ring_ids.push(chain.source_id.clone());
            rings.push(
                chain
                    .points
                    .iter()
                    .map(|p| options.placement.to_setup(*p))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    let region = Region::from_rings(geometry.grid, &rings)?;
    Region::from_rings(
        geometry.source_grid,
        &geometry
            .chains
            .iter()
            .filter(|c| c.closed)
            .map(|c| c.points.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut catalogue = ContourCatalogue::from_geometry(&geometry, &options)?;
    let mut provenance = vec![];
    let ancestry = |mut i: usize| {
        let mut ids = vec![];
        while let Some(p) = region.rings()[i].parent() {
            ids.push(ring_ids[p].clone());
            i = p;
        }
        ids
    };
    let proof = |kind, local_id: String, ids: Vec<String>, ancestry: Vec<String>| Provenance {
        kind,
        local_id,
        wires: ids
            .iter()
            .map(|id| {
                let w = active[id];
                (id.clone(), w.edges.clone(), w.closed)
            })
            .collect(),
        entity_kinds: ids
            .iter()
            .flat_map(|id| active[id].edges.iter())
            .map(|edge| {
                let e = doc.entity(&edge.entity_id).expect("validated");
                (
                    e.id.clone(),
                    match e.geometry {
                        SketchGeometry::Point { .. } => "point",
                        SketchGeometry::Line { .. } => "line",
                        SketchGeometry::Circle { .. } => "circle",
                        SketchGeometry::Arc {
                            sweep_angle_rad, ..
                        } => {
                            if sweep_angle_rad < 0. {
                                "arc_cw"
                            } else {
                                "arc_ccw"
                            }
                        }
                    }
                    .into(),
                )
            })
            .collect(),
        ancestry,
    };
    for c in &mut catalogue.open_chains {
        c.id = format!("chain-{}", c.source_id);
        c.component_id = format!("region-{}", c.source_id);
        provenance.push(proof(
            GeometryRefKind::Centerline,
            c.id.clone(),
            vec![c.source_id.clone()],
            vec![],
        ));
    }
    for c in &mut catalogue.contours {
        let i = ring_ids
            .iter()
            .position(|id| id == &c.source_id)
            .expect("closed wire");
        let ring = &region.rings()[i];
        let owner = if ring.is_hole() {
            ring.parent().expect("hole parent")
        } else {
            i
        };
        c.id = format!("contour-{}", c.source_id);
        c.component_id = format!("region-{}", ring_ids[owner]);
        c.role = if ring.is_hole() {
            ContourRole::Hole
        } else {
            ContourRole::Outer
        };
        c.parent_contour_id = ring.parent().map(|p| format!("contour-{}", ring_ids[p]));
        provenance.push(proof(
            GeometryRefKind::ClosedContour,
            c.id.clone(),
            vec![c.source_id.clone()],
            ancestry(i),
        ));
    }
    // Analytic circles publish drill points; no tessellation-dependent centroids.
    catalogue.points.retain(|p| p.exact);
    for p in &mut catalogue.points {
        let w = active[&p.source_id];
        let entity = &w.edges[0].entity_id;
        p.id = format!("center-{entity}");
        p.label = doc
            .entity(entity)?
            .label
            .clone()
            .or_else(|| w.label.clone());
        provenance.push(proof(
            GeometryRefKind::Point,
            p.id.clone(),
            vec![w.id.clone()],
            vec![],
        ));
    }
    let ring_points = region.rings_mm();
    for (i, _) in region
        .rings()
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.is_hole())
    {
        let mut members = vec![i];
        members.extend(
            region
                .rings()
                .iter()
                .enumerate()
                .filter(|(_, r)| r.parent() == Some(i) && r.is_hole())
                .map(|(j, _)| j),
        );
        let id = format!("region-{}", ring_ids[i]);
        let component = Region::from_rings(
            geometry.grid,
            &members
                .iter()
                .map(|j| ring_points[*j].clone())
                .collect::<Vec<_>>(),
        )?;
        let mut ids = members
            .iter()
            .map(|j| ring_ids[*j].clone())
            .collect::<Vec<_>>();
        ids.sort();
        provenance.push(proof(
            GeometryRefKind::FilledComponent,
            id.clone(),
            ids,
            ancestry(i),
        ));
        geometry.sources.push(SourceComponent {
            id,
            source_id: ring_ids[i].clone(),
            label: active[&ring_ids[i]].label.clone(),
            group: None,
            paint: None,
            geometry: component,
        });
    }
    geometry.selected_region_ids = selection.map_or_else(
        || geometry.sources.iter().map(|s| s.id.clone()).collect(),
        |s| s.to_vec(),
    );
    let selected = geometry.selected_region_ids.iter().collect::<BTreeSet<_>>();
    if selected.len() != geometry.selected_region_ids.len()
        || selected
            .iter()
            .any(|id| !geometry.sources.iter().any(|s| &s.id == *id))
    {
        return Err(error(
            "SKETCH_SELECTION",
            "duplicate or unknown selected region",
        ));
    }
    let sources = geometry
        .sources
        .iter()
        .filter(|s| selected.contains(&s.id))
        .collect::<Vec<_>>();
    geometry.selected = Region::union_all(
        geometry.grid,
        &sources.iter().map(|s| &s.geometry).collect::<Vec<_>>(),
    )?;
    geometry.components = sources
        .iter()
        .map(|s| MappedComponent {
            geometry: s.geometry.clone(),
            selected_region_ids: vec![s.id.clone()],
            source_ids: vec![s.source_id.clone()],
        })
        .collect();
    geometry.bounds = Bounds::of(&geometry.selected);
    for chain in &mut geometry.chains {
        chain.id = format!("chain-{}", chain.source_id);
    }
    for point in &mut geometry.points {
        let w = active[&point.source_id];
        point.id = format!("center-{}", w.edges[0].entity_id);
        point.label = catalogue.point(&point.id).and_then(|p| p.label.clone());
    }
    geometry.flattening_bound_mm += join_tolerance * options.placement.scale;
    Ok(MaterializedArtwork {
        geometry,
        catalogue,
        provenance,
    })
}
