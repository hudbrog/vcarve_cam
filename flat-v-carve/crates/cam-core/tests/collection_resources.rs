//! H4 applied resources and service inspection (plan sections 22.6, 22.7 and
//! 22.9): cutting-profile baselines are independent copied values with no
//! live library dependency (scenario 7), the one applied machine
//! configuration validates mappings per executable scope while incomplete
//! snapshots stay saveable (scenario 8), inspection DTOs expose used-by
//! references, resolved heights and generated evidence, and collection plans
//! export and replay through the same post pipeline as schema-4 plans.
use cam_core::{
    checks::check_plan_v5,
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    model::EndmillSpec,
    operations::drag_knife::evidence::{EvidenceStatus, build_evidence},
    post::{
        Coolant, LengthCompensation, PathControl,
        sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping},
    },
    project::{
        ContourOrder, ContourSide, CutDirection, DragKnifeSpec, EndmillGeometry, FaceArea,
        FaceMargins, FlatVcarveMode, HeightRef, HeightReference, KnifeAlignment, ProfileEntry,
        RectXY, SetupSettings, SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry,
        WorkZero,
        v5::{
            self, ArtworkContent, ArtworkItemId, CamJobV5, GeometryRef, GeometryRefKind,
            OperationSettingsV5, OperationV5, ReadinessScope,
            inspection::{inspect_document, inspect_plan},
            machine::{apply_machine_configuration, resolve_sequence_profile, set_tool_mapping},
            resources::{
                AssignmentRole as Role, ProfileStatus, apply_cutting_profile,
                apply_tool_to_assignment, assignment_statuses, reapply_profile, reset_assignment,
            },
        },
    },
    sequence::{OperationPlanV5, PlanLimits, TrustedPlanV5},
    svg::{ImportMode, Placement},
    tool_library::{
        CuttingPreset, KnifeCuttingPreset, LIBRARY_SCHEMA_VERSION, LibraryGeometry, LibraryTool,
        ToolLibrary,
    },
};

/// One filled plate plus one stroked centerline (the H3 fixture geometry).
const PLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="16" height="10" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M25 25 L35 25"/></svg>"##;

fn m6() -> cam_core::post::M6Contract {
    cam_core::post::M6Contract {
        reference: "test-contract".into(),
        reviewed: true,
        return_position: cam_core::post::M6Return::CallerPosition,
        preserves_work_datum: true,
        local_offsets_unused: true,
        tool_offsets_z_only: true,
    }
}

fn milling(tool_id: &str) -> v5::MillingAssignmentV5 {
    v5::MillingAssignmentV5 {
        tool_id: tool_id.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
        applied_profile: None,
    }
}

fn knife_assignment() -> v5::KnifeAssignmentV5 {
    v5::KnifeAssignmentV5 {
        tool_id: "t3".into(),
        cutting_feed_mm_min: Some(150.),
        plunge_feed_mm_min: Some(50.),
        swivel_feed_mm_min: Some(75.),
        max_stepdown_mm: Some(1.),
        applied_profile: None,
    }
}

fn setup() -> SetupSettings {
    SetupSettings {
        stock: StockSetup {
            thickness_mm: Some(8.),
            xy: Some(RectXY {
                min_x_mm: 0.,
                min_y_mm: 0.,
                width_mm: 40.,
                length_mm: 30.,
            }),
        },
        work_zero: WorkZero::default(),
        clearance_above_stock_mm: Some(5.),
        start_xy_mm: Some(Point::new(0., 0.)),
    }
}

fn tools() -> Vec<v5::JobToolV5> {
    let tool = |id: &str, name: &str, geometry| v5::JobToolV5 {
        id: id.into(),
        name: name.into(),
        geometry: Some(geometry),
        capabilities: ToolCapabilities::default(),
        library_origin: None,
    };
    vec![
        tool(
            "t1",
            "3mm endmill",
            ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 3.,
                cutting_length_mm: 8.,
            }),
        ),
        tool(
            "t3",
            "drag knife",
            ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 2.,
            }),
        ),
    ]
}

fn base_job() -> CamJobV5 {
    CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "resources".into(),
        setup: setup(),
        artwork: vec![v5::ArtworkItem {
            id: ArtworkItemId("plate".into()),
            name: "Plate".into(),
            content: ArtworkContent::Svg(SourceSnapshot {
                filename: "plate.svg".into(),
                svg: PLATE.into(),
            }),
            import_settings: v5::SvgInterpretation {
                geometry_tolerance_mm: 0.001,
                ticks_per_mm: None,
                mode: ImportMode::Centerline,
            },
            placement: Placement {
                origin_mm: Point::new(0., 0.),
                scale: 1.,
                rotation_deg: 0.,
            },
        }],
        tools: tools(),
        operations: vec![],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    }
}

/// Resolve the current live reference of one catalogue entry so selections
/// bind the item's actual revision.
fn reference_of(kind: GeometryRefKind, local_id: &str) -> GeometryRef {
    let combined = v5::artwork::inspect_artwork(&base_job()).unwrap();
    combined
        .item(&ArtworkItemId("plate".into()))
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.kind == kind && entry.reference.local_geometry_id == local_id)
        .unwrap_or_else(|| panic!("entry {local_id}/{kind:?} missing"))
        .reference
        .clone()
}

fn flat_vcarve_operation(component: GeometryRef) -> OperationV5 {
    OperationV5 {
        id: "carve-1".into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettingsV5::FlatVcarve(v5::FlatVcarveSettingsV5 {
            components: vec![component],
            mode: FlatVcarveMode::EndmillOnly,
            endmill: milling("t1"),
            vbit: v5::MillingAssignmentV5 {
                tool_id: "t1".into(),
                spindle_rpm: None,
                spindle_direction: None,
                cutting_feed_mm_min: None,
                plunge_feed_mm_min: None,
                max_stepdown_mm: None,
                stepover_mm: None,
                applied_profile: None,
            },
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            max_depth_mm: Some(1.5),
            wall_allowance_mm: Some(0.2),
            max_floor_ridge_mm: None,
            max_detail_residual_mm: None,
            rough: None,
            finish: None,
        }),
    }
}

fn profile_operation(contour: GeometryRef, top: HeightRef) -> OperationV5 {
    OperationV5 {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettingsV5::Profile(v5::ProfileSettingsV5 {
            contours: vec![v5::ProfileContourV5 {
                geometry: contour,
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: milling("t1"),
            top,
            bottom: HeightRef {
                reference: HeightReference::OperationTop,
                offset_mm: -2.,
            },
            stepdown_mm: Some(2.),
            through_cut_allowance_mm: None,
            direction: Some(CutDirection::Climb),
            order: ContourOrder::InnerBeforeOuter,
            start: Default::default(),
            finish: Default::default(),
            entry: ProfileEntry::Plunge,
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

fn knife_operation(chain: GeometryRef) -> OperationV5 {
    OperationV5 {
        id: "knife-1".into(),
        name: "Knife".into(),
        enabled: true,
        settings: OperationSettingsV5::DragKnife(v5::DragKnifeSettingsV5 {
            chains: vec![chain],
            assignment: knife_assignment(),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -1.,
            },
            stepdown_mm: Some(1.),
            swivel_depth_mm: Some(0.5),
            corner_threshold_deg: Some(90.),
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: KnifeAlignment {
                initial_heading_deg: Some(180.),
            },
        }),
    }
}

fn job_of(operations: Vec<OperationV5>) -> CamJobV5 {
    let mut job = base_job();
    job.operations = operations;
    job
}

/// The scenario-7 document: job tool `t1` backs the carve's endmill
/// assignment and the profile's milling assignment.
fn resource_job() -> CamJobV5 {
    job_of(vec![
        flat_vcarve_operation(reference_of(GeometryRefKind::FilledComponent, "plate::0")),
        profile_operation(
            reference_of(GeometryRefKind::ClosedContour, "plate-0-outer"),
            HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
        ),
    ])
}

fn library() -> ToolLibrary {
    let endmill = LibraryTool {
        id: "lib-e".into(),
        name: "Library endmill".into(),
        geometry: LibraryGeometry::Endmill(EndmillSpec {
            diameter_mm: 3.,
            cutting_length_mm: 8.,
            plunge_capable: true,
        }),
        ramp_capable: Some(true),
        plunge_capable: Some(true),
        cutting_presets: vec![
            CuttingPreset {
                id: "soft".into(),
                name: "Soft wood".into(),
                material: Some("pine".into()),
                machine: None,
                spindle_rpm: Some(12_000.),
                cutting_feed_mm_min: Some(400.),
                plunge_feed_mm_min: Some(120.),
                max_stepdown_mm: Some(0.8),
                stepover_mm: Some(2.),
            },
            CuttingPreset {
                id: "partial".into(),
                name: "Unset fields copy too".into(),
                material: None,
                machine: None,
                spindle_rpm: Some(9_000.),
                cutting_feed_mm_min: None,
                plunge_feed_mm_min: None,
                max_stepdown_mm: None,
                stepover_mm: None,
            },
        ],
        knife_cutting_presets: vec![],
    };
    let knife = LibraryTool {
        id: "lib-k".into(),
        name: "Library knife".into(),
        geometry: LibraryGeometry::DragKnife(DragKnifeSpec {
            blade_offset_mm: 1.,
            max_cut_depth_mm: 2.,
        }),
        ramp_capable: None,
        plunge_capable: None,
        cutting_presets: vec![],
        knife_cutting_presets: vec![KnifeCuttingPreset {
            id: "card".into(),
            name: "Card stock".into(),
            material: None,
            machine: None,
            cutting_feed_mm_min: Some(200.),
            plunge_feed_mm_min: Some(60.),
            swivel_feed_mm_min: Some(90.),
            max_stepdown_mm: Some(0.5),
        }],
    };
    ToolLibrary {
        schema_version: LIBRARY_SCHEMA_VERSION,
        revision: 4,
        tools: vec![endmill, knife],
    }
}

fn status_of(job: &CamJobV5, operation_id: &str, role: Role) -> ProfileStatus {
    assignment_statuses(job)
        .iter()
        .find(|status| status.operation_id == operation_id && status.role == role)
        .unwrap_or_else(|| panic!("assignment {operation_id}/{role:?} missing"))
        .status
}

/// Scenario 7 (plan section 22.12): two named cutting profiles land on one
/// job tool's different assignments; editing one leaves the other Applied;
/// after the library is gone, Reset restores the copied baseline and the
/// reopened document needs no library to describe itself.
#[test]
fn scenario7_two_named_profiles_on_one_tool_stay_independent() {
    let mut job = resource_job();
    let library = library();
    // Two named profiles onto job tool t1's two assignments.
    job = apply_cutting_profile(
        &job,
        "carve-1",
        Role::Endmill,
        &library,
        "lib-1",
        "lib-e",
        "soft",
    )
    .unwrap()
    .job;
    job = apply_cutting_profile(
        &job,
        "profile-1",
        Role::Milling,
        &library,
        "lib-1",
        "lib-e",
        "partial",
    )
    .unwrap()
    .job;
    assert_eq!(
        status_of(&job, "carve-1", Role::Endmill),
        ProfileStatus::Applied
    );
    assert_eq!(
        status_of(&job, "profile-1", Role::Milling),
        ProfileStatus::Applied
    );
    let OperationSettingsV5::FlatVcarve(carve) = &job.operations[0].settings else {
        panic!("carve settings expected");
    };
    assert_eq!(carve.endmill.spindle_rpm, Some(12_000.));
    assert_eq!(carve.endmill.stepover_mm, Some(2.));
    // The partial preset copied its unset fields as unset values.
    let OperationSettingsV5::Profile(profile) = &job.operations[1].settings else {
        panic!("profile settings expected");
    };
    assert_eq!(profile.assignment.spindle_rpm, Some(9_000.));
    assert_eq!(profile.assignment.cutting_feed_mm_min, None);
    // Sibling assignment of the same job tool is untouched (Custom, no
    // applied provenance, original unset values).
    assert_eq!(
        status_of(&job, "carve-1", Role::Vbit),
        ProfileStatus::Custom
    );
    let baseline_rpm = profile.assignment.spindle_rpm;

    // Edit one assignment directly; only it becomes Modified.
    let OperationSettingsV5::Profile(edited) = &mut job.operations[1].settings else {
        panic!("profile settings expected");
    };
    edited.assignment.cutting_feed_mm_min = Some(999.);
    assert_eq!(
        status_of(&job, "profile-1", Role::Milling),
        ProfileStatus::Modified
    );
    assert_eq!(
        status_of(&job, "carve-1", Role::Endmill),
        ProfileStatus::Applied
    );

    // The library is gone (never passed again); Reset restores the copied
    // baseline, including its unset fields.
    job = reset_assignment(&job, "profile-1", Role::Milling)
        .unwrap()
        .job;
    assert_eq!(
        status_of(&job, "profile-1", Role::Milling),
        ProfileStatus::Applied
    );
    let OperationSettingsV5::Profile(reset) = &job.operations[1].settings else {
        panic!("profile settings expected");
    };
    assert_eq!(reset.assignment.cutting_feed_mm_min, None);
    assert_eq!(reset.assignment.spindle_rpm, baseline_rpm);

    // Reopen: the document round-trips and describes itself with no library
    // or configuration file present.
    let saved = job.to_json().unwrap();
    let reopened = CamJobV5::from_json(&saved).unwrap();
    assert_eq!(assignment_statuses(&reopened), assignment_statuses(&job));
    assert_eq!(
        status_of(&reopened, "carve-1", Role::Endmill),
        ProfileStatus::Applied
    );
    assert_eq!(
        status_of(&reopened, "profile-1", Role::Milling),
        ProfileStatus::Applied
    );
}

/// Reapply loads the explicitly supplied current revision: changed preset
/// values become the new baseline; a removed library record is a located
/// error, never a silent no-op (plan section 22.6).
#[test]
fn reapply_uses_the_supplied_revision_and_fails_on_removed_records() {
    let mut job = resource_job();
    let base_library = library();
    job = apply_cutting_profile(
        &job,
        "carve-1",
        Role::Endmill,
        &base_library,
        "lib-1",
        "lib-e",
        "soft",
    )
    .unwrap()
    .job;
    // The library moved on: same tool and preset ids, new revision/values.
    let mut current = library();
    current.revision = 9;
    current.tools[0].cutting_presets[0].spindle_rpm = Some(14_000.);
    job = reapply_profile(&job, "carve-1", Role::Endmill, &current, "lib-1")
        .unwrap()
        .job;
    let OperationSettingsV5::FlatVcarve(carve) = &job.operations[0].settings else {
        panic!("carve settings expected");
    };
    assert_eq!(carve.endmill.spindle_rpm, Some(14_000.));
    assert_eq!(
        status_of(&job, "carve-1", Role::Endmill),
        ProfileStatus::Applied
    );
    assert_eq!(
        carve
            .endmill
            .applied_profile
            .as_ref()
            .unwrap()
            .revision_at_application,
        9
    );
    // The record disappeared from the current revision: explicit error.
    let mut removed = library();
    removed.tools.clear();
    let error = reapply_profile(&job, "carve-1", Role::Endmill, &removed, "lib-1").unwrap_err();
    assert_eq!(error.code, "RESOURCE_COMMAND");
    assert!(error.message.contains("lib-e"), "{error:?}");
}

/// Applying a *different* tool clears the assignment's applicable cutting
/// fields and baseline; re-applying the same tool refreshes the snapshot and
/// keeps the values for revalidation (plan section 22.6).
#[test]
fn tool_application_clears_only_on_tool_change_and_copies_provenance() {
    let mut job = resource_job();
    let base_library = library();
    job = apply_cutting_profile(
        &job,
        "carve-1",
        Role::Endmill,
        &base_library,
        "lib-1",
        "lib-e",
        "soft",
    )
    .unwrap()
    .job;
    // A different tool without a preset: values and baseline clear.
    let other = LibraryTool {
        id: "lib-e2".into(),
        name: "Second endmill".into(),
        geometry: LibraryGeometry::Endmill(EndmillSpec {
            diameter_mm: 4.,
            cutting_length_mm: 10.,
            plunge_capable: false,
        }),
        ramp_capable: None,
        plunge_capable: None,
        cutting_presets: vec![],
        knife_cutting_presets: vec![],
    };
    let mut with_other = library();
    with_other.tools.push(other);
    job = apply_tool_to_assignment(
        &job,
        "carve-1",
        Role::Endmill,
        &with_other,
        "lib-1",
        "lib-e2",
    )
    .unwrap()
    .job;
    let OperationSettingsV5::FlatVcarve(carve) = &job.operations[0].settings else {
        panic!("carve settings expected");
    };
    assert_eq!(carve.endmill.tool_id, "lib-e2");
    assert_eq!(carve.endmill.spindle_rpm, None);
    assert!(carve.endmill.applied_profile.is_none());
    assert_eq!(
        status_of(&job, "carve-1", Role::Endmill),
        ProfileStatus::Custom
    );
    let snapshot = job.tools.iter().find(|tool| tool.id == "lib-e2").unwrap();
    assert_eq!(
        snapshot.library_origin.as_ref().unwrap().library_id,
        "lib-1"
    );
    assert_eq!(snapshot.capabilities.plunge_capable, Some(false));

    // Supply values, then re-apply the SAME tool: values stay for
    // revalidation while the snapshot refreshes.
    let OperationSettingsV5::FlatVcarve(edited) = &mut job.operations[0].settings else {
        panic!("carve settings expected");
    };
    edited.endmill.spindle_rpm = Some(11_000.);
    job = apply_tool_to_assignment(
        &job,
        "carve-1",
        Role::Endmill,
        &with_other,
        "lib-1",
        "lib-e2",
    )
    .unwrap()
    .job;
    let OperationSettingsV5::FlatVcarve(carve) = &job.operations[0].settings else {
        panic!("carve settings expected");
    };
    assert_eq!(carve.endmill.spindle_rpm, Some(11_000.));

    // Role checking: a knife tool never fits a milling assignment.
    let error = apply_tool_to_assignment(
        &job,
        "carve-1",
        Role::Endmill,
        &base_library,
        "lib-1",
        "lib-k",
    )
    .unwrap_err();
    assert_eq!(error.code, "RESOURCE_COMMAND");
}

/// A schema-2 sequence profile for application tests; clearance matches the
/// setup's 5 mm so application is not a clearance conflict.
fn sequence_profile(job_tools: &[(&str, u32)]) -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "workbench".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: LengthCompensation::MacroManaged,
        path_control: PathControl::ExactPath,
        tools: job_tools
            .iter()
            .map(|(id, n)| SequenceToolMapping {
                tool_id: (*id).into(),
                tool_number: *n,
                length_offset_number: None,
            })
            .collect(),
        spindle_spinup_seconds: 0.5,
        coolant: Coolant::Off,
        m6: m6(),
    }
}

/// Scenario 8 (plan section 22.12): an incomplete applied machine snapshot
/// saves and reopens without T numbers; mapping validation covers exactly the
/// requested executable scope; every readout resolves to the same values.
#[test]
fn scenario8_incomplete_snapshot_saves_and_mappings_follow_the_scope() {
    let mut job = job_of(vec![
        profile_operation(
            reference_of(GeometryRefKind::ClosedContour, "plate-0-outer"),
            HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
        ),
        knife_operation(reference_of(GeometryRefKind::Centerline, "cut-chain-0")),
    ]);
    // An incomplete snapshot: only the origin and one empty mapping row.
    job.machine_configuration = Some(v5::AppliedMachineConfiguration {
        origin: v5::ConfigurationOrigin {
            configuration_id: "half-applied".into(),
            name: "Half applied".into(),
        },
        work_offset: None,
        clearance_z_mm: None,
        decimal_places: None,
        program_start_position_mm: None,
        length_compensation: None,
        path_control: None,
        spindle_spinup_seconds: None,
        coolant: None,
        m6: None,
        tools: vec![v5::AppliedToolMapping {
            job_tool_id: "t1".into(),
            tool_number: None,
            length_offset_number: None,
        }],
    });
    // Saving without T numbers is legal; the document reopens as saved.
    let saved = job.to_json().unwrap();
    let reopened = CamJobV5::from_json(&saved).unwrap();
    assert!(reopened.machine_configuration.is_some());
    let error = resolve_sequence_profile(&reopened, &ReadinessScope::AllEnabled).unwrap_err();
    assert_eq!(error.code, "MACHINE_CONFIGURATION_INCOMPLETE");
    assert!(error.message.contains("work_offset"), "{error:?}");

    // Apply a reusable configuration: the foreign tool row is dropped, this
    // job's t1 row is copied.
    let outcome = apply_machine_configuration(
        &job,
        &sequence_profile(&[("t1", 3), ("foreign-tool", 9)]),
        "Workbench",
    )
    .unwrap();
    job = outcome.job;
    let configuration = job.machine_configuration.as_ref().unwrap();
    assert_eq!(configuration.origin.name, "Workbench");
    assert_eq!(configuration.tools.len(), 1);
    assert_eq!(configuration.tools[0].job_tool_id, "t1");
    assert_eq!(configuration.tools[0].tool_number, Some(3));

    // The prefix scope (profile-1 only) needs exactly t1; the unmapped knife
    // tool is outside it and blocks nothing.
    let prefix = ReadinessScope::ThroughOperation {
        operation_id: "profile-1".into(),
    };
    let profile = resolve_sequence_profile(&job, &prefix).unwrap();
    assert_eq!(profile.tools.len(), 1);
    assert_eq!(profile.tools[0].tool_number, 3);
    // The full scope still needs the knife tool's mapping.
    let error = resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap_err();
    assert_eq!(error.code, "MACHINE_MAPPING_MISSING");
    assert!(error.message.contains("t3"), "{error:?}");
    // Map only the tool the full scope uses.
    job = set_tool_mapping(&job, "t3", Some(5), None).unwrap().job;
    let profile = resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap();
    assert_eq!(profile.tools.len(), 2);

    // Every readout uses the same values: the machine readout's rows equal
    // the resolved profile's mappings.
    let document = inspect_document(&job);
    let rows: Vec<_> = document
        .machine
        .rows
        .iter()
        .map(|row| (row.job_tool_id.as_str(), row.tool_number))
        .collect();
    let resolved: Vec<_> = profile
        .tools
        .iter()
        .map(|mapping| (mapping.tool_id.as_str(), Some(mapping.tool_number)))
        .collect();
    assert_eq!(rows, resolved);

    // Both numbers None removes the row; a duplicate active T blocks output.
    job = set_tool_mapping(&job, "t3", None, None).unwrap().job;
    assert!(
        job.machine_configuration
            .as_ref()
            .unwrap()
            .tools
            .iter()
            .all(|row| row.job_tool_id != "t3")
    );
    job = set_tool_mapping(&job, "t3", Some(3), None).unwrap().job;
    let error = resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap_err();
    assert_eq!(error.code, "MACHINE_MAPPING_CONFLICT");

    // Application refuses to override the job's setup clearance silently.
    let mut conflicting = sequence_profile(&[]);
    conflicting.clearance_z_mm = 6.;
    let error = apply_machine_configuration(&job, &conflicting, "Conflict").unwrap_err();
    assert_eq!(error.code, "MACHINE_CLEARANCE_CONFLICT");

    // Mapping commands address real job tools only.
    let error = set_tool_mapping(&job, "ghost", Some(1), None).unwrap_err();
    assert_eq!(error.code, "MACHINE_COMMAND");
}

/// Inspection DTOs (plan section 22.9): used-by references cover items,
/// qualified geometries and tools; plan inspection resolves heights against
/// the faces the plan itself published and reports stock without requiring
/// any milling tool.
#[test]
fn inspection_reports_used_by_heights_and_stock() {
    let job = job_of(vec![
        flat_vcarve_operation(reference_of(GeometryRefKind::FilledComponent, "plate::0")),
        knife_operation(reference_of(GeometryRefKind::Centerline, "cut-chain-0")),
    ]);
    let document = inspect_document(&job);
    // Machining order and kinds.
    assert_eq!(
        document
            .machining_order
            .iter()
            .map(|op| op.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["flat_vcarve", "drag_knife"]
    );
    // Used-by: the plate item is referenced by both operations; job tool t1
    // backs two assignments, the knife tool one.
    let plate = document
        .used_by
        .artwork_items
        .iter()
        .find(|item| item.item_id == "plate")
        .unwrap();
    assert_eq!(plate.operation_ids, vec!["carve-1", "knife-1"]);
    assert!(document.used_by.geometries.iter().any(|geometry| {
        geometry.local_geometry_id == "cut-chain-0" && geometry.kind == GeometryRefKind::Centerline
    }));
    let t1 = document
        .used_by
        .job_tools
        .iter()
        .find(|tool| tool.tool_id == "t1")
        .unwrap();
    assert!(t1.exists);
    assert_eq!(t1.assignments.len(), 2);
    let t3 = document
        .used_by
        .job_tools
        .iter()
        .find(|tool| tool.tool_id == "t3")
        .unwrap();
    assert_eq!(t3.assignments.len(), 1);
    assert_eq!(document.assignments.len(), 3);

    // Plan-level inspection over a generated face-then-profile plan: the
    // profile's top resolves against the face the plan published.
    let face = OperationV5 {
        id: "face-1".into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettingsV5::Face(v5::FaceSettingsV5 {
            area: FaceArea::EntireStock,
            margins: FaceMargins::default(),
            entry_overrun_mm: Some(2.),
            exit_overrun_mm: Some(2.),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -1.5,
            },
            stepdown_mm: Some(1.),
            stepover_mm: Some(2.),
            pass_angle_deg: None,
            pattern: Default::default(),
            assignment: milling("t1"),
        }),
    };
    let planned_job = job_of(vec![
        face,
        profile_operation(
            reference_of(GeometryRefKind::ClosedContour, "plate-0-outer"),
            HeightRef {
                reference: HeightReference::FaceResult {
                    operation_id: "face-1".into(),
                },
                offset_mm: 0.,
            },
        ),
    ]);
    let plan = OperationPlanV5::plan_job_v5(
        &planned_job,
        &ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    check_plan_v5(&plan).unwrap();
    let inspection = inspect_plan(&plan).unwrap();
    let face_row = inspection
        .operations
        .iter()
        .find(|op| op.operation_id == "face-1")
        .unwrap();
    assert_eq!(face_row.heights.unwrap().bottom_z_mm, Some(-1.5));
    assert_eq!(face_row.face.unwrap().z_mm, -1.5);
    let profile_row = inspection
        .operations
        .iter()
        .find(|op| op.operation_id == "profile-1")
        .unwrap();
    let heights = profile_row.heights.unwrap();
    assert_eq!(heights.top_z_mm, -1.5);
    assert_eq!(heights.bottom_z_mm, Some(-3.5));
    // Stock readouts exist without any milling tool requirement.
    assert_eq!(inspection.stock.thickness_mm, Some(8.));
    assert!(inspection.stock.xy.is_some());
    assert!(inspection.stock.initial_stock_id.is_some());
    assert!(!inspection.stages.is_empty());
    assert_eq!(
        inspection
            .stages
            .iter()
            .map(|stage| stage.motion_count)
            .sum::<usize>(),
        plan.motions.len()
    );
}

/// Collection plans export and replay through the same post pipeline (H4 +
/// F3b): the applied machine configuration supplies the profile, the emitted
/// bytes decode, and the independent knife replay stays within budget — with
/// physical stock reported for a knife-only job.
#[test]
fn knife_evidence_replays_collection_output_within_budget() {
    let mut job = job_of(vec![knife_operation(reference_of(
        GeometryRefKind::Centerline,
        "cut-chain-0",
    ))]);
    job.tools.retain(|tool| tool.id == "t3"); // knife-only: no milling tool
    job = apply_machine_configuration(&job, &sequence_profile(&[("t3", 7)]), "Kiosk")
        .unwrap()
        .job;
    let profile = resolve_sequence_profile(&job, &ReadinessScope::AllEnabled).unwrap();
    let plan =
        OperationPlanV5::plan_job_v5(&job, &ReadinessScope::AllEnabled, &PlanLimits::default())
            .unwrap();
    let trusted = TrustedPlanV5::from_generated(plan);
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    assert!(export.program.gcode.contains("G61"));
    let decoded = prepared
        .decode_program(
            trusted.plan(),
            prepared.output_decimal_places,
            &export.program.gcode,
        )
        .unwrap();
    let evidence = build_evidence(
        trusted.plan(),
        &prepared,
        &decoded,
        &export.report.program_sha256,
        2_048,
    )
    .unwrap();
    assert_eq!(evidence.status, EvidenceStatus::Within);
    assert_eq!(evidence.program_sha256, export.report.program_sha256);
    assert_eq!(
        evidence.execution_fingerprint,
        trusted.plan().execution_fingerprint
    );
    // Stock readouts exist for the knife-only job; no milling tool needed.
    let inspection = inspect_plan(trusted.plan()).unwrap();
    assert_eq!(inspection.stock.thickness_mm, Some(8.));
}
