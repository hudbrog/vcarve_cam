//! Explicit-topology materialization spike; not a general face finder or saved schema.
use cam_core::{
    contours::{ContourCatalogue, ContourRole},
    geometry::{BooleanOp, Point, Region},
    job::SourceSnapshot,
    project::{
        CamJob,
        v5::{
            ArtworkContent, ArtworkItem, ArtworkItemId, GeometryRef, GeometryRefKind,
            SourceRevision,
        },
    },
    svg::{ImportOptions, Placement, import_svg},
};
use std::collections::BTreeMap;
const HEIGHT: f64 = 100.;
#[derive(Clone)]
enum Shape {
    Rect(f64, f64, f64, f64),
    Circle(f64, f64, f64),
    OpenArc,
}
#[derive(Clone)]
struct Wire {
    id: &'static str,
    shape: Shape,
    construction: bool,
    members: Vec<&'static str>,
}
impl Wire {
    fn path(&self) -> String {
        match self.shape {
            Shape::Rect(x, y, w, h) => format!("M{} {}h{w}v{}h{}Z", x + 100., HEIGHT - y, -h, -w),
            Shape::Circle(x, y, r) => format!(
                "M{} {}a{r} {r} 0 1 0 {} 0a{r} {r} 0 1 0 {} 0Z",
                x + 100. - r,
                HEIGHT - y,
                2. * r,
                -2. * r
            ),
            Shape::OpenArc => "M88 95L98 95A5 5 0 0 0 103 90".into(),
        }
    }
    fn element(&self) -> String {
        match self.shape {
            Shape::Circle(x, y, r) => format!(
                r#"<circle id="{}" cx="{}" cy="{}" r="{r}" fill="none" stroke="black"/>"#,
                self.id,
                x + 100.,
                HEIGHT - y
            ),
            _ => format!(
                r#"<path id="{}" d="{}" fill="none" stroke="black"/>"#,
                self.id,
                self.path()
            ),
        }
    }
}
#[derive(Clone)]
struct Face {
    id: &'static str,
    outer: &'static str,
    holes: Vec<&'static str>,
    containing_hole: Option<&'static str>,
}
#[derive(Clone)]
struct Drawing {
    wires: Vec<Wire>,
    faces: Vec<Face>,
}
fn sample() -> Drawing {
    Drawing {
        wires: vec![
            Wire {
                id: "outer",
                shape: Shape::Rect(-5., -3., 85., 63.),
                construction: false,
                members: vec!["e1", "e2", "e3", "e4"],
            },
            Wire {
                id: "hole-a",
                shape: Shape::Circle(10., 20., 6.),
                construction: false,
                members: vec!["circle-a"],
            },
            Wire {
                id: "hole-b",
                shape: Shape::Circle(55., 20., 8.),
                construction: false,
                members: vec!["circle-b"],
            },
            Wire {
                id: "island",
                shape: Shape::Rect(8., 18., 4., 4.),
                construction: false,
                members: vec!["i1", "i2", "i3", "i4"],
            },
            Wire {
                id: "knife",
                shape: Shape::OpenArc,
                construction: false,
                members: vec!["line", "arc"],
            },
            Wire {
                id: "helper",
                shape: Shape::Circle(10000., 10000., 2.),
                construction: true,
                members: vec!["construction"],
            },
        ],
        faces: vec![
            Face {
                id: "plate",
                outer: "outer",
                holes: vec!["hole-a", "hole-b"],
                containing_hole: None,
            },
            Face {
                id: "island-region",
                outer: "island",
                holes: vec![],
                containing_hole: Some("hole-a"),
            },
        ],
    }
}
fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200">{body}</svg>"#
    )
}
// Stable page envelope [-100,100]^2: compensate inside the adapter only.
fn options(p: &Placement) -> ImportOptions {
    ImportOptions {
        placement: Placement {
            origin_mm: Point::new(p.origin_mm.x + 100., p.origin_mm.y + 100.),
            ..p.clone()
        },
        ..Default::default()
    }
}
fn catalogue(source: String, placement: &Placement) -> ContourCatalogue {
    ContourCatalogue::build(&CamJob {
        name: String::new(),
        source: Some(SourceSnapshot {
            filename: "sketch.svg".into(),
            svg: source,
        }),
        import: options(placement),
        setup: Default::default(),
        tools: vec![],
        operations: vec![],
        tolerances: Default::default(),
    })
    .unwrap()
}
fn item(source: String, placement: &Placement) -> ArtworkItem {
    ArtworkItem {
        id: ArtworkItemId("sketch".into()),
        name: "Sketch".into(),
        content: ArtworkContent::Svg(SourceSnapshot {
            filename: "derived.svg".into(),
            svg: source,
        }),
        import_settings: Default::default(),
        placement: placement.clone(),
    }
}
struct Materialized {
    catalogue: ContourCatalogue,
    filled: Region,
    provenance: BTreeMap<String, Vec<String>>,
    revision: SourceRevision,
    flattening_bound_mm: f64,
    snap_bound_mm: f64,
}
fn face_path(d: &Drawing, f: &Face) -> String {
    std::iter::once(f.outer)
        .chain(f.holes.iter().copied())
        .map(|id| d.wires.iter().find(|w| w.id == id).unwrap().path())
        .collect::<Vec<_>>()
        .join(" ")
}
fn materialize(d: &Drawing, placement: &Placement) -> Materialized {
    let mut out = ContourCatalogue {
        contours: vec![],
        open_chains: vec![],
        points: vec![],
    };
    let mut provenance = BTreeMap::new();
    let mut regions = vec![];
    let mut body = String::new();
    let options = options(placement);
    let mut flattening: f64 = 0.;
    let mut snap: f64 = 0.;
    for f in &d.faces {
        let element = format!(
            r#"<path id="{}" d="{}" fill-rule="evenodd"/>"#,
            f.id,
            face_path(d, f)
        );
        let g = import_svg(&svg(&element), &options, None).unwrap();
        assert_eq!(g.selected.component_count(), 1, "face changed topology");
        assert_eq!(g.selected.hole_count(), f.holes.len(), "face changed holes");
        flattening = flattening.max(g.flattening_bound_mm);
        snap = snap.max(g.source_snap_bound_mm);
        regions.push(g.selected);
        body.push_str(&element);
        let mut signature = vec![format!("outer:{}", f.outer)];
        signature.push(format!("container:{:?}", f.containing_hole));
        signature.extend(f.holes.iter().map(|h| format!("hole:{h}")));
        for id in std::iter::once(f.outer).chain(f.holes.iter().copied()) {
            signature.extend(
                d.wires
                    .iter()
                    .find(|w| w.id == id)
                    .unwrap()
                    .members
                    .iter()
                    .map(|e| format!("{id}/{e}")),
            );
        }
        signature.sort();
        provenance.insert(f.id.into(), signature);
    }
    for w in d.wires.iter().filter(|w| !w.construction) {
        let element = w.element();
        let c = catalogue(svg(&element), placement);
        body.push_str(&element);
        let mut signature = w.members.iter().map(|m| m.to_string()).collect::<Vec<_>>();
        let face = d
            .faces
            .iter()
            .find(|f| f.outer == w.id || f.holes.contains(&w.id));
        let hole = face.is_some_and(|f| f.holes.contains(&w.id));
        if let Some(f) = face {
            signature.push(format!("face:{}:hole:{hole}", f.id));
            signature.push(format!("container:{:?}", f.containing_hole));
        }
        signature.sort();
        provenance.insert(w.id.into(), signature.clone());
        for mut contour in c.contours {
            contour.id = w.id.into();
            contour.source_id = w.id.into();
            if let Some(f) = face {
                contour.component_id = f.id.into();
            }
            contour.role = if hole {
                ContourRole::Hole
            } else {
                ContourRole::Outer
            };
            contour.parent_contour_id = if hole {
                face.map(|f| f.outer.into())
            } else {
                None
            };
            out.contours.push(contour);
        }
        for mut chain in c.open_chains {
            chain.id = format!("{}-chain", w.id);
            chain.source_id = w.id.into();
            provenance.insert(chain.id.clone(), signature.clone());
            out.open_chains.push(chain);
        }
        // Keep exact circles, exclude derived centroids and helper points.
        for mut point in c.points.into_iter().filter(|p| p.exact) {
            point.id = format!("{}-center", w.id);
            provenance.insert(point.id.clone(), signature.clone());
            out.points.push(point);
        }
    }
    let filled = Region::union_all(regions[0].grid(), &regions.iter().collect::<Vec<_>>()).unwrap();
    let revision = item(svg(&body), placement).source_revision().unwrap();
    Materialized {
        catalogue: out,
        filled,
        provenance,
        revision,
        flattening_bound_mm: flattening,
        snap_bound_mm: snap,
    }
}
fn rebind(old: &Materialized, new: &Materialized, r: &GeometryRef) -> Option<GeometryRef> {
    if r.artwork_item_id.0 != "sketch" || r.source_revision != old.revision {
        return None;
    }
    let a = old.provenance.get(&r.local_geometry_id)?;
    if new.provenance.get(&r.local_geometry_id)? != a {
        return None;
    }
    let exists = match r.kind {
        GeometryRefKind::FilledComponent => new
            .catalogue
            .contours
            .iter()
            .any(|c| c.component_id == r.local_geometry_id),
        GeometryRefKind::ClosedContour => new
            .catalogue
            .contours
            .iter()
            .any(|c| c.id == r.local_geometry_id),
        GeometryRefKind::Centerline => new
            .catalogue
            .open_chains
            .iter()
            .any(|c| c.id == r.local_geometry_id),
        GeometryRefKind::Point => new
            .catalogue
            .points
            .iter()
            .any(|p| p.id == r.local_geometry_id),
    };
    exists.then(|| GeometryRef {
        source_revision: new.revision.clone(),
        ..r.clone()
    })
}
fn pick(m: &Materialized, id: &str, kind: GeometryRefKind) -> GeometryRef {
    GeometryRef {
        artwork_item_id: ArtworkItemId("sketch".into()),
        kind,
        local_geometry_id: id.into(),
        source_revision: m.revision.clone(),
    }
}
fn check() -> serde_json::Value {
    let d = sample();
    let placement = Placement::default();
    let a = materialize(&d, &placement);
    assert_eq!(
        (
            a.catalogue.contours.len(),
            a.catalogue.open_chains.len(),
            a.catalogue.points.len()
        ),
        (4, 5, 2)
    );
    assert_eq!((a.filled.component_count(), a.filled.hole_count()), (2, 2));
    assert!(a.catalogue.points.iter().all(|p| p.exact));
    assert!((a.filled.area_mm2() - (85. * 63. - std::f64::consts::PI * 100. + 16.)).abs() < 0.1);
    let knife = a
        .catalogue
        .open_chains
        .iter()
        .find(|c| c.id == "knife-chain")
        .unwrap();
    assert!(!knife.closed);
    assert!(knife.vertices[0].distance(Point::new(-12., 5.)) < 1e-10);
    assert!(knife.vertices.last().unwrap().distance(Point::new(3., 10.)) < 1e-10);
    let compound = d
        .wires
        .iter()
        .filter(|w| !w.construction && !matches!(w.shape, Shape::OpenArc))
        .map(Wire::path)
        .collect::<Vec<_>>()
        .join(" ");
    let reference = import_svg(
        &svg(&format!(
            r#"<path id="reference" d="{compound}" fill-rule="evenodd"/>"#
        )),
        &options(&placement),
        None,
    )
    .unwrap();
    let difference = a
        .filled
        .boolean(BooleanOp::Xor, &reference.selected)
        .unwrap()
        .area_mm2();
    assert!(difference < 1e-9);
    let mut edited = d.clone();
    edited.wires[1].shape = Shape::Circle(55., 20., 6.);
    edited.wires[2].shape = Shape::Circle(10., 20., 8.);
    edited.wires[3].shape = Shape::Rect(53., 18., 4., 4.);
    edited.faces[0].holes.reverse();
    let b = materialize(&edited, &placement);
    let refs = [
        pick(&a, "hole-a", GeometryRefKind::ClosedContour),
        pick(&a, "plate", GeometryRefKind::FilledComponent),
        pick(&a, "hole-a-center", GeometryRefKind::Point),
        pick(&a, "knife-chain", GeometryRefKind::Centerline),
    ];
    assert!(refs.iter().all(|r| rebind(&a, &b, r).is_some()));
    assert_ne!(a.revision, b.revision);
    assert!(
        (b.catalogue
            .points
            .iter()
            .find(|p| p.id == "hole-a-center")
            .unwrap()
            .center
            .x
            - 55.)
            .abs()
            < 1e-10
    );
    let raw = |d: &Drawing| {
        catalogue(
            svg(&format!(
                r#"<path id="plate" d="{}" fill-rule="evenodd"/>"#,
                face_path(d, &d.faces[0])
            )),
            &placement,
        )
    };
    let ra = raw(&d);
    let rb = raw(&edited);
    let h1 = ra
        .contours
        .iter()
        .find(|c| c.id.ends_with("hole-0"))
        .unwrap();
    let h2 = rb.contours.iter().find(|c| c.id == h1.id).unwrap();
    assert!((h1.perimeter_mm - h2.perimeter_mm).abs() > 1.);
    let mut resized = d.clone();
    resized.wires[0].shape = Shape::Rect(-5., -3., 90., 63.);
    let resized = materialize(&resized, &placement);
    assert!(refs.iter().all(|r| rebind(&a, &resized, r).is_some()));
    assert_ne!(a.revision, resized.revision);
    let mut split = d.clone();
    split.wires[1].members = vec!["new-arc-a", "new-arc-b"];
    let split = materialize(&split, &placement);
    assert!(rebind(&a, &split, &refs[0]).is_none());
    assert!(rebind(&a, &split, &refs[1]).is_none());
    let mut deleted = d.clone();
    deleted.wires[2].construction = true;
    deleted.faces[0].holes.retain(|id| *id != "hole-b");
    let deleted = materialize(&deleted, &placement);
    assert!(
        rebind(
            &a,
            &deleted,
            &pick(&a, "hole-b", GeometryRefKind::ClosedContour)
        )
        .is_none()
    );
    assert!(rebind(&a, &deleted, &refs[1]).is_none());
    let mut nesting = d.clone();
    nesting.wires[3].shape = Shape::Rect(-20., 18., 4., 4.);
    nesting.faces[1].containing_hole = None;
    let nesting = materialize(&nesting, &placement);
    assert!(
        rebind(
            &a,
            &nesting,
            &pick(&a, "island", GeometryRefKind::ClosedContour)
        )
        .is_none()
    );
    assert!(
        rebind(
            &a,
            &nesting,
            &pick(&a, "island-region", GeometryRefKind::FilledComponent)
        )
        .is_none()
    );
    let mut stale = refs[0].clone();
    stale.source_revision = b.revision.clone();
    assert!(rebind(&a, &b, &stale).is_none());
    let mut foreign = refs[0].clone();
    foreign.artwork_item_id.0 = "other".into();
    assert!(rebind(&a, &b, &foreign).is_none());
    let mut helper = d.clone();
    helper.wires[5].shape = Shape::Circle(-500., -500., 8.);
    assert_eq!(a.revision, materialize(&helper, &placement).revision);
    let placed = Placement {
        origin_mm: Point::new(3., -4.),
        scale: 2.,
        rotation_deg: 37.,
    };
    let c = materialize(&d, &placed);
    assert_eq!(a.revision, c.revision);
    for p in &a.catalogue.points {
        let q = c.catalogue.points.iter().find(|q| p.id == q.id).unwrap();
        assert!(placed.to_setup(p.center).unwrap().distance(q.center) < 1e-9);
        assert!((q.diameter_mm - p.diameter_mm * 2.).abs() < 1e-9);
    }
    for p in &a.catalogue.open_chains {
        let q = c
            .catalogue
            .open_chains
            .iter()
            .find(|q| p.id == q.id)
            .unwrap();
        assert!(
            placed
                .to_setup(p.vertices[0])
                .unwrap()
                .distance(q.vertices[0])
                < 0.001
        );
    }
    let old_hole = a
        .catalogue
        .contours
        .iter()
        .find(|c| c.id == "hole-a")
        .unwrap();
    let new_hole = b
        .catalogue
        .contours
        .iter()
        .find(|c| c.id == "hole-a")
        .unwrap();
    assert_ne!(old_hole.source_fingerprint, new_hole.source_fingerprint);
    let anchor = cam_core::project::ContourAnchor {
        contour_id: "hole-a".into(),
        source_geometry_fingerprint: old_hole.source_fingerprint.clone(),
        fraction_along_source_contour: 0.3,
    };
    assert!(a.catalogue.resolve_anchor(&anchor).is_ok());
    assert!(b.catalogue.resolve_anchor(&anchor).is_err());
    let serialized = serde_json::to_string(&refs).unwrap();
    let restored: Vec<GeometryRef> = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored, refs);
    assert!(restored.iter().all(|r| rebind(&a, &a, r).is_some()));
    serde_json::json!({"passed":true,"closed_contours":4,"centerlines":5,"analytic_centers":2,"filled_components":2,"holes":2,"region_xor_area_mm2":difference,"flattening_bound_mm":a.flattening_bound_mm,"source_snap_bound_mm":a.snap_bound_mm,"hole_reorder_preserved":true,"raw_svg_hole_index_changes_meaning":true,"dimension_references_preserved":true,"split_and_deleted_references_unresolved":true,"stale_and_foreign_references_rejected":true,"construction_excluded":true,"placement_and_frame_checked":true,"changed_geometry_revision_invalidates_old_binding":true,"changed_anchor_fingerprint_requires_repair":true})
}
pub fn report() -> String {
    serde_json::to_string_pretty(&check()).unwrap()
}
#[cfg(test)]
mod tests {
    #[test]
    fn geometry_and_provenance_contract() {
        super::check();
    }
}
