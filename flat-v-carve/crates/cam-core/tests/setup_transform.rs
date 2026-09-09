//! B1 setup fixtures: work-zero resolution and the output transform from plan
//! section 19.1. Coordinates are setup-space (stock top at Z=0) and the
//! transform is `output = setup - work_zero_point` per axis.
use cam_core::{
    checks::CheckStatus,
    post::{
        LinuxCncProfile,
        sequence::{PreparedExecution, SequenceProfile},
    },
    project::{
        AnchorFraction, CamJob, OperationSettings, RectXY, StockSetup, WorkZeroXY, WorkZeroZ,
        migrate::migrate_legacy_json,
    },
    sequence::{OperationPlan, PlanLimits, TrustedPlan},
    setup::resolve_work_zero,
};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");

fn applied() -> CamJob {
    let profile = LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap();
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    cam_core::post::sequence::apply_legacy_profile(&profile, &job).unwrap()
}

/// Stock 100 x 60 x 8 at XY=(0,0); work zero stock-center / stock-bottom.
fn center_bottom_job() -> CamJob {
    let mut job = applied();
    job.setup.stock.xy = Some(RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 100.,
        length_mm: 60.,
    });
    job.setup.work_zero = cam_core::project::WorkZero {
        xy: WorkZeroXY::StockAnchor {
            x_fraction: AnchorFraction::Center,
            y_fraction: AnchorFraction::Center,
        },
        z: WorkZeroZ::StockBottom,
    };
    job.validate().unwrap();
    job
}

fn sequence_profile() -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "printnc".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: [1u32, 2]
            .iter()
            .enumerate()
            .map(
                |(index, number)| cam_core::post::sequence::SequenceToolMapping {
                    tool_id: if index == 0 { "endmill" } else { "vbit" }.into(),
                    tool_number: *number,
                    length_offset_number: None,
                },
            )
            .collect(),
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap().m6,
    }
}

#[test]
fn center_bottom_work_zero_resolves_and_transforms_the_documented_fixture() {
    let job = center_bottom_job();
    let work_zero = resolve_work_zero(&job).unwrap();
    assert_eq!(work_zero.x_mm, 50.);
    assert_eq!(work_zero.y_mm, 30.);
    assert_eq!(work_zero.z_mm, -8.);
    let (wx, wy, wz) = work_zero.output_offset();
    // Plan section 19.1: the setup point (10,20,-2) reads back from output
    // (-40,-10,6); clearance +5 becomes +13.
    let point = (10., 20., -2.);
    assert_eq!(point.0 - wx, -40.);
    assert_eq!(point.1 - wy, -10.);
    assert_eq!(point.2 - wz, 6.);
    let clearance = 5.;
    assert_eq!(clearance - wz, 13.);
}

#[test]
fn center_bottom_export_passes_numeric_readback() {
    let job = center_bottom_job();
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let trusted = TrustedPlan::from_generated(plan);
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    assert_eq!(prepared.machine_offset_mm, [50., 30., -8.]);
    let export = prepared.export(&trusted, &profile).unwrap();
    // Machine Z of the first motion is shifted by the stock thickness; the
    // readback inside export() compared every emitted coordinate against the
    // transformed plan, and its report is published only when they match.
    assert!(
        export.program.gcode.contains("Z13.000"),
        "clearance at machine Z13.000"
    );
    assert_eq!(export.report.machine_offset_mm, [50., 30., -8.]);
    assert_eq!(export.report.basic_checks.status, CheckStatus::Passed);
}

#[test]
fn changing_work_zero_changes_output_only_not_planned_removal() {
    // Two jobs with identical machining inputs and physical stock; only the
    // XY work-zero selection differs (both resolve against the same stock).
    let mut origin = center_bottom_job();
    origin.setup.work_zero.xy = WorkZeroXY::SetupOrigin;
    let anchor = center_bottom_job();
    let plan_a = OperationPlan::plan_job(&origin, &PlanLimits::default()).unwrap();
    let plan_b = OperationPlan::plan_job(&anchor, &PlanLimits::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&plan_a.motions).unwrap(),
        serde_json::to_value(&plan_b.motions).unwrap(),
        "work zero must not move artwork or planned removal"
    );
    // The output transform differs even though the motions do not.
    let profile = sequence_profile();
    let prepared_a =
        PreparedExecution::prepare(&TrustedPlan::from_generated(plan_a), &profile).unwrap();
    let prepared_b =
        PreparedExecution::prepare(&TrustedPlan::from_generated(plan_b), &profile).unwrap();
    assert_ne!(prepared_a.machine_offset_mm, prepared_b.machine_offset_mm);
    // Custom points transform identically to anchors resolving to the same spot.
    let mut custom = origin;
    custom.setup.work_zero.xy = WorkZeroXY::CustomPoint {
        x_mm: 50.,
        y_mm: 30.,
    };
    custom.validate().unwrap();
    let prepared_custom = PreparedExecution::prepare(
        &TrustedPlan::from_generated(
            OperationPlan::plan_job(&custom, &PlanLimits::default()).unwrap(),
        ),
        &profile,
    )
    .unwrap();
    assert_eq!(prepared_custom.machine_offset_mm[0..2], [50., 30.]);
}

#[test]
fn legacy_unknown_xy_stays_explicit() {
    let job = applied();
    // Migrated documents keep null XY: the setup origin resolves and the
    // display stays marked as without physical bounds.
    assert_eq!(job.setup.stock.xy, None);
    assert_eq!(
        job.setup.stock,
        StockSetup {
            thickness_mm: Some(8.),
            xy: None
        }
    );
    let work_zero = resolve_work_zero(&job).unwrap();
    assert_eq!(work_zero.output_offset(), (0., 0., -8.));
    // Selecting an anchor without physical stock is a located error.
    let mut anchored = job.clone();
    anchored.setup.work_zero.xy = WorkZeroXY::StockAnchor {
        x_fraction: AnchorFraction::Min,
        y_fraction: AnchorFraction::Min,
    };
    let err = resolve_work_zero(&anchored).unwrap_err();
    assert_eq!(err.code, "SETUP_STOCK_XY_REQUIRED");
    // Stock-bottom datum without thickness is equally explicit.
    let OperationSettings::FlatVcarve(_) = &anchored.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    let mut thin = anchored;
    thin.setup.stock.thickness_mm = None;
    let err = resolve_work_zero(&thin).unwrap_err();
    assert_eq!(err.code, "SETUP_STOCK_THICKNESS_REQUIRED");
}
