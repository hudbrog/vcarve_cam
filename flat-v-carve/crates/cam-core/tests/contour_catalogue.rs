//! D1 contour catalogue: stable contour identity across source winding,
//! exact per-contour selection, and source-parameterized anchors that move
//! with placement edits and invalidate on source edits (plan section 7).
use cam_core::{
    contours::{ContourCatalogue, ContourRole},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{CamJob, ContourAnchor, ContourSide, SetupSettings, StockSetup},
    svg::{ImportOptions, Placement},
};

const WINDING_A: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="shape" fill-rule="evenodd" d="M0 0h40v30h-40z M5 5h10v10h-10z M25 15h10v10h-10z"/></svg>"#;
/// The same artwork with every subpath traversed backwards.
const WINDING_B: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="shape" fill-rule="evenodd" d="M0 0v30h40v-30z M5 5v10h10v-10z M25 15v10h10v-10z"/></svg>"#;
/// The artwork with the outer boundary edited (one corner moved).
const EDITED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="shape" fill-rule="evenodd" d="M0 0h38v30h-38z M5 5h10v10h-10z M25 15h10v10h-10z"/></svg>"#;

fn job(svg: &str, placement: Placement) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "catalogue".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            placement,
            ..Default::default()
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: None,
            },
            ..Default::default()
        },
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances::default(),
        legacy_machine_profile: None,
    }
}

#[test]
fn reversed_source_winding_preserves_compensation_meaning() {
    let a = ContourCatalogue::build(&job(WINDING_A, Placement::default())).unwrap();
    let b = ContourCatalogue::build(&job(WINDING_B, Placement::default())).unwrap();
    assert_eq!(
        a.contours.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        b.contours.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        "contour identity survives winding reversal"
    );
    for (x, y) in a.contours.iter().zip(&b.contours) {
        assert_eq!(x.id, y.id);
        assert_eq!(x.role, y.role, "{} keeps its role", x.id);
        assert_eq!(x.parent_contour_id, y.parent_contour_id);
        assert_eq!(x.suggested_side(), y.suggested_side());
        // Canonical geometry (CCW from the smallest vertex) is the substrate
        // explicit sides compensate against: identical for either winding.
        assert_eq!(x.vertices, y.vertices, "{} canonical vertices", x.id);
        assert_eq!(x.source_fingerprint, y.source_fingerprint);
        let area = x
            .enclosed_region(cam_core::geometry::Grid::new(0.001, 50.).unwrap())
            .unwrap()
            .area_mm2();
        let expected = match x.role {
            ContourRole::Outer => 40. * 30.,
            ContourRole::Hole => 10. * 10.,
            ContourRole::Open => unreachable!("closed contours only"),
        };
        assert!(
            (area - expected).abs() < 1e-6,
            "{} encloses {area} mm2, expected {expected}",
            x.id
        );
    }
    // One outer, two holes, explicit roles and lineage.
    let outer = a.contour("shape-0-outer").unwrap();
    assert_eq!(outer.role, ContourRole::Outer);
    assert_eq!(outer.suggested_side(), ContourSide::Outside);
    let hole0 = a.contour("shape-0-hole-0").unwrap();
    let hole1 = a.contour("shape-0-hole-1").unwrap();
    assert_eq!(hole0.role, ContourRole::Hole);
    assert_eq!(hole0.parent_contour_id.as_deref(), Some("shape-0-outer"));
    assert_eq!(hole1.role, ContourRole::Hole);
    // Hole identity is page-space deterministic. The SVG page Y axis flips
    // on import, so the page-space (5,5) hole occupies setup (5,15)-(15,25)
    // and precedes the page-space (25,15) hole at setup (25,5)-(35,15).
    assert_eq!(hole0.vertices[0], Point::new(5., 15.));
    assert_eq!(hole1.vertices[0], Point::new(25., 5.));
    assert_eq!(hole0.suggested_side(), ContourSide::Inside);
}

#[test]
fn selecting_one_hole_does_not_select_every_boundary() {
    let catalogue = ContourCatalogue::build(&job(WINDING_A, Placement::default())).unwrap();
    // Exact contour selection: one hole id yields exactly that hole, never
    // the enclosing boundary or the sibling hole (contrast component union).
    let selected = catalogue.select(&["shape-0-hole-1".to_string()]).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, "shape-0-hole-1");
    assert_eq!(selected[0].role, ContourRole::Hole);
    let outer_only = catalogue.select(&["shape-0-outer".to_string()]).unwrap();
    assert_eq!(outer_only.len(), 1);
    assert_eq!(outer_only[0].role, ContourRole::Outer);
    // Unknown references are located errors, not silent skips.
    let error = catalogue
        .select(&["shape-0-hole-9".to_string()])
        .unwrap_err();
    assert_eq!(error.code, "CONTOUR_REFERENCE");
}

#[test]
fn placement_moves_anchors_correctly() {
    let identity = ContourCatalogue::build(&job(WINDING_A, Placement::default())).unwrap();
    let outer = identity.contour("shape-0-outer").unwrap();
    let anchor = ContourAnchor {
        contour_id: outer.id.clone(),
        source_geometry_fingerprint: outer.source_fingerprint.clone(),
        fraction_along_source_contour: 0.25,
    };
    let resolved = identity.resolve_anchor(&anchor).unwrap();
    // Fraction zero addresses the canonical start vertex exactly.
    let start = identity
        .resolve_anchor(&ContourAnchor {
            contour_id: outer.id.clone(),
            source_geometry_fingerprint: outer.source_fingerprint.clone(),
            fraction_along_source_contour: 0.,
        })
        .unwrap();
    assert_eq!(start.point, outer.vertices[0]);

    // Same artwork, placement: scale 2, rotate 90 degrees about the origin.
    let moved = ContourCatalogue::build(&job(
        WINDING_A,
        Placement {
            origin_mm: Point::new(0., 0.),
            scale: 2.,
            rotation_deg: 90.,
        },
    ))
    .unwrap();
    let moved_resolved = moved.resolve_anchor(&anchor).unwrap();
    let expected = Point::new(-2. * resolved.point.y, 2. * resolved.point.x);
    assert!(
        moved_resolved.point.distance(expected) < 1e-6,
        "anchor moved with the artwork: {:?} != {expected:?}",
        moved_resolved.point
    );
    // The fraction still addresses the same source location: the inverse
    // transform of the moved point is the pre-placement anchor point.
    let pre = |p: Point| Point::new(p.y / 2., -p.x / 2.);
    let back = pre(moved_resolved.point);
    assert!(
        back.distance(resolved.point) < 1e-6,
        "same source fraction: {back:?} vs {:?}",
        resolved.point
    );
    // Tangents rotate with the placement (90 degrees CCW, unit length).
    let t = moved_resolved.tangent;
    let tlen = (t.x * t.x + t.y * t.y).sqrt();
    assert!((tlen - 1.).abs() < 1e-9);
    let dot = t.x * resolved.tangent.x + t.y * resolved.tangent.y;
    assert!(dot.abs() < 1e-9, "tangent rotated by 90 degrees");
}

#[test]
fn source_geometry_edits_leave_anchors_unresolved() {
    let catalogue = ContourCatalogue::build(&job(WINDING_A, Placement::default())).unwrap();
    let outer = catalogue.contour("shape-0-outer").unwrap();
    let anchor = ContourAnchor {
        contour_id: "shape-0-outer".into(),
        source_geometry_fingerprint: outer.source_fingerprint.clone(),
        fraction_along_source_contour: 0.5,
    };
    assert!(catalogue.resolve_anchor(&anchor).is_ok());
    let edited = ContourCatalogue::build(&job(EDITED, Placement::default())).unwrap();
    let error = edited.resolve_anchor(&anchor).unwrap_err();
    assert_eq!(error.code, "CONTOUR_ANCHOR_UNRESOLVED");
    assert!(error.message.contains("reattach"));
    // An unknown contour id is a different, located failure.
    let ghost = ContourAnchor {
        contour_id: "ghost/outer".into(),
        source_geometry_fingerprint: outer.source_fingerprint.clone(),
        fraction_along_source_contour: 0.,
    };
    let error = catalogue.resolve_anchor(&ghost).unwrap_err();
    assert_eq!(error.code, "CONTOUR_REFERENCE");
}

#[test]
fn jobs_without_a_source_have_no_catalogue() {
    let mut source_free = job(WINDING_A, Placement::default());
    source_free.source = None;
    let error = ContourCatalogue::build(&source_free).unwrap_err();
    assert_eq!(error.code, "PROJECT_SOURCE_REQUIRED");
}
