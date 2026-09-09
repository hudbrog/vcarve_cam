//! B2: ordered stock history accumulation, prefix identities that stale the
//! suffix, and concrete height-reference resolution.
use cam_core::{
    geometry::Point,
    motion::Position,
    project::{HeightRef, OperationSettings, RectXY, migrate::migrate_legacy_json},
    sequence::{OperationPlan, PlanLimits},
    setup::{ResolvedHeights, resolve_heights},
    stock::history::{StockHistory, SweepBatch, SweepCutter, SweepMotion},
};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");

fn cut(start: (f64, f64, f64), end: (f64, f64, f64)) -> SweepMotion {
    SweepMotion {
        start: Position {
            x: start.0,
            y: start.1,
            z: start.2,
        },
        end: Position {
            x: end.0,
            y: end.1,
            z: end.2,
        },
    }
}

#[test]
fn two_synthetic_stages_accumulate_removal() {
    let stock = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 100.,
        length_mm: 60.,
    };
    let mut history = StockHistory::new(8., Some(stock)).unwrap();
    // Stage one: a 4 mm endmill slot at z=-2 along y=10, x in [10,40].
    history.push(SweepBatch {
        stage_id: "op1-rough".into(),
        operation_id: "op1".into(),
        cutter: SweepCutter::FlatEndmill { radius_mm: 2. },
        motions: vec![cut((10., 10., -2.), (40., 10., -2.))],
    });
    // Stage two: the same cutter deepens an overlapping section to z=-4.
    history.push(SweepBatch {
        stage_id: "op2-rough".into(),
        operation_id: "op2".into(),
        cutter: SweepCutter::FlatEndmill { radius_mm: 2. },
        motions: vec![cut((20., 10., -4.), (30., 10., -4.))],
    });

    near(
        history.material_top_at(Point::new(15., 10.)).unwrap(),
        -2.,
        "stage-one only",
    );
    near(
        history.material_top_at(Point::new(25., 10.)).unwrap(),
        -4.,
        "overlapped by stage two",
    );
    near(
        history.material_top_at(Point::new(15., 11.5)).unwrap(),
        -2.,
        "within the cutter radius of stage one",
    );
    near(
        history.material_top_at(Point::new(15., 20.)).unwrap(),
        0.,
        "untouched stock top",
    );
    near(
        history.material_top_at(Point::new(15., 11.9)).unwrap(),
        -2.,
        "radius boundary region",
    );
    near(
        history.material_top_at(Point::new(15., 12.2)).unwrap(),
        0.,
        "just outside the radius",
    );
    assert_eq!(
        history
            .material_top_at(Point::new(150., 10.))
            .unwrap_err()
            .code,
        "STOCK_POINT_OUTSIDE"
    );
    // A deeper cut than the stock is clamped at the physical bottom.
    let mut through = StockHistory::new(2., Some(stock)).unwrap();
    through.push(SweepBatch {
        stage_id: "deep".into(),
        operation_id: "op".into(),
        cutter: SweepCutter::FlatEndmill { radius_mm: 2. },
        motions: vec![cut((0., 0., -5.), (10., 0., -5.))],
    });
    near(
        through.material_top_at(Point::new(5., 0.)).unwrap(),
        -2.,
        "clamped at stock bottom",
    );

    // Traverse: a transit at clearance passes over cut and uncut material.
    let uncut = history
        .can_traverse_without_cutting(Point::new(0., 30.), Point::new(100., 30.), 2., 5.)
        .unwrap();
    assert!(uncut, "clearance transit above untouched material");
    let colliding = history
        .can_traverse_without_cutting(Point::new(15., 10.), Point::new(40., 10.), 2., -3.)
        .unwrap();
    assert!(!colliding, "transit below the remaining floor collides");
}

#[test]
fn vbit_sweeps_follow_the_conical_removal_model() {
    let stock = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    };
    let mut history = StockHistory::new(8., Some(stock)).unwrap();
    // 90-degree V-bit, 0 tip: plunging to z=-2 removes a 2 mm-deep wedge.
    history.push(SweepBatch {
        stage_id: "vbit".into(),
        operation_id: "op".into(),
        cutter: SweepCutter::VBit {
            spec: cam_core::model::VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: 0.,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 6.,
            },
        },
        motions: vec![cut((10., 15., -2.), (30., 15., -2.))],
    });
    near(
        history.material_top_at(Point::new(20., 15.)).unwrap(),
        -2.,
        "on the tip path",
    );
    near(
        history.material_top_at(Point::new(20., 16.)).unwrap(),
        -1.,
        "1 mm off-axis halves the depth",
    );
    near(
        history.material_top_at(Point::new(20., 17.)).unwrap(),
        0.,
        "beyond the cone edge",
    );
}

fn near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{what}: {actual} != {expected}"
    );
}

fn two_operation_job() -> cam_core::project::CamJob {
    let mut job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let mut second = job.operations[0].clone();
    second.id = "carve-2".into();
    second.name = "Second carve".into();
    job.operations.push(second);
    job.validate().unwrap();
    job
}

#[test]
fn prefix_identities_stale_the_suffix_when_an_earlier_operation_changes() {
    let job = two_operation_job();
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let first = &plan.operation_results[0];
    let second = &plan.operation_results[1];
    assert_eq!(
        first.stock_after_id, second.stock_before_id,
        "the suffix starts exactly at the previous after-state"
    );

    // Changing the first operation's depth stales every later stock id while
    // the second operation's own motions stay geometrically identical.
    let mut edited = job.clone();
    let OperationSettings::FlatVcarve(settings) = &mut edited.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    settings.max_depth_mm = Some(1.5);
    let replanned = OperationPlan::plan_job(&edited, &PlanLimits::default()).unwrap();
    assert_ne!(
        plan.operation_results[0].stock_after_id, replanned.operation_results[0].stock_after_id,
        "the edited operation's stock identity changes"
    );
    assert_ne!(
        plan.operation_results[1].stock_before_id, replanned.operation_results[1].stock_before_id,
        "the suffix is staled"
    );
    assert_eq!(
        serde_json::to_value(&plan.operation_results[1].stage_ids).unwrap(),
        serde_json::to_value(&replanned.operation_results[1].stage_ids).unwrap(),
        "the unchanged operation keeps its own stage identity"
    );
}

#[test]
fn plan_history_accumulates_the_executed_prefix() {
    let job = two_operation_job();
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let full = plan.stock_history(None).unwrap();
    assert_eq!(full.batches.len(), 2, "one batch per nonempty stage");
    let prefix = plan.stock_history(Some("carve-2")).unwrap();
    assert_eq!(
        prefix.batches.len(),
        2,
        "the prefix ends at the named operation"
    );
    let first_only = plan.stock_history(Some("flat-v-carve")).unwrap();
    assert_eq!(first_only.batches.len(), 1);

    // The fixture's placement puts the carved rectangle around x 3.5..26.5,
    // y 13.5..26.5 with a depth-2 cut floor.
    let center = Point::new(15., 20.);
    let top = first_only.material_top_at(center).unwrap();
    assert!(
        (-2. ..=-1.5).contains(&top),
        "carved center around the cut floor, got {top}"
    );
    let outside = Point::new(35., 5.);
    assert_eq!(
        first_only.material_top_at(outside).unwrap(),
        0.,
        "material outside the selection stays untouched"
    );
}

#[test]
fn height_references_resolve_with_explicit_failures() {
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let empty = std::collections::BTreeMap::new();
    // Simple references: face-like top/bottom over 8 mm stock.
    let resolved = resolve_heights(
        &job,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: 0.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -0.5,
        },
        &empty,
    )
    .unwrap();
    assert_eq!(
        resolved,
        ResolvedHeights {
            top_z: 0.,
            bottom_z: -0.5
        }
    );
    // OperationTop as a bottom resolves to the operation's own top.
    let resolved = resolve_heights(
        &job,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: 0.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::OperationTop,
            offset_mm: -2.,
        },
        &empty,
    )
    .unwrap();
    assert_eq!(resolved.bottom_z, -2.);
    // A FaceResult reference without an established plane stays explicit.
    let err = resolve_heights(
        &job,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::FaceResult {
                operation_id: "face-1".into(),
            },
            offset_mm: 0.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -1.,
        },
        &empty,
    )
    .unwrap_err();
    assert_eq!(err.code, "HEIGHT_REFERENCE_UNRESOLVED");
    // With a published plane, the same reference resolves (C1 publishes).
    let mut planes = std::collections::BTreeMap::new();
    planes.insert("face-1".to_string(), -0.5_f64);
    let resolved = resolve_heights(
        &job,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::FaceResult {
                operation_id: "face-1".into(),
            },
            offset_mm: 0.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::OperationTop,
            offset_mm: -2.,
        },
        &planes,
    )
    .unwrap();
    assert_eq!(
        resolved,
        ResolvedHeights {
            top_z: -0.5,
            bottom_z: -2.5
        }
    );
    // Bottom at or above top is a range error, never a silent swap.
    let err = resolve_heights(
        &job,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -1.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -1.,
        },
        &empty,
    )
    .unwrap_err();
    assert_eq!(err.code, "HEIGHT_RANGE_INVALID");
    // Missing thickness is explicit.
    let mut thin = job.clone();
    thin.setup.stock.thickness_mm = None;
    let err = resolve_heights(
        &thin,
        0,
        &HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: 0.,
        },
        &HeightRef {
            reference: cam_core::project::HeightReference::StockBottom,
            offset_mm: 0.,
        },
        &empty,
    )
    .unwrap_err();
    assert_eq!(err.code, "SETUP_STOCK_THICKNESS_REQUIRED");
    // Flat V-carve settings keep carrying their scalar depth field.
    let OperationSettings::FlatVcarve(settings) = &job.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    assert_eq!(settings.max_depth_mm, Some(2.));
}
