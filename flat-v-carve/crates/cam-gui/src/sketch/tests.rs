use super::*;
use cam_core::{
    project::v5::{ArtworkContent, ArtworkItem, SvgInterpretation, artwork::materialize_item},
    svg::Placement,
};
fn editor() -> Editor {
    Editor::new(SavedDraft {
        constraint_form: Default::default(),
        target: None,
        source_revision: None,
        name: "Test".into(),
        model: SketchDocument::default(),
        had_document: true,
    })
}
fn click(e: &mut Editor, x: f64, y: f64) {
    e.click(Point::new(x, y)).unwrap();
}
fn materialized(e: &Editor) -> cam_core::sketch::materialize::MaterializedArtwork {
    materialize_item(
        &ArtworkItem {
            id: ArtworkItemId("test".into()),
            name: "Test".into(),
            content: ArtworkContent::Sketch(e.saved.model.clone()),
            placement: Placement::default(),
            import_settings: SvgInterpretation::default(),
        },
        None,
    )
    .unwrap()
}
#[test]
fn draw_outline_hole_and_construction_then_undo_redo() {
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    click(&mut e, 0., 0.);
    click(&mut e, 40., 30.);
    e.set_tool(Tool::Circle);
    click(&mut e, 10., 10.);
    click(&mut e, 12., 10.);
    e.construction = true;
    e.set_tool(Tool::Line);
    click(&mut e, -5., -5.);
    click(&mut e, 45., 35.);
    assert_eq!(e.saved.model.entities.len(), 6);
    let m = materialized(&e);
    assert!(!m.catalogue.contours.is_empty());
    let saved = e.saved.model.clone();
    e.undo();
    assert_eq!(e.saved.model.entities.len(), 5);
    e.redo();
    assert_eq!(e.saved.model, saved);
    let json = serde_json::to_string(&e.saved).unwrap();
    serde_json::from_str::<SavedDraft>(&json)
        .unwrap()
        .validate()
        .unwrap();
}
#[test]
fn polyline_closes_and_arc_joins_with_exact_analytic_endpoints() {
    let mut e = editor();
    e.set_tool(Tool::Polyline);
    for (x, y) in [(0., 0.), (20., 0.), (20., 10.), (0., 10.), (0., 0.)] {
        click(&mut e, x, y);
    }
    assert_eq!(e.saved.model.wires.len(), 1);
    assert!(e.saved.model.wires[0].closed);
    assert!(!e.pending());
    materialized(&e);
    let wire_id = e.saved.model.wires[0].id.clone();
    e.undo();
    assert!(!e.saved.model.wires[0].closed);
    e.redo();
    assert_eq!(e.saved.model.wires[0].id, wire_id);
    let mut e = editor();
    e.set_tool(Tool::Line);
    click(&mut e, 0., 0.);
    click(&mut e, 10., 0.);
    let wire_id = e.saved.model.wires[0].id.clone();
    e.set_tool(Tool::Arc);
    click(&mut e, 10., 0.);
    click(&mut e, 5., 5.);
    click(&mut e, 0., 0.);
    assert_eq!(e.saved.model.wires.len(), 1);
    assert!(e.saved.model.wires[0].closed);
    assert_eq!(e.saved.model.wires[0].id, wire_id);
    materialized(&e);
}
#[test]
fn selection_moves_whole_wire_and_protects_constraints() {
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    click(&mut e, 0., 0.);
    click(&mut e, 20., 10.);
    let id = e.saved.model.entities[0].id.clone();
    e.select_wire(Some(&id), false);
    assert_eq!(e.selected.len(), 4);
    let old = e.saved.model.clone();
    e.translate(&old, Point::new(2., 3.)).unwrap();
    e.commit(old).unwrap();
    materialized(&e);
    e.toggle_construction().unwrap();
    assert!(e.saved.model.entities.iter().all(|e| e.construction));
    materialized(&e);
    e.undo();
    e.select_wire(Some(&id), false);
    e.saved.model.constraints.push(Constraint {
        id: "fixed".into(),
        reference: false,
        definition: ConstraintKind::Fixed {
            entity: id,
            target: e.saved.model.entities[0].geometry.clone(),
        },
    });
    let old = e.saved.model.clone();
    assert!(e.translate(&old, Point::new(2., 3.)).is_err());
    assert!(e.delete().is_err());
    assert_eq!(e.saved.model, old);
}
#[test]
fn individual_selection_survives_panels_and_never_silently_expands_an_edit() {
    use constraints::Operand;
    let mut e = editor();
    e.set_tool(Tool::Rectangle);
    click(&mut e, 0., 0.);
    click(&mut e, 20., 10.);
    e.set_tool(Tool::Select);
    let id = e.saved.model.entities[0].id.clone();
    e.pick_operand(Some(Operand::Entity(id.clone())), false);
    let before = e.saved.model.clone();
    assert_eq!(e.selected.len(), 1);
    for visible in [true, false] {
        e.show_constraints = visible;
        e.show_geometry = visible;
        assert_eq!(e.selected.len(), 1);
        assert!(
            e.toggle_construction()
                .unwrap_err()
                .contains("complete wire")
        );
        assert!(e.delete().unwrap_err().contains("complete wire"));
        assert_eq!(e.saved.model, before);
    }
    e.select_wire(Some(&id), false);
    assert_eq!(e.selected.len(), 4);
    e.toggle_construction().unwrap();
    assert!(e.saved.model.entities.iter().all(|e| e.construction));
    e.undo();
    assert_eq!(e.saved.model, before);
    e.saved.validate().unwrap();
}
#[test]
fn snapping_is_visible_geometry_only_and_scales_with_view_tolerance() {
    let mut e = editor();
    e.set_tool(Tool::Line);
    click(&mut e, 1., 2.);
    click(&mut e, 10., 2.);
    let snap = e
        .snap(Point::new(1.02, 2.03), 0.1, &BTreeSet::new())
        .unwrap();
    assert_eq!(snap.label, "Endpoint");
    assert_eq!(snap.point, Point::new(1., 2.));
    assert!(
        e.snap(Point::new(1.02, 2.03), 0.001, &BTreeSet::new())
            .is_none()
    );
    e.snapping = false;
    assert!(e.snap(Point::new(1., 2.), 1., &BTreeSet::new()).is_none());
    assert!(e.saved.model.constraints.is_empty());
}
#[test]
fn drawing_does_not_join_unrelated_preexisting_wires() {
    let mut e = editor();
    for (id, a, b) in [("a", 0., 10.), ("b", 10., 20.)] {
        e.saved.model.entities.push(Entity {
            id: id.into(),
            label: None,
            construction: false,
            geometry: SketchGeometry::Line {
                start: Point::new(a, 0.),
                end: Point::new(b, 0.),
            },
        });
        e.saved.model.wires.push(Wire {
            id: format!("wire-{id}"),
            label: None,
            edges: vec![WireEdge {
                entity_id: id.into(),
                reversed: false,
            }],
            closed: false,
        });
    }
    let wires = e.saved.model.wires.clone();
    e.set_tool(Tool::Circle);
    click(&mut e, 30., 30.);
    click(&mut e, 32., 30.);
    assert_eq!(&e.saved.model.wires[..2], &wires);
    assert_eq!(e.saved.model.wires.len(), 3);
    materialized(&e);
}
#[test]
fn invalid_gestures_are_editable_and_bounded_without_polluting_history() {
    let mut e = editor();
    e.set_tool(Tool::Arc);
    click(&mut e, 0., 0.);
    click(&mut e, 1., 1.);
    assert!(e.click(Point::new(2., 2.)).is_err());
    assert!(e.pending());
    assert_eq!(e.history_counts(), (0, 0));
    e.cancel_gesture();
    e.set_tool(Tool::Line);
    click(&mut e, 0., 0.);
    assert!(e.click(Point::new(MAX_COORDINATE_MM * 2., 0.)).is_err());
    assert!(e.saved.model.entities.is_empty());
    assert!(e.pending());
    e.cancel_gesture();
    assert!(!e.pending());
}

#[test]
fn dimensions_edit_convert_remove_and_recover_without_touching_geometry_on_failure() {
    use constraints::{Choice, Form, Operand};
    let mut e = editor();
    let ctx = egui::Context::default();
    e.set_tool(Tool::Line);
    click(&mut e, 0., 0.);
    click(&mut e, 10., 0.);
    let line = e.saved.model.entities[0].id.clone();
    e.saved.constraint_form = Form {
        choice: Choice::Length,
        operands: vec![Operand::Entity(line.clone())],
        value: "20".into(),
        ..Default::default()
    };
    e.store_constraint().unwrap();
    e.poll_solver(&ctx);
    assert!(e.saved.model.require_valid_solution().is_ok());
    assert!(
        (e.saved.model.entities[0]
            .geometry
            .line()
            .unwrap()
            .0
            .distance(e.saved.model.entities[0].geometry.line().unwrap().1)
            - 20.)
            .abs()
            < 1e-6
    );
    assert_eq!(e.history_counts().0, 2);
    let before = e.saved.model.clone();
    e.saved.constraint_form.editing = None;
    e.saved.constraint_form.value = "30".into();
    e.store_constraint().unwrap();
    e.poll_solver(&ctx);
    assert!(!e.solver.outcome.as_ref().unwrap().accepted);
    assert_eq!(e.saved.model.entities, before.entities);
    assert!(!e.solver.outcome.as_ref().unwrap().issues.is_empty());
    let conflict = e.saved.model.constraints[1].id.clone();
    e.remove_constraint(&conflict).unwrap();
    e.poll_solver(&ctx);
    assert!(e.solver.outcome.as_ref().unwrap().accepted);
    let length = e.saved.model.constraints[0].id.clone();
    e.edit_constraint(&length);
    e.saved.constraint_form.value = "20.123456789123".into();
    e.store_constraint().unwrap();
    e.poll_solver(&ctx);
    let exact = e.saved.model.constraints.clone();
    e.edit_constraint(&length);
    e.store_constraint().unwrap();
    assert_eq!(
        e.saved.model.constraints, exact,
        "opening and updating a dimension must not round its authored value"
    );
    e.saved.constraint_form.reference = true;
    e.store_constraint().unwrap();
    e.poll_solver(&ctx);
    assert!(e.saved.model.constraints[0].reference);
    assert_eq!(
        e.solver.outcome.as_ref().unwrap().degrees_of_freedom,
        Some(4)
    );
    e.saved.constraint_form.reference = false;
    e.saved.constraint_form.value = "not a number".into();
    let before = e.saved.model.clone();
    assert!(e.store_constraint().is_err());
    assert_eq!(e.saved.model, before);
    let saved: SavedDraft =
        serde_json::from_str(&serde_json::to_string(&e.saved).unwrap()).unwrap();
    saved.validate().unwrap();
    let restored = Editor::new(saved);
    assert_eq!(restored.saved.constraint_form.value, "not a number");
    assert!(restored.solver.outcome.is_none());
}

#[test]
fn solved_drag_is_one_undo_step_and_escape_restores_the_whole_transaction() {
    let mut e = editor();
    let ctx = egui::Context::default();
    e.set_tool(Tool::Line);
    click(&mut e, 0., 0.);
    click(&mut e, 10., 0.);
    e.poll_solver(&ctx);
    let point = PointRef {
        entity_id: e.saved.model.entities[0].id.clone(),
        point: PointRole::End,
    };
    let before = e.saved.model.clone();
    let count = e.history_counts().0;
    e.start_drag(Point::new(10., 0.), vec![point.clone()]);
    e.drag_to(Point::new(12., 3.));
    e.poll_solver(&ctx);
    e.drag_to(Point::new(14., 5.));
    e.drag.as_mut().unwrap().released = true;
    e.poll_solver(&ctx);
    assert_eq!(e.history_counts().0, count + 1);
    assert!(!e.pending());
    assert!(
        e.saved
            .model
            .point(&point)
            .unwrap()
            .distance(Point::new(14., 5.))
            < 1e-3
    );
    let moved = e.saved.model.clone();
    e.undo();
    e.poll_solver(&ctx);
    assert_eq!(e.saved.model, before);
    e.redo();
    e.poll_solver(&ctx);
    assert_eq!(e.saved.model, moved);
    e.start_drag(Point::new(14., 5.), vec![point]);
    e.drag_to(Point::new(30., 10.));
    e.poll_solver(&ctx);
    e.cancel_gesture();
    e.poll_solver(&ctx);
    assert_eq!(e.saved.model, moved);
    assert_eq!(e.history_counts().0, count + 1);
}
