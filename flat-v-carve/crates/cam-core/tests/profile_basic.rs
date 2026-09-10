//! D2 basic profile: explicit-side offsets (plan section 19.1 circle
//! fixture), climb/conventional traversal from the retained side, exact
//! depth layers with through allowance, nested-cutout ordering, and
//! diagnosed offset collapse. Profiles need no V-bit.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, ContourOrder, ContourSide, CutDirection, JobTool, MillingAssignment, Operation,
        OperationSettings, ProfileContour, ProfileEntry, ProfileSettings, RectXY, SetupSettings,
        SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry, TraversalDirection, WorkZero,
        WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits},
    toolpath::{MotionEffect, MotionPurpose},
};

const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><circle id="cut" cx="25" cy="25" r="20"/></svg>"#;
const NESTED_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40"><path id="frame" fill-rule="evenodd" d="M0 0h60v40h-60z M5 5h50v30h-50z"/><rect id="isle" x="20" y="12" width="20" height="16"/></svg>"#;
/// The nested artwork with every subpath traversed backwards.
const NESTED_REVERSED_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40"><path id="frame" fill-rule="evenodd" d="M0 0v40h60v-40z M5 5v30h50v-30z"/><rect id="isle" x="20" y="12" width="20" height="16"/></svg>"#;
const TINY_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm" viewBox="0 0 10 10"><circle id="tiny" cx="5" cy="5" r="2"/></svg>"#;

fn endmill() -> JobTool {
    JobTool {
        id: "endmill".into(),
        name: "6mm endmill".into(),
        geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: 6.,
            cutting_length_mm: 15.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: Some(false),
        },
    }
}

fn assignment() -> MillingAssignment {
    MillingAssignment {
        tool_id: "endmill".into(),
        spindle_rpm: Some(12_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(400.),
        plunge_feed_mm_min: Some(120.),
        max_stepdown_mm: Some(8.),
        stepover_mm: None,
    }
}

fn heights(depth: f64) -> (cam_core::project::HeightRef, cam_core::project::HeightRef) {
    use cam_core::project::{HeightRef, HeightReference};
    (
        HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: 0.,
        },
        HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: -depth,
        },
    )
}

fn profile_op(
    contours: Vec<ProfileContour>,
    depth: f64,
    stepdown: f64,
    through: Option<f64>,
    direction: Option<CutDirection>,
    spindle: SpindleDirection,
) -> Operation {
    let (top, bottom) = heights(depth);
    Operation {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours,
            assignment: MillingAssignment {
                spindle_direction: Some(spindle),
                ..assignment()
            },
            top,
            bottom,
            stepdown_mm: Some(stepdown),
            through_cut_allowance_mm: through,
            direction,
            order: ContourOrder::InnerBeforeOuter,
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

fn contour_ref(id: &str, side: ContourSide) -> ProfileContour {
    ProfileContour {
        contour_id: id.into(),
        side,
        traversal: if side == ContourSide::On {
            Some(TraversalDirection::Forward)
        } else {
            None
        },
    }
}

fn job(svg: &str, operation: Operation, stock: (f64, f64, f64)) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "profile".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(stock.2),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: stock.0,
                    length_mm: stock.1,
                }),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![endmill()],
        operations: vec![operation],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

/// Cut motions of the plan: feed moves that remove material.
fn cut_motions(plan: &OperationPlan) -> Vec<&cam_core::toolpath::PlannedMotion> {
    plan.motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough && m.effect == MotionEffect::MillingSweep)
        .collect()
}

#[test]
fn circle_fixture_has_expected_center_radius_for_every_side() {
    // Plan section 19.1: circle radius 20, cutter radius 3 — outside center
    // radius 23, inside 17, on-contour 20. The single endmill tool is the
    // only tool in the job: profiles need no V-bit.
    let center = Point::new(25., 25.);
    for (side, expected) in [
        (ContourSide::Outside, 23.),
        (ContourSide::Inside, 17.),
        (ContourSide::On, 20.),
    ] {
        let op = profile_op(
            vec![contour_ref("cut-0-outer", side)],
            5.,
            8.,
            None,
            Some(CutDirection::Climb),
            SpindleDirection::Clockwise,
        );
        let plan =
            OperationPlan::plan_job(&job(CIRCLE_SVG, op, (60., 60., 8.)), &PlanLimits::default())
                .unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            GenerationStatus::Complete,
            "{side:?} plans completely: {:?}",
            plan.generation_diagnostics
        );
        assert_eq!(plan.stages.len(), 1);
        assert_eq!(plan.stages[0].tool_id, "endmill");
        assert_eq!(
            plan.stages[0].role,
            cam_core::sequence::StageRole::ProfileRough
        );
        let cuts = cut_motions(&plan);
        assert!(!cuts.is_empty());
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for motion in &cuts {
            for p in [motion.start, motion.end] {
                let r = center.distance(Point::new(p.x, p.y));
                min = min.min(r);
                max = max.max(r);
            }
        }
        assert!(
            (min - expected).abs() < 0.02 && (max - expected).abs() < 0.02,
            "{side:?}: center radius {min}..{max}, expected {expected}"
        );
    }
}

#[test]
fn cut_direction_and_spindle_decide_traversal() {
    // Climb keeps retained material on the feed-left side for a clockwise
    // spindle: outside a circle that means counter-clockwise (positive signed
    // loop area); conventional or a mirrored spindle reverses it.
    let signed_area = |plan: &OperationPlan| {
        let layer: Vec<_> = cut_motions(plan)
            .into_iter()
            .filter(|m| (m.end.z - -5.).abs() < 1e-9)
            .collect();
        assert!(!layer.is_empty());
        let mut previous = layer[0].start;
        let area: f64 = layer
            .iter()
            .map(|m| {
                let a = previous;
                previous = m.end;
                a.x * m.end.y - m.end.x * a.y
            })
            .sum();
        area
    };
    let cases = [
        (CutDirection::Climb, SpindleDirection::Clockwise, true),
        (
            CutDirection::Conventional,
            SpindleDirection::Clockwise,
            false,
        ),
        (
            CutDirection::Climb,
            SpindleDirection::Counterclockwise,
            false,
        ),
        (
            CutDirection::Conventional,
            SpindleDirection::Counterclockwise,
            true,
        ),
    ];
    for (direction, spindle, ccw) in cases {
        let op = profile_op(
            vec![contour_ref("cut-0-outer", ContourSide::Outside)],
            5.,
            8.,
            None,
            Some(direction),
            spindle,
        );
        let plan =
            OperationPlan::plan_job(&job(CIRCLE_SVG, op, (60., 60., 8.)), &PlanLimits::default())
                .unwrap();
        let area = signed_area(&plan);
        assert_eq!(
            area > 0.,
            ccw,
            "{direction:?}/{spindle:?}: signed loop area {area}"
        );
    }
}

#[test]
fn nested_cutouts_cut_inner_first_with_exact_layers_and_through_allowance() {
    let contours = vec![
        contour_ref("frame-0-outer", ContourSide::Outside),
        contour_ref("frame-0-hole-0", ContourSide::Inside),
        contour_ref("isle-0-outer", ContourSide::Outside),
    ];
    // 8 mm stock, depth 8.2, stepdown 3: layers end at -3, -6, -8.2 with the
    // below-stock permission explicit.
    let op = profile_op(
        contours,
        8.2,
        3.,
        Some(0.2),
        Some(CutDirection::Climb),
        SpindleDirection::Clockwise,
    );
    let plan =
        OperationPlan::plan_job(&job(NESTED_SVG, op, (70., 50., 8.)), &PlanLimits::default())
            .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    // Inner-before-outer: the isle (nested deepest) first, then the frame
    // hole, then the enclosing frame cutout.
    let mut order: Vec<String> = vec![];
    for motion in cut_motions(&plan) {
        let id = motion.contour_id.clone().expect("cuts carry contour ids");
        if order.last().map(|last| *last != id).unwrap_or(true) {
            order.push(id);
        }
    }
    assert_eq!(
        order,
        [
            "isle-0-outer".to_string(),
            "frame-0-hole-0".to_string(),
            "frame-0-outer".to_string()
        ],
        "execution order"
    );
    // Depth layers per contour: exactly -3, -6, -8.2.
    for contour in ["isle-0-outer", "frame-0-hole-0", "frame-0-outer"] {
        let mut depths: Vec<f64> = cut_motions(&plan)
            .into_iter()
            .filter(|m| m.contour_id.as_deref() == Some(contour))
            .map(|m| m.end.z)
            .collect();
        depths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        depths.dedup();
        assert_eq!(
            depths,
            vec![-8.2, -6., -3.],
            "{contour} layers end exactly at the through-cut bottom"
        );
    }
    // Basic checks pass for the whole profile plan.
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);
}

#[test]
fn reversed_source_winding_keeps_retained_side_and_order() {
    let build = |svg: &str| {
        let contours = vec![
            contour_ref("frame-0-outer", ContourSide::Outside),
            contour_ref("frame-0-hole-0", ContourSide::Inside),
        ];
        let op = profile_op(
            contours,
            5.,
            5.,
            None,
            Some(CutDirection::Climb),
            SpindleDirection::Clockwise,
        );
        OperationPlan::plan_job(&job(svg, op, (70., 50., 8.)), &PlanLimits::default()).unwrap()
    };
    let forward = build(NESTED_SVG);
    let reversed = build(NESTED_REVERSED_SVG);
    assert_eq!(forward.motions.len(), reversed.motions.len());
    for (a, b) in forward.motions.iter().zip(&reversed.motions) {
        assert_eq!(a.contour_id, b.contour_id);
        assert!((a.start.x - b.start.x).abs() < 1e-9);
        assert!((a.start.y - b.start.y).abs() < 1e-9);
        assert!((a.start.z - b.start.z).abs() < 1e-9);
        assert!((a.end.x - b.end.x).abs() < 1e-9);
        assert!((a.end.y - b.end.y).abs() < 1e-9);
        assert!((a.end.z - b.end.z).abs() < 1e-9);
        assert_eq!(a.interpolation, b.interpolation);
    }
}

#[test]
fn offset_collapse_is_diagnosed_not_skipped() {
    // Circle radius 2 with cutter radius 3: the inside-compensated path
    // collapses; the contour is diagnosed, not silently omitted.
    let op = profile_op(
        vec![contour_ref("tiny-0-outer", ContourSide::Inside)],
        2.,
        2.,
        None,
        Some(CutDirection::Climb),
        SpindleDirection::Clockwise,
    );
    let plan = OperationPlan::plan_job(&job(TINY_SVG, op, (20., 20., 8.)), &PlanLimits::default())
        .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let diagnostic = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "PROFILE_OFFSET_UNAVAILABLE")
        .expect("collapse diagnostic");
    assert!(diagnostic.message.contains("tiny-0-outer"));
    assert!(!cam_core::checks::check_plan(&plan).unwrap().export_ready);
}

#[test]
fn missing_direction_and_later_slice_features_are_explicit() {
    let base = |settings: ProfileSettings| {
        OperationPlan::plan_job(
            &job(
                CIRCLE_SVG,
                Operation {
                    id: "profile-1".into(),
                    name: "Profile".into(),
                    enabled: true,
                    settings: OperationSettings::Profile(settings),
                },
                (60., 40., 8.),
            ),
            &PlanLimits::default(),
        )
        .unwrap()
    };
    // A retained side without a climb/conventional choice is a missing field.
    let mut settings = profile_settings_for(CIRCLE_SVG, Some(CutDirection::Climb));
    settings.direction = None;
    let plan = base(settings);
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "MISSING_MACHINING_SETTING"
                && d.message.contains("climb or conventional"))
    );

    // Later-slice features produce specific diagnostics, never silent drops:
    // a ramp on this fixture's non-ramp-capable endmill is a capability
    // conflict (the ramp itself ships with the E3 entries slice).
    let mut settings = profile_settings_for(CIRCLE_SVG, Some(CutDirection::Climb));
    settings.entry = ProfileEntry::Ramp {
        max_angle_deg: Some(15.),
        feed_mm_min: Some(120.),
    };
    let plan = base(settings);
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_ENTRY_CAPABILITY")
    );
    // And an unknown contour reference is located.
    let mut settings = profile_settings_for(CIRCLE_SVG, Some(CutDirection::Climb));
    settings.contours = vec![contour_ref("ghost-0-outer", ContourSide::Outside)];
    let plan = base(settings);
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "CONTOUR_REFERENCE")
    );
}

fn profile_settings_for(_svg: &str, direction: Option<CutDirection>) -> ProfileSettings {
    let (top, bottom) = heights(5.);
    ProfileSettings {
        contours: vec![contour_ref("cut-0-outer", ContourSide::Outside)],
        assignment: assignment(),
        top,
        bottom,
        stepdown_mm: Some(5.),
        through_cut_allowance_mm: None,
        direction,
        order: ContourOrder::InnerBeforeOuter,
        start: Default::default(),
        finish: Default::default(),
        entry: Default::default(),
        lead_in: Default::default(),
        lead_out: Default::default(),
        tabs: None,
    }
}
