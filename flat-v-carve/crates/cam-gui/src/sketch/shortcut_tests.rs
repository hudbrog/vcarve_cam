use super::*;

fn editor() -> Editor {
    let mut model = SketchDocument::default();
    for (id, geometry) in [
        (
            "a",
            SketchGeometry::Line {
                start: Point::new(2., 3.),
                end: Point::new(12., 4.),
            },
        ),
        (
            "b",
            SketchGeometry::Line {
                start: Point::new(4., 8.),
                end: Point::new(5., 18.),
            },
        ),
        (
            "c",
            SketchGeometry::Circle {
                center: Point::new(20., 20.),
                radius_mm: 5.,
            },
        ),
        (
            "d",
            SketchGeometry::Circle {
                center: Point::new(40., 20.),
                radius_mm: 3.,
            },
        ),
        (
            "p",
            SketchGeometry::Point {
                position: Point::new(6., 7.),
            },
        ),
    ] {
        model.entities.push(Entity {
            id: id.into(),
            label: None,
            geometry,
            construction: false,
        });
    }
    Editor::new(SavedDraft {
        target: None,
        source_revision: None,
        name: "Shortcuts".into(),
        model,
        had_document: true,
        constraint_form: Default::default(),
    })
}
fn select(e: &mut Editor, ids: &[&str]) {
    e.saved.constraint_form = Default::default();
    for (i, id) in ids.iter().enumerate() {
        e.pick_operand(Some(Operand::Entity((*id).into())), i != 0);
    }
}
fn run(e: &mut Editor, keys: &[Key]) {
    e.run_binding(exact(keys).expect("known command"));
}
fn frame(e: &mut Editor, ctx: &egui::Context, time: f64, key: Option<Key>, modifiers: Modifiers) {
    let events = key.map_or_else(Vec::new, |key| {
        vec![
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers,
            },
        ]
    });
    let _ = ctx.run(
        egui::RawInput {
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| e.shortcuts(ctx),
    );
}
#[test]
fn catalogue_is_unambiguous_and_every_supported_draw_tool_has_its_official_binding() {
    for (i, a) in BINDINGS.iter().enumerate() {
        assert!(!a.keys.is_empty() && !a.label.is_empty());
        for b in &BINDINGS[i + 1..] {
            assert_ne!(a.id, b.id);
            assert!(
                a.keys != b.keys || a.modifiers != b.modifiers,
                "{} duplicates {}",
                a.id,
                b.id
            );
        }
    }
    for (keys, tool) in [
        (&[Key::G, Key::L][..], Tool::Line),
        (&[Key::G, Key::M], Tool::Polyline),
        (&[Key::G, Key::R], Tool::Rectangle),
        (&[Key::G, Key::C], Tool::Circle),
        (&[Key::G, Key::Num3, Key::A], Tool::Arc),
        (&[Key::G, Key::Y], Tool::Point),
    ] {
        let mut e = editor();
        run(&mut e, keys);
        assert_eq!(e.tool, tool);
    }
    for binding in BINDINGS.iter().filter(|b| b.action().is_none()) {
        let mut e = editor();
        let before = e.saved.model.clone();
        e.run_binding(binding);
        assert!(e.message.contains("not available"), "{}", binding.id);
        assert_eq!(before, e.saved.model);
        assert_eq!(e.tool, Tool::Select);
    }
}
#[test]
fn sequences_handle_ambiguity_timeouts_unknown_and_unsupported_without_fallthrough() {
    let mut e = editor();
    let ctx = egui::Context::default();
    for (i, k) in [Key::G, Key::E, Key::A].into_iter().enumerate() {
        frame(&mut e, &ctx, i as f64 * 0.1, Some(k), Modifiers::NONE);
    }
    assert!(e.message.contains("Elliptical Arc"));
    assert!(e.saved.model.constraints.is_empty());
    frame(&mut e, &ctx, 0.3, Some(Key::G), Modifiers::NONE);
    frame(&mut e, &ctx, 0.4, Some(Key::D), Modifiers::NONE);
    assert!(e.message.contains("No Sketcher shortcut"));
    assert!(!e.focus_dimension);
    select(&mut e, &["c", "d"]);
    frame(&mut e, &ctx, 0.5, Some(Key::C), Modifiers::NONE);
    assert!(e.saved.model.constraints.is_empty());
    frame(&mut e, &ctx, 1.3, None, Modifiers::NONE);
    assert!(matches!(
        e.saved.model.constraints[0].definition,
        ConstraintKind::Coincident { .. }
    ));
    let mut e = editor();
    select(&mut e, &["c", "d"]);
    frame(&mut e, &ctx, 1.4, Some(Key::C), Modifiers::NONE);
    frame(&mut e, &ctx, 1.5, Some(Key::C), Modifiers::NONE);
    frame(&mut e, &ctx, 2.5, None, Modifiers::NONE);
    assert_eq!(
        e.saved.model.constraints.len(),
        1,
        "C,C must execute exactly once"
    );
    frame(&mut e, &ctx, 2.6, Some(Key::G), Modifiers::NONE);
    frame(&mut e, &ctx, 4.7, None, Modifiers::NONE);
    assert!(e.shortcut_prefix.is_none());
    frame(&mut e, &ctx, 4.8, Some(Key::D), Modifiers::CTRL);
    assert!(!e.focus_dimension, "Ctrl+D is not D");
}
#[test]
fn contextual_alignment_equality_coincidence_midpoint_and_tangency() {
    let mut e = editor();
    select(&mut e, &["a"]);
    run(&mut e, &[Key::A]);
    assert!(
        matches!(&e.saved.model.constraints[0].definition,ConstraintKind::Horizontal {line} if line=="a")
    );
    select(&mut e, &["b"]);
    run(&mut e, &[Key::A]);
    assert!(
        matches!(&e.saved.model.constraints[1].definition,ConstraintKind::Vertical {line} if line=="b")
    );
    select(&mut e, &["c", "d"]);
    run(&mut e, &[Key::E]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::EqualRadius { .. }
    ));
    select(&mut e, &["a", "b"]);
    run(&mut e, &[Key::E]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::EqualLength { .. }
    ));
    select(&mut e, &["c", "p"]);
    run(&mut e, &[Key::O]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::PointOnCircle { .. }
    ));
    select(&mut e, &["a", "p"]);
    run(&mut e, &[Key::C]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::PointOnLine { .. }
    ));
    select(&mut e, &["a", "p"]);
    run(&mut e, &[Key::S]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::Midpoint { .. }
    ));
    select(&mut e, &["c", "a"]);
    run(&mut e, &[Key::T]);
    assert!(matches!(
        e.saved.model.constraints.last().unwrap().definition,
        ConstraintKind::Tangent { .. }
    ));
    let mut e = editor();
    select(&mut e, &["a", "b"]);
    run(&mut e, &[Key::T]);
    assert_eq!(e.saved.model.constraints.len(), 2);
    assert!(
        e.saved
            .model
            .constraints
            .iter()
            .all(|c| matches!(c.definition, ConstraintKind::PointOnLine { .. }))
    );
    e.undo();
    assert!(
        e.saved.model.constraints.is_empty(),
        "Collinear is one undo step"
    );
}
#[test]
fn lock_block_reference_selection_and_alignment_removal() {
    let mut e = editor();
    select(&mut e, &["p"]);
    run(&mut e, &[Key::K, Key::L]);
    assert_eq!(e.saved.model.constraints.len(), 2);
    assert!(matches!(
        e.saved.model.constraints[0].definition,
        ConstraintKind::HorizontalDistance { value_mm: 6., .. }
    ));
    assert!(matches!(
        e.saved.model.constraints[1].definition,
        ConstraintKind::VerticalDistance { value_mm: 7., .. }
    ));
    e.undo();
    assert!(e.saved.model.constraints.is_empty());
    select(&mut e, &["p"]);
    run(&mut e, &[Key::K, Key::B]);
    assert!(matches!(
        e.saved.model.constraints[0].definition,
        ConstraintKind::FixedPoint { .. }
    ));
    let mut e = editor();
    select(&mut e, &["a", "b"]);
    run(&mut e, &[Key::H]);
    assert_eq!(e.saved.model.constraints.len(), 2);
    run(&mut e, &[Key::Z, Key::R]);
    assert!(e.saved.model.constraints.is_empty());
    e.undo();
    assert_eq!(e.saved.model.constraints.len(), 2);
    select(&mut e, &["p"]);
    run(&mut e, &[Key::Z, Key::O]);
    assert_eq!(e.saved.constraint_form.operands.len(), 2);
    run(&mut e, &[Key::Z, Key::O]);
    assert_eq!(e.saved.constraint_form.operands.len(), 1);
    run(&mut e, &[Key::Z, Key::H]);
    run(&mut e, &[Key::Z, Key::V]);
    assert!(e.selected.contains(X_AXIS_ID) && e.selected.contains(Y_AXIS_ID));
    let ctx = egui::Context::default();
    frame(
        &mut e,
        &ctx,
        0.,
        Some(Key::A),
        Modifiers::CTRL | Modifiers::COMMAND,
    );
    assert_eq!(e.selected.len(), 5);
    assert!(!e.selected.contains(X_AXIS_ID));
}
#[test]
fn dimension_commands_reuse_focus_and_reference_conversion() {
    let mut e = editor();
    select(&mut e, &["c"]);
    run(&mut e, &[Key::R]);
    assert_eq!(e.saved.constraint_form.choice, Choice::Diameter);
    assert_eq!(e.saved.constraint_form.value, "10");
    assert!(e.focus_dimension);
    e.store_constraint().unwrap();
    run(&mut e, &[Key::K, Key::X]);
    assert!(e.saved.model.constraints[0].reference);
    run(&mut e, &[Key::Z, Key::E]);
    assert_eq!(e.selected, ["c".to_string()].into_iter().collect());
    select(&mut e, &["b"]);
    run(&mut e, &[Key::K, Key::A]);
    assert_eq!(e.saved.constraint_form.choice, Choice::Angle);
    assert_eq!(
        e.saved.constraint_form.operands[0],
        Operand::Entity(X_AXIS_ID.into())
    );
    let mut e = editor();
    run(&mut e, &[Key::K, Key::X]);
    select(&mut e, &["a"]);
    run(&mut e, &[Key::D]);
    assert!(e.saved.constraint_form.reference);
    assert!(!e.focus_dimension);
    e.finish_dimension_edit();
    assert!(e.saved.model.constraints[0].reference);
    select(&mut e, &["p"]);
    e.pick_operand(Some(Operand::Point(origin_ref())), true);
    run(&mut e, &[Key::H]);
    assert!(
        !e.saved.model.constraints.last().unwrap().reference,
        "Alignment stays driving in reference dimension mode"
    );
}
