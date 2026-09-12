use cam_core::project::{FlatVcarveMode, SpindleDirection, WorkZeroXY, WorkZeroZ, v5};
use cam_gui_runtime::{
    app::Document,
    authoring::{self, settings_mut},
    compute,
    session::{self, Command},
};
use cam_service::retained::Retained;
const SVG: &str = include_str!("../../../fixtures/gui2/new-carving.svg");

#[test]
fn gui3_policy_fields_preserve_siblings_and_portable_inactive_finish() {
    let mut doc = configured();
    authoring::set_mode(&mut doc.job, FlatVcarveMode::Combined);
    let endmill = session::settings(&doc.job).endmill.clone();
    let vbit = session::settings(&doc.job).vbit.clone();
    for (f, n) in [
        (47, -0.2),
        (48, 31.),
        (49, 120.),
        (50, 90000.),
        (52, 60000.),
        (53, 90000.),
        (54, 800000.),
        (55, 128.),
        (56, 0.),
        (57, 0.5),
        (58, 900000.),
        (59, 80000.),
        (60, 16.),
    ] {
        doc.edit(f, n.to_string()).unwrap();
        assert_eq!(cam_gui_runtime::app::value(&doc.job, f), Some(n));
        let previous = doc.job.clone();
        for invalid in ["", "-", "NaN"] {
            assert!(doc.edit(f, invalid.into()).is_err());
            assert_eq!(doc.job, previous);
            assert!(doc.pending());
        }
        doc.edit(f, n.to_string()).unwrap();
        if f != 47 && f != 57 {
            assert!(doc.edit(f, "1.5".into()).is_err());
            assert_eq!(doc.job, previous);
            doc.edit(f, n.to_string()).unwrap();
        }
    }
    assert_eq!(session::settings(&doc.job).endmill, endmill);
    assert_eq!(session::settings(&doc.job).vbit, vbit);
    let finish = session::settings(&doc.job).finish.clone();
    authoring::set_mode(&mut doc.job, FlatVcarveMode::EndmillOnly);
    let reopened = session::open(&doc.job.to_json().unwrap()).unwrap();
    assert_eq!(session::settings(&reopened).finish, finish);
    doc = Document::new(reopened);
    let scope = v5::references::ReadinessScope::AllEnabled;
    let identity =
        cam_core::sequence::OperationPlanV5::machining_identity(&doc.job, &scope).unwrap();
    settings_mut(&mut doc.job)
        .finish
        .as_mut()
        .unwrap()
        .max_paths = 100;
    assert_eq!(
        cam_core::sequence::OperationPlanV5::machining_identity(&doc.job, &scope).unwrap(),
        identity
    );
    settings_mut(&mut doc.job).finish = finish.clone();
    let plan =
        cam_core::sequence::OperationPlanV5::plan_job_v5(&doc.job, &scope, &Default::default())
            .unwrap();
    assert!(!plan.motions.is_empty());
    assert!(
        plan.stages
            .iter()
            .all(|s| s.role == cam_core::sequence::StageRole::VcarveRough)
    );
    doc.edit(57, "-".into()).unwrap_err();
    doc.edit(4, "-".into()).unwrap_err();
    assert!(
        !doc.pending(),
        "inactive finish draft must not block roughing"
    );
    authoring::set_mode(&mut doc.job, FlatVcarveMode::Combined);
    assert!(doc.pending());
    doc.edit(57, "0.5".into()).unwrap();
    doc.edit(4, "0.1".into()).unwrap();
    assert!(!doc.pending());
    assert_eq!(session::settings(&doc.job).finish, finish);
}

#[test]
fn gui3_ramp_requires_explicit_values_and_tool_changes_clear_only_one_assignment() {
    use cam_core::pocket::EntryStrategy;
    let mut doc = configured();
    doc.edit(14, "7".into()).unwrap();
    doc.edit(51, "250".into()).unwrap();
    assert!(matches!(
        session::settings(&doc.job).rough.as_ref().unwrap().entry,
        EntryStrategy::Plunge
    ));
    authoring::set_group(&mut doc.job, 14, &[7., 250.]).unwrap();
    let previous = doc.job.clone();
    assert!(doc.edit(14, "90".into()).is_err());
    assert_eq!(doc.job, previous);
    doc.edit(14, "5".into()).unwrap();
    assert_eq!(
        session::settings(&doc.job).rough.as_ref().unwrap().entry,
        EntryStrategy::Ramp {
            max_angle_deg: 5.,
            feed_mm_min: 250.
        }
    );
    let finish = session::settings(&doc.job).vbit.clone();
    let tools = doc.job.tools.clone();
    let mut alternate = tools[0].clone();
    alternate.id = "alternate-endmill".into();
    doc.job.tools.push(alternate);
    authoring::assign_tool(&mut doc.job, false, "alternate-endmill").unwrap();
    let s = session::settings(&doc.job);
    assert_eq!(s.endmill.tool_id, "alternate-endmill");
    assert_eq!(s.endmill.cutting_feed_mm_min, None);
    assert_eq!(s.endmill.spindle_rpm, None);
    assert_eq!(s.vbit, finish);
    assert_eq!(&doc.job.tools[..2], &tools);
    let previous = doc.job.clone();
    assert!(authoring::assign_tool(&mut doc.job, false, &finish.tool_id).is_err());
    assert_eq!(doc.job, previous);
}

#[test]
fn gui3_lettering_subset_supports_complete_combined_export() {
    let base = configured();
    let mut doc = Document::new(
        authoring::import_svg(
            "lettering.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap(),
    );
    doc.job.setup = base.job.setup;
    doc.job.tools = base.job.tools;
    doc.job.tolerances = base.job.tolerances;
    doc.job.operations = base.job.operations;
    doc.edit(12, "2.5".into()).unwrap();
    doc.job.artwork[0].placement.origin_mm = cam_core::geometry::Point::new(-5., -3.);
    let catalogue = v5::artwork::inspect_artwork(&doc.job).unwrap();
    settings_mut(&mut doc.job).components = vec![
        catalogue.items[0]
            .entries
            .iter()
            .find(|e| e.reference.local_geometry_id == "letter-l::0")
            .unwrap()
            .reference
            .clone(),
    ];
    authoring::set_mode(&mut doc.job, FlatVcarveMode::Combined);
    for (f, value) in [
        (3, "1000"),
        (21, "300"),
        (20, "1"),
        (46, "1"),
        (22, "12000"),
    ] {
        doc.edit(f, value.into()).unwrap();
    }
    settings_mut(&mut doc.job).vbit.spindle_direction = Some(SpindleDirection::Clockwise);
    authoring::tool_mut(&mut doc.job, true)
        .unwrap()
        .capabilities
        .plunge_capable = Some(true);
    for detail in [0.1, 0.05] {
        doc.edit(5, detail.to_string()).unwrap();
        let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
            &doc.job,
            &v5::references::ReadinessScope::AllEnabled,
            &Default::default(),
        )
        .unwrap();
        assert!(
            plan.operation_results
                .iter()
                .all(|r| r.generation_status == cam_core::sequence::GenerationStatus::Complete),
            "detail {detail}: {:?}",
            plan.generation_diagnostics
        );
    }
}

#[test]
fn gui3_readiness_keeps_output_only_reference_repairs_out_of_generation() {
    let mut doc = configured();
    let profile = cam_core::post::sequence::SequenceProfile::from_json(session::PROFILE).unwrap();
    doc.job = v5::machine::apply_machine_configuration(&doc.job, &profile, &profile.id)
        .unwrap()
        .job;
    doc.job
        .machine_configuration
        .as_mut()
        .unwrap()
        .tools
        .push(v5::AppliedToolMapping {
            job_tool_id: "deleted-output-only-tool".into(),
            tool_number: Some(99),
            length_offset_number: None,
        });
    assert!(
        !v5::references::inspect_references(&doc.job)
            .unwrap()
            .issues
            .is_empty()
    );
    let (scene, _) = session::execute(
        &mut Retained::new(),
        Command::Generate {
            job: doc.job.to_json().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(scene.report["gui2"]["kind"], "generated");
    assert!(scene.motions > 0);
}

fn edit(doc: &mut Document, field: usize, text: &str) {
    let _ = doc.edit(field, text.into());
}
fn configured() -> Document {
    let mut doc =
        Document::new(authoring::import_svg("new-carving.svg".into(), SVG.into()).unwrap());
    let catalogue = v5::artwork::inspect_artwork(&doc.job).unwrap();
    settings_mut(&mut doc.job).components = vec![
        catalogue.items[0]
            .entries
            .iter()
            .find(|e| {
                e.kind == v5::GeometryRefKind::FilledComponent
                    && e.reference.local_geometry_id.starts_with("panel")
            })
            .unwrap()
            .reference
            .clone(),
    ];
    for (field, text) in [
        (0, "1"),
        (1, "0"),
        (4, "0.1"),
        (5, "0.1"),
        (6, "10"),
        (7, "5"),
        (8, "0.5"),
        (9, "1"),
        (2, "1200"),
        (10, "400"),
        (11, "12000"),
        (12, "3"),
        (13, "15"),
        (16, "90"),
        (17, "0.1"),
        (18, "12"),
        (19, "5"),
        (23, "0.01"),
        (25, "0.05"),
        (30, "0"),
        (31, "0"),
        (40, "-20"),
        (41, "-20"),
        (42, "80"),
        (43, "60"),
    ] {
        edit(&mut doc, field, text);
    }
    settings_mut(&mut doc.job).rough = Some(Default::default());
    settings_mut(&mut doc.job).endmill.spindle_direction = Some(SpindleDirection::Clockwise);
    authoring::tool_mut(&mut doc.job, false)
        .unwrap()
        .capabilities
        .plunge_capable = Some(true);
    assert!(
        !doc.pending(),
        "{:?}",
        authoring::FIELDS
            .iter()
            .filter(|&&f| authoring::active(&doc.job, f))
            .filter_map(|&f| {
                let parsed = cam_gui_runtime::state::Draft::parse(&doc.text(f));
                (parsed.ok() != Some(cam_gui_runtime::app::value(&doc.job, f)))
                    .then(|| (f, doc.text(f), cam_gui_runtime::app::value(&doc.job, f)))
            })
            .collect::<Vec<_>>()
    );
    doc
}
#[test]
fn import_is_unset_saveable_and_selects_nothing() {
    let job = authoring::import_svg("new.svg".into(), SVG.into()).unwrap();
    assert!(session::settings(&job).components.is_empty());
    assert!(job.setup.stock.thickness_mm.is_none());
    assert!(job.setup.stock.xy.is_none());
    assert!(
        job.tools
            .iter()
            .all(|t| t.geometry.is_none() && t.capabilities.plunge_capable.is_none())
    );
    assert!(
        session::settings(&job)
            .endmill
            .cutting_feed_mm_min
            .is_none()
    );
    assert!(job.machine_configuration.is_none());
    assert_eq!(session::open(&job.to_json().unwrap()).unwrap(), job);
    let (preview, _) = session::execute(
        &mut Retained::new(),
        Command::ImportSvg {
            filename: "new.svg".into(),
            svg: SVG.into(),
        },
    )
    .unwrap();
    assert!(
        preview.contour_vertices > 0,
        "unselected artwork is visible"
    );
    assert_eq!(
        preview.report["gui2"]["components"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(authoring::import_svg("bad.svg".into(), "<svg>broken".into()).is_err());
}
#[test]
fn units_placement_selection_and_partial_geometry_round_trip() {
    let mut doc = configured();
    let before = v5::artwork::inspect_artwork(&doc.job).unwrap();
    let b = before.items[0]
        .entries
        .iter()
        .find(|e| {
            e.kind == v5::GeometryRefKind::FilledComponent
                && e.reference.local_geometry_id.starts_with("panel")
        })
        .unwrap();
    assert!((b.bounds.max_x_mm - b.bounds.min_x_mm - 16.).abs() < 1e-6);
    let reference = settings_mut(&mut doc.job).components[0].clone();
    for (f, v) in [(26, "2"), (27, "3"), (28, "90"), (29, "0.5")] {
        doc.edit(f, v.into()).unwrap();
    }
    assert_eq!(session::open(&doc.job.to_json().unwrap()).unwrap(), doc.job);
    assert_eq!(
        settings_mut(&mut doc.job).components[0],
        reference,
        "placement does not invalidate component identity"
    );
    let after = v5::artwork::inspect_artwork(&doc.job).unwrap();
    let a = after.items[0]
        .entries
        .iter()
        .find(|e| e.reference == reference)
        .unwrap();
    assert!((a.bounds.max_x_mm - a.bounds.min_x_mm - 7.).abs() < 1e-6);
    let old = authoring::tool(&doc.job, false).unwrap().geometry.clone();
    assert!(doc.edit(12, "-".into()).is_err());
    assert!(doc.pending());
    assert_eq!(authoring::tool(&doc.job, false).unwrap().geometry, old);
    let snapshot = doc.snapshot();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.draft.raw[&doc.raw.key(12)], "-");
}
#[test]
fn both_modes_generate_and_export_with_one_datum_and_explicit_mappings() {
    let mut doc = configured();
    doc.job.setup.work_zero.xy = WorkZeroXY::CustomPoint { x_mm: 2., y_mm: 3. };
    doc.job.setup.work_zero.z = WorkZeroZ::StockBottom;
    let mut profile: serde_json::Value = serde_json::from_str(session::PROFILE).unwrap();
    profile["tools"][0]["tool_id"] = serde_json::json!("unrelated-endmill");
    profile["tools"][1]["tool_id"] = serde_json::json!("unrelated-vbit");
    let (applied, _) = session::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: doc.job.to_json().unwrap(),
            json: profile.to_string(),
        },
    )
    .unwrap();
    doc = Document::new(session::open(&applied.job).unwrap());
    assert_eq!(doc.job.setup.work_zero.z, WorkZeroZ::StockBottom);
    assert!(
        doc.job
            .machine_configuration
            .as_ref()
            .unwrap()
            .tools
            .is_empty()
    );
    doc.edit(32, "3".into()).unwrap();
    doc.edit(7, "6".into()).unwrap();
    assert_eq!(doc.job.setup.clearance_above_stock_mm, Some(6.));
    assert_eq!(
        doc.job
            .machine_configuration
            .as_ref()
            .unwrap()
            .clearance_z_mm,
        Some(6.)
    );
    assert!(doc.edit(32, "3.5".into()).is_err());
    doc.edit(32, "3".into()).unwrap();
    // Unused partial finishing text cannot block an endmill-only execution.
    assert!(doc.edit(3, "-".into()).is_err());
    assert!(!doc.pending());
    let mut retained = Retained::new();
    let (rough, _) = session::execute(
        &mut retained,
        Command::Generate {
            job: doc.job.to_json().unwrap(),
        },
    )
    .unwrap();
    assert!(rough.motions > 0);
    assert_eq!(rough.rough_vertices / 2, rough.motions);
    let handle = rough.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let (prepared, _) = session::execute(
        &mut retained,
        Command::Prepare {
            job: doc.job.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert_eq!(prepared.report["gui2"]["retained"]["plansRun"], 1);
    assert_eq!(
        compute::hash(
            prepared.report["gui2"]["file"]["gcode"]
                .as_str()
                .unwrap()
                .as_bytes()
        ),
        prepared.report["gui2"]["file"]["sha256"]
    );
    authoring::set_mode(&mut doc.job, FlatVcarveMode::Combined);
    assert!(doc.pending());
    for (f, v) in [
        (3, "1000"),
        (21, "300"),
        (20, "1"),
        (22, "12000"),
        (46, "1"),
        (38, "8"),
    ] {
        doc.edit(f, v.into()).unwrap();
    }
    settings_mut(&mut doc.job).vbit.spindle_direction = Some(SpindleDirection::Clockwise);
    authoring::tool_mut(&mut doc.job, true)
        .unwrap()
        .capabilities
        .plunge_capable = Some(true);
    assert!(!doc.pending());
    let (combined, _) = session::execute(
        &mut retained,
        Command::Generate {
            job: doc.job.to_json().unwrap(),
        },
    )
    .unwrap();
    assert!(combined.motions > combined.rough_vertices / 2);
    let handle = combined.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_owned();
    session::execute(
        &mut retained,
        Command::Prepare {
            job: doc.job.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    let feed = session::settings(&doc.job).vbit.cutting_feed_mm_min;
    authoring::set_mode(&mut doc.job, FlatVcarveMode::EndmillOnly);
    assert_eq!(session::settings(&doc.job).vbit.cutting_feed_mm_min, feed);
    assert_eq!(session::open(&doc.job.to_json().unwrap()).unwrap(), doc.job);
}
