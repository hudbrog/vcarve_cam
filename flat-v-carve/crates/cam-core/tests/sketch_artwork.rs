use cam_core::{
    geometry::Point,
    project::{
        self,
        v5::{
            self, ArtworkContent, GeometryRefKind as Kind, OperationSettingsV5, artwork, commands,
        },
    },
    sketch::{materialize::materialize, *},
    svg::{ImportOptions, Placement},
};

fn entity(id: &str, geometry: SketchGeometry) -> Entity {
    Entity {
        id: id.into(),
        label: None,
        construction: false,
        geometry,
    }
}
fn wire(id: &str, ids: &[&str], closed: bool) -> Wire {
    Wire {
        id: id.into(),
        label: Some(id.into()),
        edges: ids
            .iter()
            .map(|id| WireEdge {
                entity_id: (*id).into(),
                reversed: false,
            })
            .collect(),
        closed,
    }
}
fn line(id: &str, a: (f64, f64), b: (f64, f64)) -> Entity {
    entity(
        id,
        SketchGeometry::Line {
            start: Point::new(a.0, a.1),
            end: Point::new(b.0, b.1),
        },
    )
}
fn rectangle() -> SketchDocument {
    SketchDocument {
        entities: vec![
            line("a", (2., 2.), (18., 2.)),
            line("b", (18., 2.), (18., 12.)),
            line("c", (18., 12.), (2., 12.)),
            line("d", (2., 12.), (2., 2.)),
        ],
        wires: vec![wire("plate", &["a", "b", "c", "d"], true)],
        ..Default::default()
    }
}

#[test]
fn builtin_references_roundtrip_without_entering_cam_geometry_and_are_immutable() {
    let mut d = rectangle();
    let baseline = artwork::materialize_item(&item(d.clone()), None).unwrap();
    d.constraints.push(Constraint {
        id: "offset".into(),
        reference: false,
        definition: ConstraintKind::HorizontalDistance {
            a: PointRef {
                entity_id: "a".into(),
                point: PointRole::Start,
            },
            b: PointRef {
                entity_id: Y_AXIS_ID.into(),
                point: PointRole::Start,
            },
            value_mm: 2.,
        },
    });
    d.require_valid_solution().unwrap();
    let roundtrip: SketchDocument =
        serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(d, roundtrip);
    let result = artwork::materialize_item(&item(roundtrip), None).unwrap();
    assert_eq!(result.provenance, baseline.provenance);
    let contours = |m: &cam_core::sketch::materialize::MaterializedArtwork| {
        m.catalogue
            .contours
            .iter()
            .map(|c| (c.id.clone(), c.vertices.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(contours(&result), contours(&baseline));
    assert_eq!(
        result.catalogue.points.len(),
        baseline.catalogue.points.len()
    );
    assert_eq!(
        result.catalogue.open_chains.len(),
        baseline.catalogue.open_chains.len()
    );
    assert_eq!(d.entities.len(), 4);
    assert_eq!(d.point(&origin_ref()).unwrap(), Point::new(0., 0.));
    let mut invalid = d.clone();
    invalid.wires[0].edges[0].entity_id = X_AXIS_ID.into();
    assert!(invalid.validate_structure().is_err());
    let mut invalid = d.clone();
    invalid.entities.push(reference_entities()[0].clone());
    assert!(invalid.validate_structure().is_err());
    for definition in [
        ConstraintKind::Length {
            line: X_AXIS_ID.into(),
            value_mm: 1.,
        },
        ConstraintKind::FixedPoint {
            point: origin_ref(),
            position: Point::new(1., 0.),
        },
        ConstraintKind::Fixed {
            entity: X_AXIS_ID.into(),
            target: SketchGeometry::Line {
                start: Point::new(2., 0.),
                end: Point::new(3., 0.),
            },
        },
        ConstraintKind::PointOnLine {
            point: origin_ref(),
            line: "@unknown".into(),
        },
    ] {
        let mut invalid = d.clone();
        invalid.constraints.push(Constraint {
            id: "invalid".into(),
            reference: false,
            definition,
        });
        assert!(invalid.validate_structure().is_err());
    }
}
fn circle(doc: &mut SketchDocument, id: &str, x: f64, y: f64, r: f64) {
    doc.entities.push(entity(
        id,
        SketchGeometry::Circle {
            center: Point::new(x, y),
            radius_mm: r,
        },
    ));
    doc.wires.push(wire(id, &[id], true));
}
fn item(doc: SketchDocument) -> v5::ArtworkItem {
    v5::ArtworkItem {
        id: v5::ArtworkItemId("sketch".into()),
        name: "Sketch".into(),
        content: ArtworkContent::Sketch(doc),
        import_settings: Default::default(),
        placement: Default::default(),
    }
}
fn fixture() -> v5::CamJobV5 {
    v5::CamJobV5::from_json(include_str!("../../../fixtures/v5/full-job.json")).unwrap()
}
fn bound_job() -> v5::CamJobV5 {
    let mut j = fixture();
    j.setup.stock.xy = Some(project::RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 100.,
        length_mm: 60.,
    });
    j.tolerances.motion_tolerance_mm = Some(0.01);
    j.tolerances.verification_tolerance_mm = Some(0.05);
    let mut drawing = rectangle();
    drawing.entities.push(line("knife", (25., 5.), (35., 5.)));
    drawing.wires.push(wire("knife", &["knife"], false));
    j.artwork = vec![item(drawing)];
    let r = |kind, id: &str| v5::GeometryRef {
        artwork_item_id: j.artwork[0].id.clone(),
        kind,
        local_geometry_id: id.into(),
        source_revision: j.artwork[0].source_revision().unwrap(),
    };
    for op in &mut j.operations {
        match &mut op.settings {
            OperationSettingsV5::FlatVcarve(s) => {
                s.endmill.spindle_direction = Some(project::SpindleDirection::Clockwise);
                s.vbit.spindle_direction = Some(project::SpindleDirection::Clockwise);
                s.wall_allowance_mm = Some(0.2);
                s.rough = Some(project::FlatVcarveRoughSettings {
                    strategy: cam_core::pocket::ClearingStrategy::DepthDependent,
                    entry: cam_core::pocket::EntryStrategy::Plunge,
                    max_layers: 32,
                    max_loops_per_layer: 512,
                    max_motions: 100_000,
                });
                s.components = vec![r(Kind::FilledComponent, "region-plate")]
            }
            OperationSettingsV5::Profile(s) => {
                s.assignment.spindle_direction = Some(project::SpindleDirection::Clockwise);
                s.stepdown_mm = Some(1.);
                s.contours[0].geometry = r(Kind::ClosedContour, "contour-plate");
                s.start = Default::default();
                s.tabs = None;
            }
            OperationSettingsV5::DragKnife(s) => {
                s.chains = vec![r(Kind::Centerline, "chain-knife")];
                s.start = Default::default();
            }
            OperationSettingsV5::Face(s) => {
                s.assignment.spindle_direction = Some(project::SpindleDirection::Clockwise)
            }
            _ => {}
        }
    }
    j
}

#[test]
fn apply_report_distinguishes_continuity_split_hole_membership_and_removal() {
    use commands::{BindingChange as B, OutputChange as O};
    let j = bound_job();
    let id = &j.artwork[0].id;
    let ArtworkContent::Sketch(original) = &j.artwork[0].content else {
        panic!()
    };
    let mut resized = original.clone();
    for entity in &mut resized.entities {
        if let SketchGeometry::Line { start, end } = &mut entity.geometry {
            if start.x == 18. {
                start.x = 20.;
            }
            if end.x == 18. {
                end.x = 20.;
            }
        }
    }
    let applied = commands::apply_sketch_with_report(&j, id, resized).unwrap();
    assert!(applied.report.geometry_changed);
    assert!(applied.report.outputs.is_empty());
    assert_eq!(applied.report.preserved_count(), 3);
    assert_eq!(applied.report.repair_count(), 0);
    assert!(
        applied.outcome.issues.is_empty(),
        "{:?}",
        applied.outcome.issues
    );

    // Same wire ID with a newly split edge is not continuity evidence.
    let mut split = original.clone();
    split.entities[0] = line("a", (2., 2.), (10., 2.));
    split.entities.push(line("a2", (10., 2.), (18., 2.)));
    split.wires[0].edges.insert(
        1,
        WireEdge {
            entity_id: "a2".into(),
            reversed: false,
        },
    );
    let split = commands::apply_sketch_with_report(&j, id, split).unwrap();
    assert_eq!(split.report.repair_count(), 2);
    assert!(
        split
            .report
            .bindings
            .iter()
            .filter(|b| b.kind != Kind::Centerline)
            .all(|b| b.change == B::TopologyChanged)
    );
    assert!(
        split
            .report
            .outputs
            .iter()
            .any(|o| o.local_id == "contour-plate" && o.change == O::TopologyChanged)
    );

    // A new hole changes filled membership, while its outer contour survives.
    let mut holed = original.clone();
    circle(&mut holed, "hole", 10., 7., 2.);
    let holed = commands::apply_sketch_with_report(&j, id, holed).unwrap();
    assert_eq!(holed.report.repair_count(), 1);
    assert!(
        holed
            .report
            .bindings
            .iter()
            .any(|b| b.kind == Kind::FilledComponent && b.change == B::TopologyChanged)
    );
    assert!(
        holed
            .report
            .bindings
            .iter()
            .any(|b| b.kind == Kind::ClosedContour && b.change == B::Preserved)
    );
    assert!(
        holed
            .report
            .outputs
            .iter()
            .any(|o| o.local_id == "center-hole" && o.change == O::Added)
    );

    let empty = commands::apply_sketch_with_report(&j, id, SketchDocument::default()).unwrap();
    assert_eq!(empty.report.repair_count(), 3);
    assert!(empty.report.bindings.iter().all(|b| b.change == B::Removed));
    assert!(empty.report.outputs.iter().all(|o| o.change == O::Removed));
    assert_eq!(empty.outcome.job.setup, j.setup);
    assert_eq!(j, bound_job()); // Commands did not mutate their input.
}

#[test]
fn start_and_manual_tabs_advance_only_on_unchanged_boundaries() {
    use commands::BindingChange as B;
    let mut j = bound_job();
    let id = j.artwork[0].id.clone();
    let m = artwork::materialize_item(&j.artwork[0], None).unwrap();
    let OperationSettingsV5::Profile(s) = &mut j.operations[2].settings else {
        panic!()
    };
    let anchor = v5::ContourAnchorV5 {
        geometry: s.contours[0].geometry.clone(),
        source_geometry_fingerprint: m
            .catalogue
            .contour("contour-plate")
            .unwrap()
            .source_fingerprint
            .clone(),
        fraction_along_source_contour: 0.25,
    };
    s.start = v5::StartSelectionV5::Anchor(Box::new(anchor.clone()));
    s.tabs = Some(v5::TabSettingsV5 {
        height_mm: Some(0.5),
        width_mm: Some(2.),
        shape: Default::default(),
        placement: v5::TabPlacementV5::Manual {
            anchors: vec![anchor],
        },
    });
    let ArtworkContent::Sketch(original) = &j.artwork[0].content else {
        panic!()
    };
    let mut unrelated = original.clone();
    if let SketchGeometry::Line { end, .. } = &mut unrelated.entities[4].geometry {
        end.x += 1.;
    }
    let applied = commands::apply_sketch_with_report(&j, &id, unrelated).unwrap();
    assert_eq!(applied.report.repair_count(), 0);
    assert_eq!(applied.report.preserved_count(), 5);
    assert!(applied.outcome.issues.is_empty());

    let mut changed = original.clone();
    for e in &mut changed.entities[..4] {
        if let SketchGeometry::Line { start, end } = &mut e.geometry {
            start.x += 1.;
            end.x += 1.;
        }
    }
    let applied = commands::apply_sketch_with_report(&j, &id, changed).unwrap();
    assert_eq!(applied.report.repair_count(), 2);
    assert!(
        applied
            .report
            .bindings
            .iter()
            .filter(|b| b.change.needs_repair())
            .all(|b| b.change == B::BoundaryChanged)
    );
    assert!(
        applied
            .report
            .bindings
            .iter()
            .any(|b| b.field_path.ends_with("tabs.placement.anchors[0]"))
    );
    let OperationSettingsV5::Profile(s) = &applied.outcome.job.operations[2].settings else {
        panic!()
    };
    let OperationSettingsV5::Profile(old) = &j.operations[2].settings else {
        panic!()
    };
    assert_eq!(s.start, old.start);
    assert_eq!(s.tabs, old.tabs);
    let ArtworkContent::Sketch(model) = &applied.outcome.job.artwork[0].content else {
        panic!()
    };
    let reapplied =
        commands::apply_sketch_with_report(&applied.outcome.job, &id, model.clone()).unwrap();
    assert!(!reapplied.report.geometry_changed);
    assert_eq!(reapplied.report.repair_count(), 2);
    assert!(
        reapplied
            .report
            .bindings
            .iter()
            .filter(|b| b.change.needs_repair())
            .all(|b| b.change == B::AlreadyUnresolved)
    );
}

#[test]
fn svg_snapshot_preserves_units_holes_islands_open_wires_and_page_origin() {
    let mut doc = rectangle();
    circle(&mut doc, "hole", 10., 7., 3.);
    circle(&mut doc, "island", 10., 7., 1.);
    circle(&mut doc, "drill", 30., 10., 2.);
    doc.entities.push(entity(
        "arc",
        SketchGeometry::Arc {
            center: Point::new(-5., -5.),
            radius_mm: 3.,
            start_angle_rad: 0.,
            sweep_angle_rad: -std::f64::consts::PI,
        },
    ));
    doc.wires.push(wire("arc", &["arc"], false));
    let mut construction = line("construction", (0., 0.), (30., 30.));
    construction.construction = true;
    doc.entities.push(construction);
    doc.wires
        .push(wire("construction", &["construction"], false));
    let options = ImportOptions {
        placement: Placement {
            origin_mm: Point::new(3., 4.),
            scale: 1.5,
            rotation_deg: 17.,
        },
        ..Default::default()
    };
    let expected = materialize(&doc, &options, None).unwrap();
    let snapshot = export::to_svg(&doc, &options).unwrap();
    assert!(!snapshot.contains("id=\"construction\""));
    assert!(snapshot.contains("fill-rule=\"evenodd\""));
    assert!(snapshot.contains("<circle id=\"region-drill\""));
    let mut options = options;
    options.placement.origin_mm.x -= doc.frame.min.x;
    options.placement.origin_mm.y -= doc.frame.min.y;
    let imported = cam_core::svg::import_svg(&snapshot, &options, None).unwrap();
    assert!((imported.page_width_mm - doc.frame.width_mm).abs() < 1e-9);
    assert!((imported.page_height_mm - doc.frame.height_mm).abs() < 1e-9);
    assert!((imported.selected.area_mm2() - expected.geometry.selected.area_mm2()).abs() < 0.01);
    assert!(
        imported
            .selected
            .boolean(
                cam_core::geometry::BooleanOp::Xor,
                &expected.geometry.selected
            )
            .unwrap()
            .area_mm2()
            < 0.01
    );
    assert_eq!(
        imported.selected.rings().len(),
        expected.geometry.selected.rings().len()
    );
    let actual_chain = imported.chains.iter().find(|c| !c.closed).unwrap();
    let expected_chain = expected.geometry.chains.iter().find(|c| !c.closed).unwrap();
    assert_eq!(actual_chain.points.len(), expected_chain.points.len());
    assert!(
        actual_chain
            .points
            .iter()
            .zip(&expected_chain.points)
            .all(|(a, b)| a.distance(*b) < 1e-10)
    );
    let mut invalid = doc;
    invalid.constraints.push(Constraint {
        id: "bad".into(),
        reference: false,
        definition: ConstraintKind::Radius {
            curve: "drill".into(),
            value_mm: 8.,
        },
    });
    assert!(export::to_svg(&invalid, &options).is_err());
}

#[test]
fn strict_saved_model_roundtrip_and_version_gate() {
    let sample =
        v5::CamJobV5::from_json(include_str!("../../../fixtures/v5/sketch-job.json")).unwrap();
    assert!(
        artwork::resolve_artwork_item(&sample.artwork[0])
            .unwrap()
            .import_error
            .is_none()
    );
    let j = bound_job();
    let text = j.to_json().unwrap();
    assert_eq!(v5::CamJobV5::from_json(&text).unwrap(), j);
    assert!(text.contains("\"sketch_version\": 1"));
    assert!(!text.contains("arael"));
    let mut json = serde_json::to_value(&j).unwrap();
    json["artwork"][0]["content"]["sketch_version"] = 2.into();
    assert!(v5::CamJobV5::from_json(&json.to_string()).is_err());
    json["artwork"][0]["content"]["sketch_version"] = 1.into();
    json["artwork"][0]["content"]["solver_cache"] = true.into();
    assert!(v5::CamJobV5::from_json(&json.to_string()).is_err());
}
#[test]
fn structure_and_numeric_validation_are_separate() {
    let mut d = rectangle();
    d.constraints.push(Constraint {
        id: "length".into(),
        reference: false,
        definition: ConstraintKind::Length {
            line: "a".into(),
            value_mm: 20.,
        },
    });
    d.validate_structure().unwrap();
    assert!(!d.validate_solution().unwrap().valid());
    assert_eq!(
        materialize(&d, &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "SKETCH_UNSATISFIED"
    );
    d.constraints[0].reference = true;
    let report = d.validate_solution().unwrap();
    assert!(report.valid());
    assert_eq!(report.constraints[0].measured, Some(16.));
    d.constraints[0].definition = ConstraintKind::Horizontal { line: "a".into() };
    assert!(d.validate_structure().is_err());
    d.constraints.clear();
    d.entities[0].id = "b".into();
    assert!(d.validate_structure().is_err());
    let mut d = rectangle();
    d.wires[0].edges[0].entity_id = "missing".into();
    assert!(d.validate_structure().is_err());
    let mut d = rectangle();
    d.entities[0].geometry = SketchGeometry::Circle {
        center: Point::new(0., 0.),
        radius_mm: f64::NAN,
    };
    assert!(d.validate_structure().is_err());
    let mut d = rectangle();
    d.entities.resize(MAX_ENTITIES + 1, d.entities[0].clone());
    assert_eq!(d.validate_structure().unwrap_err().code, "SKETCH_LIMIT");
}
#[test]
fn analytic_geometry_placement_and_nested_identity() {
    let mut d = rectangle();
    circle(&mut d, "hole-a", 6., 7., 1.);
    circle(&mut d, "hole-b", 13., 7., 1.);
    let options = ImportOptions {
        placement: Placement {
            origin_mm: Point::new(3., 4.),
            scale: 2.,
            rotation_deg: 90.,
        },
        ..Default::default()
    };
    let m = materialize(&d, &options, None).unwrap();
    assert_eq!(m.catalogue.contours.len(), 3);
    assert_eq!(m.geometry.sources.len(), 1);
    assert!(
        (m.geometry.selected.area_mm2() - (160. - 2. * std::f64::consts::PI) * 4.).abs() < 0.02
    );
    let point = m.catalogue.point("center-hole-a").unwrap();
    assert!(point.center.distance(Point::new(-6., 6.)) < 1e-10);
    assert_eq!(point.diameter_mm, 4.);
    assert_eq!(
        m.catalogue
            .contour("contour-hole-a")
            .unwrap()
            .parent_contour_id
            .as_deref(),
        Some("contour-plate")
    );
    let mut reordered = d.clone();
    reordered.wires.reverse();
    reordered.entities.reverse();
    assert_eq!(
        d.geometry_digest().unwrap(),
        reordered.geometry_digest().unwrap()
    );
    let reordered = materialize(&reordered, &options, None).unwrap();
    assert_eq!(
        m.provenance(Kind::FilledComponent, "region-plate"),
        reordered.provenance(Kind::FilledComponent, "region-plate")
    );
    for e in &mut d.entities {
        if let SketchGeometry::Circle { center, .. } = &mut e.geometry {
            center.x = 19. - center.x;
        }
    }
    let swapped = materialize(&d, &options, None).unwrap();
    assert_eq!(
        m.provenance(Kind::ClosedContour, "contour-hole-a"),
        swapped.provenance(Kind::ClosedContour, "contour-hole-a")
    );
}
#[test]
fn topology_gate_rejects_open_joins_crossings_and_unwired_edges() {
    let mut d = rectangle();
    if let SketchGeometry::Line { end, .. } = &mut d.entities[0].geometry {
        end.x += 0.1;
    }
    assert_eq!(
        materialize(&d, &Default::default(), None).unwrap_err().code,
        "SKETCH_OPEN_JOIN"
    );
    let mut d = rectangle();
    d.entities.push(line("loose", (1., 1.), (2., 3.)));
    assert_eq!(
        materialize(&d, &Default::default(), None).unwrap_err().code,
        "SKETCH_WIRE"
    );
    let mut d = rectangle();
    circle(&mut d, "crossing", 2., 7., 2.);
    assert!(materialize(&d, &Default::default(), None).is_err());
    let mut d = rectangle();
    d.entities[0].construction = true;
    assert_eq!(
        materialize(&d, &Default::default(), None).unwrap_err().code,
        "SKETCH_WIRE"
    );
    let mut d = rectangle();
    d.entities.iter_mut().for_each(|e| e.construction = true);
    assert!(
        materialize(&d, &Default::default(), None)
            .unwrap()
            .catalogue
            .contours
            .is_empty()
    );
    assert!(
        materialize(&SketchDocument::default(), &Default::default(), None)
            .unwrap()
            .catalogue
            .open_chains
            .is_empty()
    );
}
#[test]
fn signed_arc_and_open_chain_use_shared_flattening() {
    let d = SketchDocument {
        entities: vec![entity(
            "arc",
            SketchGeometry::Arc {
                center: Point::new(-5., -5.),
                radius_mm: 3.,
                start_angle_rad: 0.,
                sweep_angle_rad: -std::f64::consts::PI,
            },
        )],
        wires: vec![wire("arc-wire", &["arc"], false)],
        ..Default::default()
    };
    let m = materialize(&d, &Default::default(), None).unwrap();
    assert!(m.geometry.sources.is_empty());
    let c = m.catalogue.chain("chain-arc-wire").unwrap();
    assert!(c.vertices.len() > 10);
    assert!(c.vertices[0].distance(Point::new(-2., -5.)) < 1e-9);
    assert!(c.vertices.iter().any(|p| p.y < -7.99));
    assert!(m.catalogue.points.is_empty());
}
#[test]
fn apply_preserves_proven_selections_but_never_stale_or_changed_anchors() {
    let mut j = bound_job();
    let id = j.artwork[0].id.clone();
    let before = artwork::materialize_item(&j.artwork[0], None).unwrap();
    let OperationSettingsV5::Profile(s) = &mut j.operations[2].settings else {
        panic!()
    };
    s.start = v5::StartSelectionV5::Anchor(Box::new(v5::ContourAnchorV5 {
        geometry: s.contours[0].geometry.clone(),
        source_geometry_fingerprint: before
            .catalogue
            .contour("contour-plate")
            .unwrap()
            .source_fingerprint
            .clone(),
        fraction_along_source_contour: 0.25,
    }));
    let anchor = s.start.clone();
    let mut changed = rectangle();
    for e in &mut changed.entities {
        if let SketchGeometry::Line { start, end } = &mut e.geometry {
            start.x += 1.;
            end.x += 1.;
        }
    }
    let applied = commands::apply_sketch(&j, &id, changed.clone()).unwrap();
    assert_eq!(applied.job.setup, j.setup);
    let OperationSettingsV5::Profile(s) = &applied.job.operations[2].settings else {
        panic!()
    };
    assert_eq!(s.start, anchor);
    assert_eq!(
        s.contours[0].geometry.source_revision,
        applied.job.artwork[0].source_revision().unwrap()
    );
    assert!(!applied.issues.is_empty());
    let OperationSettingsV5::DragKnife(s) = &mut j.operations[3].settings else {
        panic!()
    };
    s.chains[0].source_revision.content_digest = "stale".into();
    let applied = commands::apply_sketch(&j, &id, changed).unwrap();
    let OperationSettingsV5::DragKnife(s) = &applied.job.operations[3].settings else {
        panic!()
    };
    assert_eq!(s.chains[0].source_revision.content_digest, "stale");
    let mut removed = rectangle();
    removed.wires[0].id = "replacement".into();
    let applied = commands::apply_sketch(&j, &id, removed).unwrap();
    let OperationSettingsV5::FlatVcarve(s) = &applied.job.operations[1].settings else {
        panic!()
    };
    assert_eq!(
        s.components[0].source_revision,
        j.artwork[0].source_revision().unwrap()
    );
}
#[test]
fn annotation_identity_differs_from_machining_and_invalid_constraints_block_retained_identity() {
    use cam_core::sequence::OperationPlanV5;
    let j = bound_job();
    let scope = v5::ReadinessScope::AllEnabled;
    let hash = OperationPlanV5::machining_identity(&j, &scope).unwrap();
    let mut changed = j.clone();
    let old_cache = changed.artwork[0].authored_revision().unwrap();
    let ArtworkContent::Sketch(s) = &mut changed.artwork[0].content else {
        panic!()
    };
    s.entities[0].label = Some("Renamed".into());
    s.constraints.push(Constraint {
        id: "measured".into(),
        reference: true,
        definition: ConstraintKind::Length {
            line: "a".into(),
            value_mm: 99.,
        },
    });
    assert_eq!(
        OperationPlanV5::machining_identity(&changed, &scope).unwrap(),
        hash
    );
    assert_ne!(changed.artwork[0].authored_revision().unwrap(), old_cache);
    let ArtworkContent::Sketch(s) = &mut changed.artwork[0].content else {
        panic!()
    };
    s.constraints[0].reference = false;
    assert!(changed.to_json().is_ok());
    assert!(
        artwork::resolve_artwork_item(&changed.artwork[0])
            .unwrap()
            .import_error
            .is_some()
    );
    assert!(OperationPlanV5::machining_identity(&changed, &scope).is_err());
}
#[test]
fn filled_selection_matches_svg_and_plans_profile_knife_and_carving() {
    let j = bound_job();
    let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
        &j,
        &v5::ReadinessScope::AllEnabled,
        &Default::default(),
    )
    .unwrap();
    assert!(
        plan.operation_results
            .iter()
            .all(|r| r.generation_status == cam_core::sequence::GenerationStatus::Complete),
        "{:?}",
        plan.generation_diagnostics
    );
    let sketch = artwork::materialize_item(&j.artwork[0], Some(&["region-plate".into()])).unwrap();
    let svg=cam_core::svg::import_svg("<svg width='40mm' height='30mm' viewBox='0 0 40 30'><rect x='2' y='18' width='16' height='10'/></svg>",&Default::default(),None).unwrap();
    assert_eq!(sketch.geometry.selected.area_mm2(), svg.selected.area_mm2());
    assert_eq!(sketch.geometry.selected.rings_mm(), svg.selected.rings_mm());
}
#[test]
fn empty_draft_addition_is_atomic_and_never_resizes_stock() {
    let j = fixture();
    let added = commands::add_sketch(
        &j,
        "New sketch".into(),
        SketchDocument::default(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(added.job.setup, j.setup);
    assert_eq!(added.job.artwork.len(), j.artwork.len() + 1);
    let bad = SketchDocument {
        sketch_version: 99,
        ..Default::default()
    };
    assert!(
        commands::add_sketch(
            &j,
            "bad".into(),
            bad,
            Default::default(),
            Default::default()
        )
        .is_err()
    );
}

#[test]
fn sketch_pocket_and_analytic_drill_reach_existing_planners() {
    for drill in [false, true] {
        let mut j = bound_job();
        j.operations.clear();
        let ArtworkContent::Sketch(d) = &mut j.artwork[0].content else {
            panic!()
        };
        circle(d, "spot", 30., 20., 2.);
        j = commands::add_operation(
            &j,
            if drill {
                commands::NewOperationKind::Drill
            } else {
                commands::NewOperationKind::Pocket
            },
            "cut",
            "Cut",
        )
        .unwrap()
        .job;
        let pick = artwork::GeometryPick {
            artwork_item_id: j.artwork[0].id.clone(),
            kind: if drill {
                Kind::Point
            } else {
                Kind::FilledComponent
            },
            local_geometry_id: if drill { "center-spot" } else { "region-plate" }.into(),
        };
        j = if drill {
            commands::set_point_selection(&j, "cut", &[pick])
        } else {
            commands::set_component_selection(&j, "cut", &[pick])
        }
        .unwrap()
        .job;
        let assignment = match &mut j.operations[0].settings {
            OperationSettingsV5::Pocket(s) => {
                s.bottom.offset_mm = -1.;
                s.direction = Some(project::CutDirection::Climb);
                &mut s.assignment
            }
            OperationSettingsV5::Drill(s) => &mut s.assignment,
            _ => panic!(),
        };
        assignment.spindle_rpm = Some(6000.);
        assignment.spindle_direction = Some(project::SpindleDirection::Clockwise);
        assignment.cutting_feed_mm_min = Some(300.);
        assignment.plunge_feed_mm_min = Some(100.);
        assignment.max_stepdown_mm = Some(1.);
        assignment.stepover_mm = Some(1.5);
        let tool = j
            .tools
            .iter_mut()
            .find(|t| t.id == assignment.tool_id)
            .unwrap();
        tool.geometry = Some(if drill {
            project::ToolGeometry::Drill(cam_core::model::DrillSpec {
                diameter_mm: 3.,
                tip_angle_deg: 118.,
                cutting_length_mm: 20.,
            })
        } else {
            project::ToolGeometry::Endmill(project::EndmillGeometry {
                diameter_mm: 3.,
                cutting_length_mm: 10.,
            })
        });
        tool.capabilities = project::ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: Some(true),
        };
        let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
            &j,
            &v5::ReadinessScope::AllEnabled,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            cam_core::sequence::GenerationStatus::Complete,
            "{:?}",
            plan.generation_diagnostics
        );
        assert!(cam_core::checks::check_plan_v5(&plan).unwrap().export_ready);
    }
}

#[test]
fn closed_wire_accepts_small_solver_residual_without_a_collapsed_closing_edge() {
    let mut d = rectangle();
    if let SketchGeometry::Line { end, .. } = &mut d.entities[3].geometry {
        end.y += 0.0000004;
    }
    let m = materialize(&d, &Default::default(), None).unwrap();
    assert!((m.geometry.selected.area_mm2() - 160.).abs() < 0.001);
    let mut closed = rectangle();
    closed.wires[0].closed = false;
    assert!(materialize(&closed, &Default::default(), None).is_err());
}

#[test]
fn tangent_slot_with_reversed_arc_preserves_center_geometry() {
    let mut d = SketchDocument {
        entities: vec![
            line("bottom", (5., 2.), (15., 2.)),
            entity(
                "right",
                SketchGeometry::Arc {
                    center: Point::new(15., 5.),
                    radius_mm: 3.,
                    start_angle_rad: -std::f64::consts::FRAC_PI_2,
                    sweep_angle_rad: std::f64::consts::PI,
                },
            ),
            line("top", (15., 8.), (5., 8.)),
            entity(
                "left",
                SketchGeometry::Arc {
                    center: Point::new(5., 5.),
                    radius_mm: 3.,
                    start_angle_rad: -std::f64::consts::FRAC_PI_2,
                    sweep_angle_rad: -std::f64::consts::PI,
                },
            ),
        ],
        wires: vec![wire("slot", &["bottom", "right", "top", "left"], true)],
        ..Default::default()
    };
    d.wires[0].edges[3].reversed = true;
    let m = materialize(&d, &Default::default(), None).unwrap();
    assert!((m.geometry.selected.area_mm2() - (60. + 9. * std::f64::consts::PI)).abs() < 0.01);
    assert_eq!(m.catalogue.contours.len(), 1);
    assert!(m.catalogue.points.is_empty());
}

#[test]
fn nested_island_and_hole_deletion_change_component_provenance() {
    let mut d = rectangle();
    circle(&mut d, "hole", 10., 7., 3.);
    circle(&mut d, "island", 10., 7., 1.);
    let m = materialize(&d, &Default::default(), None).unwrap();
    assert_eq!(m.geometry.sources.len(), 2);
    assert_eq!(
        m.catalogue
            .contour("contour-island")
            .unwrap()
            .parent_contour_id
            .as_deref(),
        Some("contour-hole")
    );
    assert!((m.geometry.selected.area_mm2() - (160. - 8. * std::f64::consts::PI)).abs() < 0.01);
    let mut j = bound_job();
    j.artwork[0] = item(d);
    let revision = j.artwork[0].source_revision().unwrap();
    let OperationSettingsV5::FlatVcarve(s) = &mut j.operations[1].settings else {
        panic!()
    };
    s.components[0].source_revision = revision.clone();
    let applied = commands::apply_sketch(&j, &j.artwork[0].id, rectangle()).unwrap();
    let OperationSettingsV5::FlatVcarve(s) = &applied.job.operations[1].settings else {
        panic!()
    };
    assert_eq!(s.components[0].source_revision, revision);
    assert_ne!(revision, applied.job.artwork[0].source_revision().unwrap());
}
