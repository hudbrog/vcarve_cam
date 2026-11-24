use cam_core::{
    checks::check_plan_v5,
    post::{
        Coolant,
        sequence::{OutputLayout, PreparedExecution, SequenceProfile},
    },
    project::v5::{
        self, CamJobV5, GeometryPick, GeometryRefKind, OperationSettingsV5, ReadinessScope,
        commands::{self, NewOperationKind},
    },
    sequence::{GenerationStatus, OperationPlanV5, PlanLimits, TrustedPlanV5},
    toolpath::{Interpolation, MotionEffect, MotionPurpose},
};

const DRAWING: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30">
<path id="a" fill="none" stroke="black" d="M2 5H10"/>
<path id="b" fill="none" stroke="black" d="M10 5V10"/>
<path id="box" fill="none" stroke="black" d="M10.002 10h5v5h-5Z"/>
<circle id="circle" cx="20" cy="15" r="5" fill="none" stroke="black"/>
<path id="cross" fill="none" stroke="black" d="M8 8h4"/>
</svg>"#;

fn selected(names: &[&str]) -> CamJobV5 {
    let job = v5::authoring::from_svg("lines.svg".into(), DRAWING.into(), 0.0001).unwrap();
    let job = commands::add_operation(&job, NewOperationKind::CenterlinePlot, "plot", "Drawing")
        .unwrap()
        .job;
    let item = &v5::inspect_artwork(&job).unwrap().items[0];
    let picks: Vec<_> = names
        .iter()
        .map(|name| GeometryPick {
            artwork_item_id: item.id.clone(),
            kind: GeometryRefKind::Centerline,
            local_geometry_id: format!("{name}-chain-0"),
        })
        .collect();
    let mut job = commands::set_chain_selection(&job, "plot", &picks)
        .unwrap()
        .job;
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.draw_feed_mm_min = Some(320.);
    s.lower_feed_mm_min = Some(80.);
    s.compression_mm = 0.2;
    job
}

fn plan(job: &CamJobV5) -> OperationPlanV5 {
    OperationPlanV5::plan_job_v5(job, &ReadinessScope::AllEnabled, &PlanLimits::default()).unwrap()
}

fn set_gap(job: &mut CamJobV5, gap: f64) {
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.closure_gap_mm = gap;
}

fn drawn_length(p: &OperationPlanV5) -> f64 {
    p.motions
        .iter()
        .filter(|m| m.effect == MotionEffect::PenTrace)
        .map(|m| m.start.xy().distance(m.end.xy()))
        .sum()
}

#[test]
fn plot_selection_rejects_zero_length_paths_and_cleanup_only_removes_current_matches() {
    let mut job = selected(&[]);
    let v5::ArtworkContent::Svg(svg) = &mut job.artwork[0].content else {
        unreachable!()
    };
    svg.svg = svg.svg.replace(
        "</svg>",
        r#"<polyline id="dot" points="1,1 1,1" fill="none" stroke="black"/></svg>"#,
    );
    let catalogue = v5::inspect_artwork(&job).unwrap();
    let dot = catalogue.items[0]
        .entries
        .iter()
        .find(|e| e.is_undrawable_plot_path())
        .unwrap()
        .reference
        .clone();
    let pick = GeometryPick {
        artwork_item_id: dot.artwork_item_id.clone(),
        kind: dot.kind,
        local_geometry_id: dot.local_geometry_id.clone(),
    };
    assert!(
        commands::set_chain_selection(&job, "plot", &[pick])
            .unwrap_err()
            .message
            .contains("no drawable path")
    );
    let mut stale = dot.clone();
    stale.source_revision.content_digest = "0".repeat(64);
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.chains = vec![stale.clone(), dot.clone()];
    let repaired = commands::remove_undrawable_plot_paths(&job, "plot")
        .unwrap()
        .job;
    let OperationSettingsV5::CenterlinePlot(s) = &repaired.operations[0].settings else {
        unreachable!()
    };
    assert_eq!(s.chains, vec![stale]);
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.chains = vec![dot];
    let repaired = commands::remove_undrawable_plot_paths(&job, "plot")
        .unwrap()
        .job;
    let OperationSettingsV5::CenterlinePlot(s) = &repaired.operations[0].settings else {
        unreachable!()
    };
    assert!(s.chains.is_empty());
}

#[test]
fn circle_gap_is_measured_along_the_placed_stroke_and_ends_with_a_vertical_lift() {
    let mut job = selected(&["circle"]);
    job.artwork[0].placement.scale = 0.75;
    let old_json = job.to_json().unwrap();
    assert!(!old_json.contains("closure_gap_mm"));
    let full = plan(&CamJobV5::from_json(&old_json).unwrap());
    set_gap(&mut job, 0.2);
    let trimmed = plan(&CamJobV5::from_json(&job.to_json().unwrap()).unwrap());
    assert!(check_plan_v5(&trimmed).unwrap().export_ready);
    assert!((drawn_length(&full) - drawn_length(&trimmed) - 0.2).abs() < 1e-9);
    let lift = trimmed.motions.last().unwrap();
    assert_eq!(lift.purpose, MotionPurpose::Clearance);
    assert_eq!(lift.start.xy(), lift.end.xy());
    assert_eq!(lift.start, trimmed.motions[trimmed.motions.len() - 2].end);
    assert_ne!(lift.end.xy(), full.motions.last().unwrap().end.xy());
}

#[test]
fn gap_can_consume_whole_edges_but_must_leave_a_drawable_loop() {
    let mut job = selected(&["box"]);
    let length = drawn_length(&plan(&job));
    for (gap, count) in [(0.2, 4), (5., 3), (6.2, 3), (19., 1)] {
        set_gap(&mut job, gap);
        let p = plan(&job);
        assert!(check_plan_v5(&p).unwrap().export_ready);
        assert!((drawn_length(&p) - (length - gap)).abs() < 1e-9);
        assert_eq!(
            p.motions
                .iter()
                .filter(|m| m.effect == MotionEffect::PenTrace)
                .count(),
            count
        );
    }
    for gap in [length, length + 1.] {
        set_gap(&mut job, gap);
        let p = plan(&job);
        assert!(
            p.generation_diagnostics
                .iter()
                .any(|i| i.code == "PLOT_CLOSURE_GAP")
        );
        assert!(!check_plan_v5(&p).unwrap().export_ready);
    }
    set_gap(&mut job, -0.2);
    assert!(job.validate_structure().is_err());
}

#[test]
fn touches_join_but_small_gaps_and_crossings_lift() {
    let job = selected(&["a", "b", "box", "cross"]);
    let p = plan(&CamJobV5::from_json(&job.to_json().unwrap()).unwrap());
    assert_eq!(
        p.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        p.operation_results
    );
    let m = &p.motions;
    let draws: Vec<_> = m
        .iter()
        .filter(|m| m.effect == MotionEffect::PenTrace)
        .collect();
    assert_eq!(draws.len(), 7);
    assert_eq!(draws[0].id + 1, draws[1].id);
    assert_eq!(draws[0].end, draws[1].start);
    assert_eq!(draws[2].start, draws[5].end);
    assert!(
        draws
            .iter()
            .all(|m| m.start.z == -0.2 && m.end.z == -0.2 && m.feed_mm_min == Some(320.))
    );
    assert_eq!(
        m.iter()
            .filter(|m| m.purpose == MotionPurpose::Entry)
            .count(),
        3
    );
    assert!(
        m.iter()
            .filter(|m| m.interpolation == Interpolation::Rapid && m.start.xy() != m.end.xy())
            .all(|m| m.start.z == 5. && m.end.z == 5.)
    );
    assert!(m.windows(2).all(|w| w[0].end == w[1].start));
    assert_eq!(m.last().unwrap().end.z, 5.);
    assert!(p.stock_history(None).unwrap().batches.is_empty());
    assert!(check_plan_v5(&p).unwrap().export_ready);
}

#[test]
fn source_direction_and_selection_order_determine_contact() {
    let p = plan(&selected(&["b", "a"]));
    assert_eq!(
        p.motions
            .iter()
            .filter(|m| m.purpose == MotionPurpose::Entry)
            .count(),
        2
    );
    assert!(
        p.motions
            .iter()
            .find(|m| m.effect == MotionEffect::PenTrace)
            .unwrap()
            .contour_id
            .as_ref()
            .unwrap()
            .ends_with("b-chain-0")
    );
}

#[test]
fn linuxcnc_exports_one_continuous_run_for_touching_paths() {
    let job = selected(&["a", "b"]);
    let mut profile =
        SequenceProfile::from_json(include_str!("../../../fixtures/gui2/machine.json")).unwrap();
    profile.tools.truncate(1);
    profile.tools[0].tool_id = "pen".into();
    profile.coolant = Coolant::Flood;
    let job = v5::machine::apply_machine_configuration(&job, &profile, "Plotter")
        .unwrap()
        .job;
    let profile = v5::machine::resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap();
    let trusted = TrustedPlanV5::from_generated(plan(&job));
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    for layout in [OutputLayout::OneProgram, OutputLayout::SequentialFiles] {
        let bundle = prepared.export_bundle(&trusted, &profile, layout).unwrap();
        let g = &bundle.files[0].gcode;
        assert!(g.contains("G61"));
        assert!(
            !g.split_whitespace()
                .any(|word| matches!(word, "M3" | "M4" | "M7" | "M8"))
        );
        let decoded = prepared
            .decode_program(trusted.plan(), bundle.report.output_decimal_places, g)
            .unwrap();
        assert_eq!(decoded.motions.len(), trusted.plan().motions.len());
    }
}

#[test]
fn grbl_plots_with_the_native_planner_and_preserves_strokes_at_controller_precision() {
    use cam_core::post::{grbl, machine::GrblProfile};

    let mut job = selected(&["box", "a", "b"]);
    set_gap(&mut job, 0.002);
    let mut profile: GrblProfile =
        serde_json::from_str(include_str!("../../../fixtures/gui2/grbl-machine.json")).unwrap();
    profile.decimal_places = 1;
    profile.coolant = Coolant::Flood;
    let trusted = TrustedPlanV5::from_generated(plan(&job));
    for layout in [OutputLayout::OneProgram, OutputLayout::SequentialFiles] {
        let bundle = grbl::export_bundle(&trusted, &profile, layout).unwrap();
        let g = &bundle.files[0].gcode;
        assert_eq!(bundle.report.output_decimal_places, 3);
        assert!(g.starts_with("M5\nM9\n"));
        assert!(
            !g.split_whitespace()
                .any(|w| matches!(w, "M3" | "M4" | "M7" | "M8" | "G61" | "G64" | "G4"))
        );
        // Finish the square short of its start, lift vertically, then cross to the open strokes.
        assert!(
            g.contains("G1 X10.002 Y19.998 Z-0.2 F320\nG0 X10.002 Y19.998 Z5\nG0 X2 Y25 Z5"),
            "{g}"
        );
        // The next two paths share an endpoint and are drawn without lifting.
        assert!(
            g.contains("G1 X10 Y25 Z-0.2 F320\nG1 X10 Y20 Z-0.2 F320"),
            "{g}"
        );
    }
}

#[test]
fn exported_coordinates_keep_a_small_closure_gap_open() {
    let mut job = selected(&["box"]);
    set_gap(&mut job, 0.02);
    let mut profile =
        SequenceProfile::from_json(include_str!("../../../fixtures/gui2/machine.json")).unwrap();
    profile.tools.truncate(1);
    profile.tools[0].tool_id = "pen".into();
    profile.decimal_places = 1;
    let job = v5::machine::apply_machine_configuration(&job, &profile, "Plotter")
        .unwrap()
        .job;
    let profile = v5::machine::resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap();
    let trusted = TrustedPlanV5::from_generated(plan(&job));
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let output = prepared.export(&trusted, &profile).unwrap();
    let places = output.report.output_decimal_places;
    assert_eq!(places, 2);
    let decoded = prepared
        .decode_program(trusted.plan(), places, &output.program.gcode)
        .unwrap();
    let start = decoded.motions.first().unwrap().end.xy();
    let lift = decoded.motions.last().unwrap();
    assert_eq!(lift.start.unwrap().xy(), lift.end.xy());
    assert!((lift.end.xy().distance(start) - 0.02).abs() < 1e-9);
}

#[test]
fn unfinished_operation_saves_and_reports_missing_inputs() {
    let mut job = selected(&[]);
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.draw_feed_mm_min = None;
    assert!(job.to_json().is_ok());
    let fields = v5::inspection::inspect_operation_fields(&job, "plot").unwrap();
    for name in ["chains", "draw_feed_mm_min"] {
        assert!(fields.iter().any(|f| f.field_path.as_deref() == Some(name)));
    }
    assert_ne!(
        plan(&job).operation_results[0].generation_status,
        GenerationStatus::Complete
    );
}

#[test]
fn a_tampered_gap_or_drawing_direction_fails_the_plot_check() {
    let mut p = plan(&selected(&["a", "cross"]));
    let draw = p
        .motions
        .iter_mut()
        .find(|m| m.effect == MotionEffect::PenTrace)
        .unwrap();
    draw.contour_id = None;
    let report = check_plan_v5(&p).unwrap();
    assert!(!report.export_ready);
    assert!(report.findings.iter().any(|f| f.code == "PLOT_TRACE"));
}

#[test]
fn mixed_job_restores_milling_process_and_exports_a_plot_prefix() {
    use cam_core::project::{SpindleDirection, WorkZeroZ};
    let source = CamJobV5::from_json(include_str!("../../../fixtures/v5/full-job.json")).unwrap();
    let mut job = selected(&["a", "b"]);
    job.setup.work_zero.z = WorkZeroZ::StockBottom;
    let mut face = source.operations[0].clone();
    let OperationSettingsV5::Face(s) = &mut face.settings else {
        unreachable!()
    };
    s.assignment.spindle_direction = Some(SpindleDirection::Clockwise);
    job.tools.push(source.tools[0].clone());
    job.operations.insert(0, face.clone());
    face.id = "face-again".into();
    job.operations.push(face);
    let mut machine =
        SequenceProfile::from_json(include_str!("../../../fixtures/gui2/machine.json")).unwrap();
    machine.tools[0].tool_id = "t1".into();
    machine.tools[1].tool_id = "pen".into();
    machine.coolant = Coolant::Flood;
    let job = v5::machine::apply_machine_configuration(&job, &machine, "Mixed")
        .unwrap()
        .job;
    let p = plan(&job);
    assert!(
        check_plan_v5(&p).unwrap().export_ready,
        "{:?}",
        p.operation_results
    );
    assert_eq!(
        p.stock_history(Some("plot")).unwrap().batches.len(),
        p.stock_history(Some("face-1")).unwrap().batches.len()
    );
    let profile = v5::machine::resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap();
    let trusted = TrustedPlanV5::from_generated(p);
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let bundle = prepared
        .export_bundle(&trusted, &profile, OutputLayout::SequentialFiles)
        .unwrap();
    assert_eq!(bundle.files.len(), 3);
    for (i, file) in bundle.files.iter().enumerate() {
        let words: Vec<_> = file.gcode.split_whitespace().collect();
        assert_eq!(words.contains(&"M3"), i != 1);
        assert_eq!(words.contains(&"M8"), i != 1);
    }
    prepared.export(&trusted, &profile).unwrap();
    let scope = ReadinessScope::ThroughOperation {
        operation_id: "plot".into(),
    };
    let prefix = OperationPlanV5::plan_job_v5(&job, &scope, &PlanLimits::default()).unwrap();
    assert_eq!(prefix.stages.last().unwrap().operation_id, "plot");
    assert_eq!(prefix.motions.last().unwrap().end.z, 5.);
}

#[test]
fn placed_sketch_centerlines_use_the_same_plot_path() {
    let job = CamJobV5::from_json(include_str!("../../../fixtures/v5/sketch-job.json")).unwrap();
    let mut job = commands::add_operation(
        &job,
        NewOperationKind::CenterlinePlot,
        "sketch-plot",
        "Sketch plot",
    )
    .unwrap()
    .job;
    let item = &v5::inspect_artwork(&job).unwrap().items[0];
    let pick = item
        .entries
        .iter()
        .find(|e| e.kind == GeometryRefKind::Centerline)
        .unwrap();
    let pick = GeometryPick {
        artwork_item_id: item.id.clone(),
        kind: pick.kind,
        local_geometry_id: pick.reference.local_geometry_id.clone(),
    };
    job = commands::set_chain_selection(&job, "sketch-plot", &[pick])
        .unwrap()
        .job;
    let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.draw_feed_mm_min = Some(350.);
    s.lower_feed_mm_min = Some(60.);
    let original = plan(&job);
    assert!(
        check_plan_v5(&original).unwrap().export_ready,
        "{:?}",
        original.operation_results
    );
    // Placement subtracts the page origin, so moving it left shifts the drawing right.
    job.artwork[0].placement.origin_mm.x -= 1.;
    let moved = plan(&job);
    assert!(check_plan_v5(&moved).unwrap().export_ready);
    let start = |p: &OperationPlanV5| {
        p.motions
            .iter()
            .find(|m| m.effect == MotionEffect::PenTrace)
            .unwrap()
            .start
    };
    assert!((start(&moved).x - start(&original).x - 1.).abs() < 1e-6);
}

#[test]
fn replacing_artwork_requires_selection_repair_before_export() {
    let mut job = selected(&["a", "b"]);
    let v5::ArtworkContent::Svg(svg) = &mut job.artwork[0].content else {
        unreachable!()
    };
    svg.svg = svg.svg.replace("M2 5H10", "M3 5H10");
    assert!(!check_plan_v5(&plan(&job)).unwrap().export_ready);
}

#[test]
fn closed_chain_can_continue_at_its_seam_after_rotation() {
    let mut job = selected(&["box"]);
    let v5::ArtworkContent::Svg(svg) = &mut job.artwork[0].content else {
        unreachable!()
    };
    svg.svg = svg.svg.replace(
        "</svg>",
        r#"<path id="tail" fill="none" stroke="black" d="M10.002 10h-4"/></svg>"#,
    );
    job.artwork[0].placement.rotation_deg = 90.;
    job.artwork[0].placement.origin_mm.y = 30.;
    let owner = job.artwork[0].id.clone();
    let paths: Vec<_> = ["box", "tail"]
        .into_iter()
        .map(|id| GeometryPick {
            artwork_item_id: owner.clone(),
            kind: GeometryRefKind::Centerline,
            local_geometry_id: format!("{id}-chain-0"),
        })
        .collect();
    job = commands::set_chain_selection(&job, "plot", &paths)
        .unwrap()
        .job;
    let p = plan(&job);
    assert!(
        check_plan_v5(&p).unwrap().export_ready,
        "{:?}",
        p.operation_results
    );
    assert_eq!(
        p.motions
            .iter()
            .filter(|m| m.purpose == MotionPurpose::Entry)
            .count(),
        1
    );
    let m: Vec<_> = p
        .motions
        .iter()
        .filter(|m| m.effect == MotionEffect::PenTrace)
        .collect();
    assert_eq!(m.len(), 5);
    assert_eq!(m[0].start, m[3].end);
    assert_eq!(m[3].id + 1, m[4].id);

    set_gap(&mut job, 0.2);
    let trimmed = plan(&job);
    assert!(check_plan_v5(&trimmed).unwrap().export_ready);
    let tail = trimmed
        .motions
        .iter()
        .find(|m| m.pass_id == 1 && m.effect == MotionEffect::PenTrace)
        .unwrap();
    assert_eq!((tail.start, tail.end), (m[4].start, m[4].end));
    let transfer = &trimmed.motions[tail.id - 3..tail.id];
    assert_eq!(transfer[0].start.xy(), transfer[0].end.xy());
    assert!((transfer[1].start.xy().distance(transfer[1].end.xy()) - 0.2).abs() < 1e-9);
    assert_eq!((transfer[1].start.z, transfer[1].end.z), (5., 5.));
    assert_eq!(transfer[2].purpose, MotionPurpose::Entry);
}

#[test]
fn invalid_feeds_and_compression_are_rejected_at_the_document_boundary() {
    for (draw, lower, compression) in [(0., 50., 0.), (100., -1., 0.), (100., 50., -0.2)] {
        let mut job = selected(&["a"]);
        let OperationSettingsV5::CenterlinePlot(s) = &mut job.operations[0].settings else {
            unreachable!()
        };
        s.draw_feed_mm_min = Some(draw);
        s.lower_feed_mm_min = Some(lower);
        s.compression_mm = compression;
        assert!(job.validate_structure().is_err());
    }
}
