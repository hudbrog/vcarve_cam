use cam_core::{
    project::{
        self,
        v5::{
            self, OperationSettingsV5,
            commands::{self, NewOperationKind},
        },
    },
    sequence::{GenerationStatus, OperationPlanV5, PlanLimits, StageRole},
    toolpath::{MotionEffect, MotionPurpose},
};

const RECT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="40mm" viewBox="0 0 80 40"><rect id="a" x="3" y="3" width="30" height="30"/><rect id="b" x="45" y="5" width="25" height="25"/></svg>"#;
const ISLAND: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 40 40"><path id="a" fill-rule="evenodd" d="M3 3H37V37H3Z M14 14H26V26H14Z"/></svg>"#;
const SMALL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><rect id="a" x="3" y="3" width="10" height="10"/></svg>"#;

fn job(svg: &str) -> v5::CamJobV5 {
    let job = v5::authoring::from_svg("pocket.svg".into(), svg.into(), 0.001).unwrap();
    let mut job = commands::add_operation(&job, NewOperationKind::Pocket, "pocket", "Pocket")
        .unwrap()
        .job;
    let catalogue = v5::inspect_artwork(&job).unwrap();
    let picks: Vec<_> = catalogue.items[0]
        .entries
        .iter()
        .filter(|e| e.kind == v5::GeometryRefKind::FilledComponent)
        .map(|e| v5::GeometryPick {
            artwork_item_id: e.reference.artwork_item_id.clone(),
            kind: e.kind,
            local_geometry_id: e.reference.local_geometry_id.clone(),
        })
        .collect();
    job = commands::set_component_selection(&job, "pocket", &picks)
        .unwrap()
        .job;
    job.tools[0].geometry = Some(project::ToolGeometry::Endmill(project::EndmillGeometry {
        diameter_mm: 4.,
        cutting_length_mm: 15.,
    }));
    job.tools[0].capabilities = project::ToolCapabilities {
        plunge_capable: Some(true),
        ramp_capable: Some(true),
    };
    let s = settings(&mut job);
    s.bottom.offset_mm = -2.5;
    s.direction = Some(project::CutDirection::Climb);
    s.assignment.spindle_rpm = Some(10000.);
    s.assignment.spindle_direction = Some(project::SpindleDirection::Clockwise);
    s.assignment.cutting_feed_mm_min = Some(600.);
    s.assignment.plunge_feed_mm_min = Some(150.);
    s.assignment.max_stepdown_mm = Some(1.);
    s.assignment.stepover_mm = Some(2.);
    job
}
fn settings(job: &mut v5::CamJobV5) -> &mut v5::PocketSettingsV5 {
    let OperationSettingsV5::Pocket(s) = &mut job.operations[0].settings else {
        panic!()
    };
    s
}
fn plan(job: &v5::CamJobV5) -> OperationPlanV5 {
    OperationPlanV5::plan_job_v5(job, &v5::ReadinessScope::AllEnabled, &PlanLimits::default())
        .unwrap()
}
fn complete(p: &OperationPlanV5) {
    assert_eq!(
        p.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        p.operation_results
    );
    let checks = cam_core::checks::check_plan_v5(p).unwrap();
    assert!(checks.export_ready, "{checks:?}");
}

#[test]
fn multiple_pockets_share_depth_and_end_on_partial_layer() {
    let p = plan(&job(RECT));
    complete(&p);
    assert_eq!(p.stages.len(), 2);
    let stock = p.stock_history(Some("pocket")).unwrap();
    assert_eq!(
        stock.batches.len(),
        2,
        "operation prefix includes every pocket stage"
    );
    assert_eq!(
        stock
            .material_top_at(cam_core::geometry::Point::new(55., 20.))
            .unwrap(),
        -2.5
    );
    assert!(p.stages.iter().all(|s| s.role == StageRole::PocketRough));
    assert_eq!(
        p.motions
            .iter()
            .filter(|m| m.effect == MotionEffect::MillingSweep)
            .map(|m| m.end.z)
            .fold(0_f64, f64::min),
        -2.5
    );
    assert!(p.motions.windows(2).all(|m| m[0].end == m[1].start));
    assert!(
        p.motions
            .iter()
            .filter(|m| m.purpose == MotionPurpose::Rough)
            .all(|m| [-1., -2., -2.5].contains(&m.end.z))
    );
}

#[test]
fn island_is_preserved_and_wall_finish_follows_roughing() {
    let mut job = job(ISLAND);
    settings(&mut job).wall_allowance_mm = Some(0.3);
    settings(&mut job).finish_walls = true;
    let p = plan(&job);
    complete(&p);
    assert_eq!(p.stages.len(), 2);
    assert_eq!(p.stages[1].role, StageRole::PocketFinish);
    assert!(
        p.motions
            .iter()
            .any(|m| m.purpose == MotionPurpose::Finish && m.end.z == -1.)
    );
}

#[test]
fn inaccessible_selected_pocket_and_resource_exhaustion_block_completion() {
    let mut j = job(RECT);
    if let Some(project::ToolGeometry::Endmill(g)) = &mut j.tools[0].geometry {
        g.diameter_mm = 26.;
    }
    assert_eq!(
        plan(&j).operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let mut j = job(RECT);
    settings(&mut j).limits.max_motions = 5;
    let p = plan(&j);
    assert_eq!(
        p.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(!cam_core::checks::check_plan_v5(&p).unwrap().export_ready);
}

#[test]
fn undeclared_plunge_and_below_stock_bottom_are_rejected() {
    let mut j = job(RECT);
    j.tools[0].capabilities.plunge_capable = None;
    assert_eq!(
        plan(&j).operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    j.tools[0].capabilities.plunge_capable = Some(true);
    settings(&mut j).bottom.offset_mm = -30.;
    assert_eq!(
        plan(&j).operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
}

#[test]
fn ramp_and_helix_work_without_plunge_capability() {
    for entry in [
        v5::PocketEntry::Ramp {
            max_angle_deg: Some(10.),
            feed_mm_min: Some(200.),
        },
        v5::PocketEntry::Helix {
            radius_mm: Some(1.),
            max_angle_deg: Some(10.),
            feed_mm_min: Some(200.),
        },
    ] {
        let mut j = job(SMALL);
        j.tools[0].capabilities.plunge_capable = Some(false);
        let helix = matches!(entry, v5::PocketEntry::Helix { .. });
        settings(&mut j).entry = entry;
        settings(&mut j).bottom.offset_mm = -1.3;
        let p = plan(&j);
        complete(&p);
        assert!(
            p.motions
                .iter()
                .filter(|m| m.purpose == MotionPurpose::Entry && m.start.z > m.end.z)
                .all(|m| m.start.xy() != m.end.xy())
        );
        if helix {
            let arcs: Vec<_> = p
                .motions
                .iter()
                .filter(|m| {
                    matches!(
                        m.interpolation,
                        cam_core::toolpath::Interpolation::ArcFeed(_)
                    )
                })
                .collect();
            assert!(!arcs.is_empty());
            for m in arcs {
                assert!(m.start.z - m.end.z <= 0.5 + 1e-9);
            }
        }
    }
}

#[test]
fn tangent_line_and_arc_leads_are_recorded_and_oversized_leads_refused() {
    for lead in [
        project::LeadSpec::TangentLine {
            length_mm: Some(0.2),
            feed_mm_min: Some(200.),
        },
        project::LeadSpec::TangentArc {
            radius_mm: Some(0.2),
            sweep_deg: Some(90.),
            feed_mm_min: Some(200.),
        },
    ] {
        let mut j = job(SMALL);
        settings(&mut j).bottom.offset_mm = -1.;
        settings(&mut j).lead_in = lead.clone();
        settings(&mut j).lead_out = lead;
        let p = plan(&j);
        complete(&p);
        assert!(p.motions.iter().any(|m| m.purpose == MotionPurpose::LeadIn));
        assert!(
            p.motions
                .iter()
                .any(|m| m.purpose == MotionPurpose::LeadOut)
        );
    }
    let mut j = job(SMALL);
    settings(&mut j).lead_in = project::LeadSpec::TangentLine {
        length_mm: Some(20.),
        feed_mm_min: Some(200.),
    };
    assert_eq!(
        plan(&j).operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
}

#[test]
fn helix_that_leaves_a_core_or_does_not_fit_is_refused() {
    for radius in [2.5, 1.8] {
        let mut j = job(SMALL);
        settings(&mut j).entry = v5::PocketEntry::Helix {
            radius_mm: Some(radius),
            max_angle_deg: Some(10.),
            feed_mm_min: Some(200.),
        };
        if radius < 2. {
            settings(&mut j).wall_allowance_mm = Some(1.5);
        }
        assert_eq!(
            plan(&j).operation_results[0].generation_status,
            GenerationStatus::Incomplete
        );
    }
}

#[test]
fn independent_verifier_rejects_missing_floor_and_island_crossing() {
    let j = job(ISLAND);
    let p = plan(&j);
    let OperationSettingsV5::Pocket(s) = &j.operations[0].settings else {
        panic!()
    };
    let resolved =
        v5::resolve::resolve_filled_region(&j, &s.components, &v5::inspect_artwork(&j).unwrap())
            .unwrap();
    let verify = |motions: &[cam_core::toolpath::PlannedMotion]| {
        cam_core::operations::pocket::verify_recorded_motions(
            &resolved.region,
            s,
            2.,
            0.,
            -2.5,
            0.05,
            motions,
        )
    };
    verify(&p.motions).unwrap();
    let mut missing = p.motions.clone();
    missing.retain(|m| !(m.layer == 1 && m.purpose == MotionPurpose::Rough));
    assert_eq!(verify(&missing).unwrap_err().code, "POCKET_COVERAGE");
    let mut crossing = p.motions.clone();
    let cut = crossing
        .iter_mut()
        .find(|m| m.purpose == MotionPurpose::Rough)
        .unwrap();
    cut.end.x = 20.;
    cut.end.y = 20.;
    assert_eq!(verify(&crossing).unwrap_err().code, "POCKET_CONTAINMENT");
    let mut deep = p.motions.clone();
    let cut = deep
        .iter_mut()
        .find(|m| m.purpose == MotionPurpose::Rough)
        .unwrap();
    cut.end.z = -2.;
    assert_eq!(verify(&deep).unwrap_err().code, "POCKET_STEPDOWN");
}

#[test]
fn helical_pocket_exports_and_numeric_readback_checks_the_z_arcs() {
    use cam_core::{
        post::{
            self,
            sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping},
        },
        sequence::TrustedPlanV5,
    };
    let mut j = job(SMALL);
    settings(&mut j).bottom.offset_mm = -1.3;
    settings(&mut j).entry = v5::PocketEntry::Helix {
        radius_mm: Some(1.),
        max_angle_deg: Some(10.),
        feed_mm_min: Some(200.),
    };
    let p = plan(&j);
    complete(&p);
    let profile = SequenceProfile {
        schema_version: 2,
        id: "pocket-test".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: post::LengthCompensation::MacroManaged,
        path_control: post::PathControl::ExactPath,
        tools: vec![SequenceToolMapping {
            tool_id: j.tools[0].id.clone(),
            tool_number: 1,
            length_offset_number: None,
        }],
        spindle_spinup_seconds: 0.5,
        holder: None,
        rapid_rate_mm_min: None,
        coolant: post::Coolant::Off,
        m6: post::LinuxCncProfile::from_json(include_str!(
            "../../../../real_data/machine-profile.json"
        ))
        .unwrap()
        .m6,
    };
    let trusted = TrustedPlanV5::from_generated(p);
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let exported = prepared.export(&trusted, &profile).unwrap();
    assert!(
        exported
            .program
            .gcode
            .lines()
            .any(|l| l.starts_with("G3 ") && l.contains("Z-"))
    );
    post::sequence::verify_program(&prepared, &trusted, &exported.program).unwrap();
    assert!(exported.report.basic_checks.export_ready);
}

#[test]
fn facing_establishes_pocket_top_and_later_incomplete_work_is_outside_prefix() {
    let mut j = job(SMALL);
    settings(&mut j).top = project::HeightRef {
        reference: project::HeightReference::FaceResult {
            operation_id: "face".into(),
        },
        offset_mm: 0.,
    };
    settings(&mut j).bottom.offset_mm = -1.3;
    let assignment = settings(&mut j).assignment.clone();
    let face = v5::OperationV5 {
        id: "face".into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettingsV5::Face(v5::FaceSettingsV5 {
            area: project::FaceArea::Rectangle {
                rect: j.setup.stock.xy.unwrap(),
            },
            margins: Default::default(),
            entry: Default::default(),
            entry_overrun_mm: Some(2.),
            exit_overrun_mm: Some(2.),
            top: project::HeightRef {
                reference: project::HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: project::HeightRef {
                reference: project::HeightReference::StockTop,
                offset_mm: -0.5,
            },
            stepdown_mm: Some(0.5),
            stepover_mm: Some(2.),
            pass_angle_deg: Some(0.),
            pattern: Default::default(),
            assignment,
        }),
    };
    j.operations.insert(0, face);
    j = commands::add_operation(&j, NewOperationKind::Profile, "later", "Later profile")
        .unwrap()
        .job;
    let scope = v5::ReadinessScope::ThroughOperation {
        operation_id: "pocket".into(),
    };
    let p = OperationPlanV5::plan_job_v5(&j, &scope, &PlanLimits::default()).unwrap();
    assert_eq!(p.operation_results.len(), 2);
    assert!(
        p.operation_results
            .iter()
            .all(|r| r.generation_status == GenerationStatus::Complete),
        "{:?}",
        p.operation_results
    );
    assert!(cam_core::checks::check_plan_v5(&p).unwrap().export_ready);
    assert!((p.motions.iter().map(|m| m.end.z).fold(0_f64, f64::min) + 1.8).abs() < 1e-9);
    j.operations[0].enabled = false;
    let invalid = OperationPlanV5::plan_job_v5(&j, &scope, &PlanLimits::default());
    assert!(
        invalid.is_err()
            || invalid.is_ok_and(|p| !cam_core::checks::check_plan_v5(&p).unwrap().export_ready)
    );
}

#[test]
fn curved_concave_and_split_center_regions_are_verified_or_explicitly_incomplete() {
    for shape in [
        r#"<circle cx="20" cy="20" r="13"/>"#,
        r#"<path d="M3 3H30V13H15V30H3Z"/>"#,
        r#"<path d="M3 3H15V12H24V3H36V27H24V16H15V27H3Z"/>"#,
    ] {
        let mut j = job(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="40mm" viewBox="0 0 40 40">{shape}</svg>"#
        ));
        settings(&mut j).bottom.offset_mm = -0.5;
        let p = plan(&j);
        if !shape.contains("H24") {
            complete(&p);
        }
        if p.operation_results[0].generation_status == GenerationStatus::Complete {
            complete(&p);
        } else {
            assert!(p.motions.is_empty());
            assert!(
                p.generation_diagnostics
                    .iter()
                    .any(|d| d.code == "POCKET_COVERAGE"),
                "{:?}",
                p.operation_results
            );
        }
    }
}

#[test]
fn multiple_helix_pockets_have_separate_entries_and_cut_direction_reverses() {
    let mut j = job(RECT);
    settings(&mut j).bottom.offset_mm = -0.7;
    settings(&mut j).entry = v5::PocketEntry::Helix {
        radius_mm: Some(1.),
        max_angle_deg: Some(10.),
        feed_mm_min: Some(200.),
    };
    let p = plan(&j);
    complete(&p);
    for stage in &p.stages {
        assert!(
            p.motions[stage.motion_range.0..stage.motion_range.1]
                .iter()
                .any(|m| matches!(
                    m.interpolation,
                    cam_core::toolpath::Interpolation::ArcFeed(_)
                ))
        );
    }
    let mut island = job(ISLAND);
    settings(&mut island).bottom.offset_mm = -0.5;
    settings(&mut island).finish_walls = true;
    let climb = plan(&island);
    settings(&mut island).direction = Some(project::CutDirection::Conventional);
    let conventional = plan(&island);
    complete(&conventional);
    let signed_runs = |p: &OperationPlanV5, purpose: MotionPurpose| {
        let mut areas = Vec::new();
        let mut area = 0.;
        let mut cutting = false;
        for m in &p.motions {
            if m.purpose == purpose {
                cutting = true;
                area += m.start.x * m.end.y - m.end.x * m.start.y;
            } else if cutting {
                areas.push(area);
                area = 0.;
                cutting = false;
            }
        }
        areas
    };
    let a = signed_runs(&climb, MotionPurpose::Rough);
    let b = signed_runs(&conventional, MotionPurpose::Rough);
    assert_eq!(a.len(), b.len());
    assert!(
        a.iter().any(|a| *a > 0.) && a.iter().any(|a| *a < 0.),
        "outer and island rings"
    );
    assert!(a.iter().zip(&b).all(|(a, b)| (a + b).abs() < 1e-6));
    let walls = signed_runs(&climb, MotionPurpose::Finish);
    assert_eq!(walls.len(), 2);
    assert!(walls[0] > 0., "CW-spindle climb outer wall is CCW");
    assert!(walls[1] < 0., "CW-spindle climb island wall is CW");
    let other = signed_runs(&conventional, MotionPurpose::Finish);
    assert!(walls.iter().zip(&other).all(|(a, b)| (a + b).abs() < 1e-6));
    settings(&mut island).direction = Some(project::CutDirection::Climb);
    settings(&mut island).assignment.spindle_direction =
        Some(project::SpindleDirection::Counterclockwise);
    assert_eq!(signed_runs(&plan(&island), MotionPurpose::Finish), other);
}
