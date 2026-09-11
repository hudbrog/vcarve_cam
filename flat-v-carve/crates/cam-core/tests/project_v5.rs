//! H1 schema-5 collection model: artwork/ref/provenance shapes, the
//! integrity-vs-readiness validation split, migration from the frozen
//! schema-4 DTO, and scope-local blocking of dangling references (plan
//! sections 22.2, 22.3, 22.5 and 22.11).
use cam_core::{
    contours::ContourCatalogue,
    geometry::Point,
    job::{MachineProfile, PlanningTolerances, SourceSnapshot},
    model::VBitSpec,
    project::v5::{
        self, AppliedMachineConfiguration, AppliedProfile, AppliedToolMapping, ArtworkContent,
        ArtworkItemId, CamJobV5, ConfigurationOrigin, CuttingBaseline, GeometryRef,
        GeometryRefKind, LibraryOrigin, MIGRATED_ARTWORK_ITEM_ID, ReadinessScope,
        artwork::item_catalogue,
        migrate::{migrate_json, migrate_v4},
        references::{inspect_references, planning_readiness},
    },
    project::{
        CAM_JOB_SCHEMA_VERSION, CamJob, ContourAnchor, ContourSide, CutDirection, DragKnifeSpec,
        EndmillGeometry, FaceArea, FacePattern, FaceSettings, FlatVcarveMode, FlatVcarveSettings,
        HeightRef, HeightReference, JobTool, KnifeAlignment, KnifeAssignment, MillingAssignment,
        Operation, OperationSettings, ProfileContour, ProfileSettings, RectXY, SetupSettings,
        StockSetup, TabPlacement, TabSettings, ToolCapabilities, ToolGeometry,
    },
    svg::{ImportMode, ImportOptions, Placement},
};

/// One filled region plus one stroked centerline: with the centerline import
/// mode this yields exactly one component (`plate::0`) with one outer
/// contour, and one open chain (`cut-chain-0`).
const ARTWORK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/></svg>"##;
/// The same artwork with a sub-fingerprint-grid edit (0.001 mm, below the
/// 0.01 mm contour fingerprint quantization).
const ARTWORK_EDITED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4.001"/></svg>"##;

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");

fn milling(tool_id: &str) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool_id.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: None,
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
    }
}

fn knife_assignment() -> KnifeAssignment {
    KnifeAssignment {
        tool_id: "t3".into(),
        cutting_feed_mm_min: Some(150.),
        plunge_feed_mm_min: Some(50.),
        swivel_feed_mm_min: Some(75.),
        max_stepdown_mm: Some(1.),
    }
}

fn tools() -> Vec<JobTool> {
    vec![
        JobTool {
            id: "t1".into(),
            name: "3mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 3.,
                cutting_length_mm: 8.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: Some(true),
            },
        },
        JobTool {
            id: "t2".into(),
            name: "90 degree V-bit".into(),
            geometry: Some(ToolGeometry::Vbit(VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: 0.2,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 3.,
            })),
            capabilities: ToolCapabilities::default(),
        },
        JobTool {
            id: "t3".into(),
            name: "drag knife".into(),
            geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 1.5,
                max_cut_depth_mm: 2.,
            })),
            capabilities: ToolCapabilities::default(),
        },
    ]
}

fn placement() -> Placement {
    Placement {
        origin_mm: Point::new(3., 4.),
        scale: 2.,
        rotation_deg: 90.,
    }
}

fn v4_job(svg: &str, operations: Vec<Operation>) -> CamJob {
    CamJob {
        schema_version: CAM_JOB_SCHEMA_VERSION,
        name: "collection source".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            placement: placement(),
            mode: ImportMode::Centerline,
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: None,
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        tools: tools(),
        operations,
        tolerances: PlanningTolerances::default(),
        legacy_machine_profile: Some(MachineProfile {
            id: "legacy-machine".into(),
            work_offset: Some("G54".into()),
            clearance_z_mm: Some(5.),
            endmill_tool_number: Some(1),
            vbit_tool_number: Some(2),
            m6_contract: None,
        }),
    }
}

fn face_operation() -> Operation {
    Operation {
        id: "face-1".into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::Rectangle {
                rect: RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 100.,
                    length_mm: 60.,
                },
            },
            margins: Default::default(),
            entry_overrun_mm: Some(2.),
            exit_overrun_mm: Some(2.),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -0.5,
            },
            stepdown_mm: Some(0.5),
            stepover_mm: Some(2.),
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: milling("t1"),
        }),
    }
}

/// The complete single-source job: every operation kind with real catalogue
/// selections and anchors. Contour/chain IDs and fingerprints come from the
/// catalogue itself, exactly like a user's picks.
fn full_v4_job() -> CamJob {
    let bare = v4_job(ARTWORK, vec![]);
    let catalogue = ContourCatalogue::build(&bare).unwrap();
    let contour = &catalogue.contours[0];
    let chain = &catalogue.open_chains[0];
    let anchor = |fraction: f64| ContourAnchor {
        contour_id: contour.id.clone(),
        source_geometry_fingerprint: contour.source_fingerprint.clone(),
        fraction_along_source_contour: fraction,
    };
    let operations = vec![
        face_operation(),
        Operation {
            id: "carve-1".into(),
            name: "Carve the plate".into(),
            enabled: true,
            settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
                component_ids: vec!["plate::0".into()],
                mode: FlatVcarveMode::EndmillOnly,
                endmill: milling("t1"),
                vbit: milling("t2"),
                top: Default::default(),
                max_depth_mm: Some(1.5),
                wall_allowance_mm: None,
                max_floor_ridge_mm: None,
                max_detail_residual_mm: None,
                rough: None,
                finish: None,
            }),
        },
        Operation {
            id: "profile-1".into(),
            name: "Cut the plate free".into(),
            enabled: true,
            settings: OperationSettings::Profile(ProfileSettings {
                contours: vec![ProfileContour {
                    contour_id: contour.id.clone(),
                    side: ContourSide::Outside,
                    traversal: None,
                }],
                assignment: milling("t1"),
                top: HeightRef {
                    reference: HeightReference::FaceResult {
                        operation_id: "face-1".into(),
                    },
                    offset_mm: 0.,
                },
                bottom: HeightRef {
                    reference: HeightReference::OperationTop,
                    offset_mm: -2.,
                },
                stepdown_mm: None,
                through_cut_allowance_mm: None,
                direction: Some(CutDirection::Climb),
                order: Default::default(),
                start: cam_core::project::StartSelection::Anchor(Box::new(anchor(0.25))),
                finish: Default::default(),
                entry: Default::default(),
                lead_in: Default::default(),
                lead_out: Default::default(),
                tabs: Some(TabSettings {
                    height_mm: Some(2.),
                    width_mm: Some(5.),
                    shape: Default::default(),
                    placement: TabPlacement::Manual {
                        anchors: vec![anchor(0.6)],
                    },
                }),
            }),
        },
        Operation {
            id: "knife-1".into(),
            name: "Score the cut line".into(),
            enabled: true,
            settings: OperationSettings::DragKnife(cam_core::project::DragKnifeSettings {
                chains: vec![chain.id.clone()],
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
                    initial_heading_deg: Some(90.),
                },
            }),
        },
    ];
    v4_job(ARTWORK, operations)
}

fn issue_codes(issues: &[cam_core::operations::LocatedDiagnostic]) -> Vec<String> {
    issues.iter().map(|i| i.code.clone()).collect()
}

fn readiness_of(
    job: &CamJobV5,
    scope: &ReadinessScope,
) -> std::collections::BTreeMap<String, bool> {
    let readiness = planning_readiness(job, scope).unwrap();
    readiness
        .operations
        .iter()
        .map(|o| (o.operation_id.clone(), o.ready))
        .collect()
}

#[test]
fn source_free_face_job_migrates_to_an_empty_collection() {
    let mut job = full_v4_job();
    job.source = None;
    // A source-free job cannot carry geometry selections; strip them so the
    // document is a plain face-only job.
    job.operations.retain(|op| op.id == "face-1");
    if let OperationSettings::Face(settings) = &mut job.operations[0].settings {
        settings.stepover_mm = None;
    }
    let migrated = migrate_v4(&job).unwrap();
    assert!(migrated.artwork.is_empty(), "face-only jobs migrate to []");
    assert_eq!(migrated.operations.len(), 1);
    assert_eq!(migrated.setup, job.setup);
    match &migrated.operations[0].settings {
        v5::OperationSettingsV5::Face(settings) => {
            assert_eq!(settings.stepover_mm, None, "missing values stay missing");
            assert_eq!(settings.assignment.cutting_feed_mm_min, Some(300.));
        }
        other => panic!("unexpected settings {other:?}"),
    }
    let json = migrated.to_json().unwrap();
    assert_eq!(CamJobV5::from_json(&json).unwrap(), migrated);
    assert!(
        !json.contains("\"source\"") && !json.contains("\"import\""),
        "no top-level source/import fallback authorities exist"
    );
    let readiness = planning_readiness(&migrated, &ReadinessScope::AllEnabled).unwrap();
    assert!(
        readiness.ready(),
        "a face-only collection job is reference-ready"
    );
}

#[test]
fn single_source_job_migrates_with_placements_selections_and_anchors() {
    let job = full_v4_job();
    let catalogue_v4 = ContourCatalogue::build(&job).unwrap();
    let migrated = migrate_v4(&job).unwrap();

    assert_eq!(migrated.artwork.len(), 1);
    let item = &migrated.artwork[0];
    assert_eq!(item.id, ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()));
    assert_eq!(item.name, "art.svg");
    assert_eq!(
        item.placement,
        placement(),
        "the placement formula survives"
    );
    assert_eq!(item.import_settings.mode, ImportMode::Centerline);
    assert_eq!(item.import_settings.geometry_tolerance_mm, 0.001);
    let revision = item.source_revision().unwrap();

    // Identical placed geometry: the item catalogue rebuilt through the
    // reassembled temporary import matches the schema-4 catalogue exactly.
    let catalogue_v5 = item_catalogue(item).unwrap();
    for (a, b) in catalogue_v4.contours.iter().zip(&catalogue_v5.contours) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.source_fingerprint, b.source_fingerprint);
        assert!((a.perimeter_mm - b.perimeter_mm).abs() < 1e-9);
    }
    for (a, b) in catalogue_v4
        .open_chains
        .iter()
        .zip(&catalogue_v5.open_chains)
    {
        assert_eq!(a.id, b.id);
        assert_eq!(a.vertices, b.vertices);
    }

    assert_eq!(
        migrated
            .operations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        vec!["face-1", "carve-1", "profile-1", "knife-1"],
        "operation order and IDs are preserved"
    );
    let expected_ref = |kind, local: &str| GeometryRef {
        artwork_item_id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        kind,
        local_geometry_id: local.into(),
        source_revision: revision.clone(),
    };
    match &migrated.operations[1].settings {
        v5::OperationSettingsV5::FlatVcarve(settings) => {
            assert_eq!(
                settings.components,
                vec![expected_ref(GeometryRefKind::FilledComponent, "plate::0")]
            );
            assert_eq!(settings.mode, FlatVcarveMode::EndmillOnly);
            assert_eq!(settings.endmill.cutting_feed_mm_min, Some(300.));
        }
        other => panic!("unexpected settings {other:?}"),
    }
    let contour_id = catalogue_v4.contours[0].id.clone();
    match &migrated.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => {
            assert_eq!(
                settings.contours[0].geometry,
                expected_ref(GeometryRefKind::ClosedContour, &contour_id)
            );
            assert_eq!(settings.contours[0].side, ContourSide::Outside);
            match &settings.start {
                v5::StartSelectionV5::Anchor(anchor) => {
                    assert_eq!(
                        anchor.geometry,
                        expected_ref(GeometryRefKind::ClosedContour, &contour_id)
                    );
                    assert_eq!(anchor.fraction_along_source_contour, 0.25);
                    assert_eq!(
                        anchor.source_geometry_fingerprint,
                        catalogue_v4.contours[0].source_fingerprint
                    );
                }
                other => panic!("unexpected start {other:?}"),
            }
            match &settings.tabs.as_ref().unwrap().placement {
                v5::TabPlacementV5::Manual { anchors } => {
                    assert_eq!(anchors[0].fraction_along_source_contour, 0.6);
                }
                other => panic!("unexpected tab placement {other:?}"),
            }
        }
        other => panic!("unexpected settings {other:?}"),
    }
    let chain_id = catalogue_v4.open_chains[0].id.clone();
    match &migrated.operations[3].settings {
        v5::OperationSettingsV5::DragKnife(settings) => {
            assert_eq!(
                settings.chains,
                vec![expected_ref(GeometryRefKind::Centerline, &chain_id)]
            );
            assert_eq!(settings.alignment.initial_heading_deg, Some(90.));
        }
        other => panic!("unexpected settings {other:?}"),
    }

    // Everything resolves: no inspection issues and a fully ready scope.
    assert!(inspect_references(&migrated).unwrap().issues.is_empty());
    assert!(
        planning_readiness(&migrated, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );

    let json = migrated.to_json().unwrap();
    assert_eq!(CamJobV5::from_json(&json).unwrap(), migrated);
}

#[test]
fn migration_fabricates_nothing() {
    let job = full_v4_job();
    let migrated = migrate_v4(&job).unwrap();
    assert_eq!(migrated.machine_configuration, None);
    assert_eq!(migrated.legacy_machine_profile, job.legacy_machine_profile);
    for tool in &migrated.tools {
        assert_eq!(tool.library_origin, None);
    }
    let assignments: Vec<_> = migrated
        .operations
        .iter()
        .flat_map(|op| match &op.settings {
            v5::OperationSettingsV5::FlatVcarve(s) => {
                vec![&s.endmill.applied_profile, &s.vbit.applied_profile]
            }
            v5::OperationSettingsV5::Face(s) => vec![&s.assignment.applied_profile],
            v5::OperationSettingsV5::Profile(s) => vec![&s.assignment.applied_profile],
            v5::OperationSettingsV5::DragKnife(s) => vec![&s.assignment.applied_profile],
        })
        .collect();
    assert!(assignments.iter().all(|p| p.is_none()));
}

#[test]
fn unknown_local_ids_migrate_to_typed_dangling_references() {
    let mut job = full_v4_job();
    if let OperationSettings::Profile(settings) = &mut job.operations[2].settings {
        settings.contours[0].contour_id = "ghost-contour".into();
    }
    if let OperationSettings::FlatVcarve(settings) = &mut job.operations[1].settings {
        settings.component_ids = vec!["ghost::9".into()];
    }
    let migrated = migrate_v4(&job).unwrap();
    match &migrated.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => {
            assert_eq!(
                settings.contours[0].geometry.local_geometry_id,
                "ghost-contour"
            );
        }
        other => panic!("unexpected settings {other:?}"),
    }
    let inspection = inspect_references(&migrated).unwrap();
    assert!(issue_codes(&inspection.issues).contains(&"GEOMETRY_REFERENCE".into()));
    let readiness = readiness_of(&migrated, &ReadinessScope::AllEnabled);
    assert_eq!(
        readiness,
        vec![
            ("carve-1".to_string(), false),
            ("face-1".to_string(), true),
            ("knife-1".to_string(), true),
            ("profile-1".to_string(), false)
        ]
        .into_iter()
        .collect(),
        "unknown local IDs block exactly the operations that select them"
    );
}

#[test]
fn dangling_references_round_trip_and_block_only_the_affected_scope() {
    let mut migrated = migrate_v4(&full_v4_job()).unwrap();
    // Deleting the artwork item keeps its references dangling but saveable.
    migrated.artwork.clear();
    let json = migrated.to_json().unwrap();
    let mut reopened = CamJobV5::from_json(&json).unwrap();
    assert_eq!(reopened, migrated, "the broken draft round-trips");

    let inspection = inspect_references(&reopened).unwrap();
    let owner_codes: Vec<(Option<&str>, String)> = inspection
        .issues
        .iter()
        .map(|i| (i.operation_id.as_deref(), i.code.clone()))
        .collect();
    for owner in ["carve-1", "profile-1", "knife-1"] {
        assert!(
            owner_codes.contains(&(Some(owner), "ARTWORK_REFERENCE".into())),
            "{owner} should carry an artwork reference issue, got {owner_codes:?}"
        );
    }
    assert!(
        !owner_codes
            .iter()
            .any(|(owner, _)| *owner == Some("face-1")),
        "the face operation references no artwork"
    );

    assert_eq!(
        readiness_of(&reopened, &ReadinessScope::AllEnabled),
        vec![
            ("carve-1".to_string(), false),
            ("face-1".to_string(), true),
            ("knife-1".to_string(), false),
            ("profile-1".to_string(), false)
        ]
        .into_iter()
        .collect()
    );
    // A prefix ending before any broken operation stays plannable.
    assert_eq!(
        readiness_of(
            &reopened,
            &ReadinessScope::ThroughOperation {
                operation_id: "face-1".into()
            }
        ),
        vec![("face-1".to_string(), true)].into_iter().collect()
    );
    // Disabling the broken operations unblocks the enabled scope.
    for op in reopened.operations.iter_mut() {
        if op.id != "face-1" {
            op.enabled = false;
        }
    }
    assert!(
        planning_readiness(&reopened, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
    // Unknown prefix targets are located errors.
    let err = planning_readiness(
        &reopened,
        &ReadinessScope::ThroughOperation {
            operation_id: "ghost".into(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "READINESS_SCOPE");
}

#[test]
fn forward_face_references_are_saveable_and_block_only_their_operation() {
    let mut migrated = migrate_v4(&full_v4_job()).unwrap();
    // Move the face after the profile: its FaceResult reference becomes
    // forward while staying structurally valid and saveable.
    let face = migrated.operations.remove(0);
    migrated.operations.insert(2, face);
    migrated.to_json().unwrap();
    let inspection = inspect_references(&migrated).unwrap();
    assert!(issue_codes(&inspection.issues).contains(&"HEIGHT_REFERENCE_FORWARD".into()));
    assert_eq!(
        readiness_of(&migrated, &ReadinessScope::AllEnabled),
        vec![
            ("carve-1".to_string(), true),
            ("face-1".to_string(), true),
            ("knife-1".to_string(), true),
            ("profile-1".to_string(), false)
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn source_revision_changes_invalidate_references_until_reattached() {
    let migrated = migrate_v4(&full_v4_job()).unwrap();

    // Any content change invalidates, including sub-fingerprint-grid edits.
    let mut edited = migrated.clone();
    edited.artwork[0].content = ArtworkContent::Svg(SourceSnapshot {
        filename: "art.svg".into(),
        svg: ARTWORK_EDITED.into(),
    });
    let inspection = inspect_references(&edited).unwrap();
    assert!(issue_codes(&inspection.issues).contains(&"ARTWORK_REVISION_MISMATCH".into()));
    assert!(
        !planning_readiness(&edited, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );

    // Renaming and moving the item never invalidate: the revision excludes
    // display name and placement.
    let mut moved = migrated.clone();
    moved.artwork[0].name = "Renamed artwork".into();
    moved.artwork[0].placement.origin_mm = Point::new(10., 10.);
    assert!(inspect_references(&moved).unwrap().issues.is_empty());
    assert!(
        planning_readiness(&moved, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
}

#[test]
fn identical_byte_items_stay_distinct() {
    let mut job = migrate_v4(&full_v4_job()).unwrap();
    let first = job.artwork[0].clone();
    let mut second = first.clone();
    second.id = ArtworkItemId("lettering".into());
    second.name = "Lettering".into();
    second.placement.origin_mm = Point::new(-5., 0.);
    job.artwork = vec![first.clone(), second];
    // Point the carve and profile at item two, keep the knife on item one.
    let first_revision = job.artwork[0].source_revision().unwrap();
    let second_revision = job.artwork[1].source_revision().unwrap();
    assert_eq!(first_revision, second_revision, "identical bytes/settings");
    let to_second = |reference: &GeometryRef| GeometryRef {
        artwork_item_id: ArtworkItemId("lettering".into()),
        source_revision: second_revision.clone(),
        ..reference.clone()
    };
    let to_first = |reference: &GeometryRef| GeometryRef {
        artwork_item_id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        source_revision: first_revision.clone(),
        ..reference.clone()
    };
    if let v5::OperationSettingsV5::FlatVcarve(settings) = &mut job.operations[1].settings {
        settings.components = settings.components.iter().map(&to_second).collect();
    }
    if let v5::OperationSettingsV5::Profile(settings) = &mut job.operations[2].settings {
        settings.contours[0].geometry = to_second(&settings.contours[0].geometry);
        if let v5::StartSelectionV5::Anchor(anchor) = &mut settings.start {
            anchor.geometry = to_second(&anchor.geometry);
        }
        if let Some(tabs) = &mut settings.tabs
            && let v5::TabPlacementV5::Manual { anchors } = &mut tabs.placement
        {
            anchors[0].geometry = to_second(&anchors[0].geometry);
        }
    }
    if let v5::OperationSettingsV5::DragKnife(settings) = &mut job.operations[3].settings {
        settings.chains = settings.chains.iter().map(&to_first).collect();
    }
    let inspection = inspect_references(&job).unwrap();
    assert!(inspection.issues.is_empty(), "{:?}", inspection.issues);
    assert!(
        planning_readiness(&job, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
    let json = job.to_json().unwrap();
    assert_eq!(CamJobV5::from_json(&json).unwrap(), job);
}

#[test]
fn provenance_and_machine_configuration_round_trip() {
    let mut job = migrate_v4(&full_v4_job()).unwrap();
    job.tools[0].library_origin = Some(LibraryOrigin {
        library_id: "library-1".into(),
        tool_id: "lib-tool-7".into(),
        copied_revision: 3,
        name_at_copy: "3mm endmill".into(),
    });
    if let v5::OperationSettingsV5::Profile(settings) = &mut job.operations[2].settings {
        settings.assignment.applied_profile = Some(AppliedProfile {
            library_id: "library-1".into(),
            library_tool_id: "lib-tool-7".into(),
            preset_id: "preset-2".into(),
            revision_at_application: 5,
            name_at_application: "Birch 3mm".into(),
            baseline: CuttingBaseline::Milling {
                spindle_rpm: Some(9_000.),
                cutting_feed_mm_min: Some(200.),
                plunge_feed_mm_min: None,
                max_stepdown_mm: Some(0.8),
                stepover_mm: None,
            },
        });
    }
    if let v5::OperationSettingsV5::DragKnife(settings) = &mut job.operations[3].settings {
        settings.assignment.applied_profile = Some(AppliedProfile {
            library_id: "library-1".into(),
            library_tool_id: "lib-knife-1".into(),
            preset_id: "preset-k".into(),
            revision_at_application: 2,
            name_at_application: "Paper score".into(),
            baseline: CuttingBaseline::Knife {
                cutting_feed_mm_min: Some(120.),
                plunge_feed_mm_min: None,
                swivel_feed_mm_min: Some(60.),
                max_stepdown_mm: None,
            },
        });
    }
    job.machine_configuration = Some(AppliedMachineConfiguration {
        origin: ConfigurationOrigin {
            configuration_id: "machine-1".into(),
            name: "Shop router".into(),
        },
        work_offset: Some("G55".into()),
        clearance_z_mm: None,
        decimal_places: None,
        program_start_position_mm: None,
        length_compensation: None,
        path_control: None,
        spindle_spinup_seconds: None,
        coolant: None,
        m6: None,
        tools: vec![AppliedToolMapping {
            job_tool_id: "t1".into(),
            tool_number: Some(1),
            length_offset_number: None,
        }],
    });
    let json = job.to_json().unwrap();
    let reopened = CamJobV5::from_json(&json).unwrap();
    assert_eq!(reopened, job);
    // The baseline keeps unset fields distinct from supplied zeros: the
    // round-trip preserves `None` vs `Some(_)` per field.
    if let v5::OperationSettingsV5::Profile(settings) = &reopened.operations[2].settings {
        let CuttingBaseline::Milling {
            plunge_feed_mm_min, ..
        } = &settings
            .assignment
            .applied_profile
            .as_ref()
            .unwrap()
            .baseline
        else {
            panic!("milling baseline");
        };
        assert_eq!(plunge_feed_mm_min, &None);
    }
}

#[test]
fn machine_mapping_dangling_rows_are_inspection_issues() {
    let mut job = migrate_v4(&full_v4_job()).unwrap();
    job.machine_configuration = Some(AppliedMachineConfiguration {
        origin: ConfigurationOrigin {
            configuration_id: "machine-1".into(),
            name: "Shop router".into(),
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
        tools: vec![AppliedToolMapping {
            job_tool_id: "deleted-tool".into(),
            tool_number: Some(4),
            length_offset_number: None,
        }],
    });
    job.to_json().unwrap();
    let inspection = inspect_references(&job).unwrap();
    assert!(issue_codes(&inspection.issues).contains(&"MACHINE_MAPPING_REFERENCE".into()));
    // Readiness is about machining references; the dangling mapping row does
    // not block planning scopes (it blocks output preparation, which is H4).
    assert!(
        planning_readiness(&job, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
}

#[test]
fn structural_integrity_still_rejects_malformed_documents() {
    let base = migrate_v4(&full_v4_job()).unwrap();

    let mut duplicate_item = base.clone();
    duplicate_item
        .artwork
        .push(duplicate_item.artwork[0].clone());
    assert_eq!(
        duplicate_item.validate_structure().unwrap_err().code,
        "PROJECT_ARTWORK_ID"
    );

    let mut duplicate_operation = base.clone();
    duplicate_operation
        .operations
        .push(duplicate_operation.operations[0].clone());
    assert_eq!(
        duplicate_operation.validate_structure().unwrap_err().code,
        "PROJECT_OPERATION_ID"
    );

    let mut bad_fraction = base.clone();
    if let v5::OperationSettingsV5::Profile(settings) = &mut bad_fraction.operations[2].settings
        && let v5::StartSelectionV5::Anchor(anchor) = &mut settings.start
    {
        anchor.fraction_along_source_contour = 1.5;
    }
    assert_eq!(
        bad_fraction.validate_structure().unwrap_err().code,
        "PROJECT_PARAMETER"
    );

    let mut oversized = base.clone();
    oversized.artwork[0].content = ArtworkContent::Svg(SourceSnapshot {
        filename: "big.svg".into(),
        svg: "x".repeat(cam_core::svg::MAX_SVG_BYTES + 1),
    });
    assert_eq!(
        oversized.validate_structure().unwrap_err().code,
        "PROJECT_RESOURCE_LIMIT"
    );

    // Strict parsing: unknown fields and stale top-level authorities refuse.
    let json = base.to_json().unwrap();
    let polluted = json.replace(
        "\"schema_version\": 5",
        "\"schema_version\": 5,\n  \"source\": {}",
    );
    assert_eq!(
        CamJobV5::from_json(&polluted).unwrap_err().code,
        "PROJECT_JSON"
    );
    // The frozen schema-4 parser refuses schema-5 documents instead of
    // flattening them: the collection shape has no schema-4 spelling (no
    // top-level import), so strict parsing rejects it outright.
    assert!(CamJob::from_json(&json).is_err());
}

#[test]
fn migrate_json_routes_every_supported_schema() {
    let v4 = full_v4_job();
    let v5 = migrate_v4(&v4).unwrap();

    // Schema 5 parses directly.
    assert_eq!(migrate_json(&v5.to_json().unwrap()).unwrap(), v5);
    // Schema 4 migrates.
    assert_eq!(migrate_json(&v4.to_json().unwrap()).unwrap(), v5);
    // Legacy schema 2 chains through the frozen compatibility steps.
    let legacy = migrate_json(M3_RECTANGLE).unwrap();
    assert_eq!(legacy.schema_version, v5::CAM_JOB_V5_SCHEMA_VERSION);
    assert_eq!(legacy.artwork.len(), 1);
    assert_eq!(
        legacy.artwork[0].id,
        ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into())
    );
    assert_eq!(legacy.operations.len(), 1);
    match &legacy.operations[0].settings {
        v5::OperationSettingsV5::FlatVcarve(settings) => {
            assert_eq!(settings.mode, FlatVcarveMode::EndmillOnly);
            assert_eq!(
                settings.max_depth_mm,
                Some(2.),
                "cutting values are preserved"
            );
        }
        other => panic!("unexpected settings {other:?}"),
    }
    assert!(
        inspect_references(&legacy).unwrap().issues.is_empty(),
        "the migrated legacy selection resolves against its snapshot"
    );
    // Unknown schema versions stay rejected.
    let unknown = v5
        .to_json()
        .unwrap()
        .replace("\"schema_version\": 5", "\"schema_version\": 99");
    assert_eq!(
        migrate_json(&unknown).unwrap_err().code,
        "CAM_JOB_SCHEMA_VERSION"
    );
}

#[test]
fn unresolvable_items_report_import_failures_without_blocking_saves() {
    let mut job = migrate_v4(&full_v4_job()).unwrap();
    job.artwork[0].content = ArtworkContent::Svg(SourceSnapshot {
        filename: "art.svg".into(),
        svg: "not xml at all".into(),
    });
    job.to_json().unwrap();
    let inspection = inspect_references(&job).unwrap();
    assert!(issue_codes(&inspection.issues).contains(&"ARTWORK_IMPORT_FAILED".into()));
    assert!(
        !planning_readiness(&job, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
}
