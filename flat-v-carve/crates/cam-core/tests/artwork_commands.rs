//! H2 artwork catalogue and document commands (plan sections 22.3, 22.4,
//! 22.5 and the H2 slice of 22.11): per-item import/placement through the
//! shared resolver, owner-qualified catalogues, atomic
//! import/replace/duplicate/remove, explicit assignment and fit-stock
//! commands. Same-name/identical-byte sources never collide; moving one
//! source leaves the others fixed; replacement/deletion retains actionable
//! references until explicitly reassigned or reattached.
use cam_core::{
    contours::ContourCatalogue,
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    model::VBitSpec,
    project::{
        CAM_JOB_SCHEMA_VERSION, CamJob, ContourAnchor, ContourSide, CutDirection, DragKnifeSpec,
        EndmillGeometry, FaceArea, FacePattern, FaceSettings, FlatVcarveMode, FlatVcarveSettings,
        HeightRef, HeightReference, JobTool, KnifeAlignment, KnifeAssignment, MillingAssignment,
        Operation, OperationSettings, ProfileContour, ProfileSettings, RectXY, SetupSettings,
        StockSetup, TabPlacement, TabSettings, ToolCapabilities, ToolGeometry,
        v5::{
            self, ArtworkItemId, GeometryRefKind, MIGRATED_ARTWORK_ITEM_ID, ReadinessScope,
            artwork::{self, inspect_artwork, parse_wire_id},
            commands::{
                AddArtworkOutcome, AnchorTarget, ArtworkInput, CommandOutcome, FitStockMargins,
                FitStockRequest, ProfileContourPick, add_artwork, apply_stock_rectangle,
                duplicate_artwork, place_artwork, propose_fit_stock, reattach_anchor,
                remove_artwork, rename_artwork, reorder_artwork, replace_artwork,
                set_chain_selection, set_component_selection, set_contour_selection,
            },
            migrate::migrate_v4,
            references::planning_readiness,
        },
    },
    svg::{ImportMode, ImportOptions, Placement},
};

/// One filled region plus one stroked centerline (see project_v5.rs).
const ARTWORK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/></svg>"##;
/// The same artwork with a sub-fingerprint-grid edit (0.001 mm).
const ARTWORK_EDITED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4.001"/></svg>"##;
/// A second, independent source: one filled word outline.
const LETTERING: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 20 10"><rect id="word" x="1" y="1" width="8" height="4" fill="#fff"/></svg>"##;

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

/// The single-source v4 fixture (identical to project_v5.rs's) migrated to
/// the collection model: face, carve, profile (anchor start + manual tab)
/// and knife, all selecting migrated artwork-1 with catalogue-derived
/// anchors exactly like a user's picks.
fn migrated_job() -> v5::CamJobV5 {
    let v4 = |operations: Vec<Operation>| CamJob {
        schema_version: CAM_JOB_SCHEMA_VERSION,
        name: "collection source".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: ARTWORK.into(),
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
        legacy_machine_profile: None,
    };
    let bare = v4(vec![]);
    let catalogue = ContourCatalogue::build(&bare).unwrap();
    let contour = &catalogue.contours[0];
    let chain = &catalogue.open_chains[0];
    let anchor = |fraction: f64| ContourAnchor {
        contour_id: contour.id.clone(),
        source_geometry_fingerprint: contour.source_fingerprint.clone(),
        fraction_along_source_contour: fraction,
    };
    let operations = vec![
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
        },
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
                assignment: KnifeAssignment {
                    tool_id: "t3".into(),
                    cutting_feed_mm_min: Some(150.),
                    plunge_feed_mm_min: Some(50.),
                    swivel_feed_mm_min: Some(75.),
                    max_stepdown_mm: Some(1.),
                },
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
    migrate_v4(&v4(operations)).unwrap()
}

fn centerline_interpretation() -> v5::SvgInterpretation {
    v5::SvgInterpretation {
        geometry_tolerance_mm: 0.001,
        ticks_per_mm: None,
        mode: ImportMode::Centerline,
    }
}

fn file_input(filename: &str, svg: &str, origin: Point) -> ArtworkInput {
    ArtworkInput {
        filename: filename.into(),
        svg: svg.into(),
        interpretation: centerline_interpretation(),
        placement: Placement {
            origin_mm: origin,
            scale: 1.,
            rotation_deg: 0.,
        },
        name: None,
    }
}

/// A job with migrated artwork-1 plus a placed lettering item, and the
/// profile selecting one contour from each (the cross-source selection the
/// assignment commands must keep distinct).
fn two_source_job() -> v5::CamJobV5 {
    let base = migrated_job();
    let added = add_artwork(
        &base,
        vec![file_input("lettering.svg", LETTERING, Point::new(30., 0.))],
    )
    .unwrap()
    .outcome
    .job;
    let catalogue = inspect_artwork(&added).unwrap();
    let picks: Vec<_> = [MIGRATED_ARTWORK_ITEM_ID, "lettering"]
        .into_iter()
        .map(|item| {
            let entry = catalogue
                .item(&ArtworkItemId(item.into()))
                .unwrap()
                .entries
                .iter()
                .find(|entry| entry.kind == GeometryRefKind::ClosedContour)
                .unwrap();
            parse_wire_id(&entry.wire_id).unwrap()
        })
        .collect();
    set_contour_selection(
        &added,
        "profile-1",
        &[
            ProfileContourPick {
                geometry: picks[0].clone(),
                side: ContourSide::Outside,
                traversal: None,
            },
            ProfileContourPick {
                geometry: picks[1].clone(),
                side: ContourSide::Inside,
                traversal: None,
            },
        ],
    )
    .unwrap()
    .job
}

fn issue_codes(outcome: &CommandOutcome) -> Vec<String> {
    outcome.issues.iter().map(|i| i.code.clone()).collect()
}

fn readiness(job: &v5::CamJobV5) -> std::collections::BTreeMap<String, bool> {
    planning_readiness(job, &ReadinessScope::AllEnabled)
        .unwrap()
        .operations
        .into_iter()
        .map(|o| (o.operation_id, o.ready))
        .collect()
}

fn pick_of(job: &v5::CamJobV5, item: &str, kind: GeometryRefKind) -> artwork::GeometryPick {
    let catalogue = inspect_artwork(job).unwrap();
    let entry = catalogue
        .item(&ArtworkItemId(item.into()))
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.kind == kind)
        .unwrap();
    parse_wire_id(&entry.wire_id).unwrap()
}

/// Repair every selection and anchor through the assignment/reattach
/// commands so the job is fully ready against `item` again (the explicit
/// reattach route of addendum scenario 5).
fn reassign_all(job: &v5::CamJobV5, item: &str) -> v5::CamJobV5 {
    let job = set_component_selection(
        job,
        "carve-1",
        &[pick_of(job, item, GeometryRefKind::FilledComponent)],
    )
    .unwrap()
    .job;
    let job = set_contour_selection(
        &job,
        "profile-1",
        &[ProfileContourPick {
            geometry: pick_of(&job, item, GeometryRefKind::ClosedContour),
            side: ContourSide::Outside,
            traversal: None,
        }],
    )
    .unwrap()
    .job;
    let job = reattach_anchor(
        &job,
        "profile-1",
        AnchorTarget::Start,
        &pick_of(&job, item, GeometryRefKind::ClosedContour),
        None,
    )
    .unwrap()
    .job;
    let job = reattach_anchor(
        &job,
        "profile-1",
        AnchorTarget::Tab(0),
        &pick_of(&job, item, GeometryRefKind::ClosedContour),
        None,
    )
    .unwrap()
    .job;
    set_chain_selection(
        &job,
        "knife-1",
        &[pick_of(&job, item, GeometryRefKind::Centerline)],
    )
    .unwrap()
    .job
}

#[test]
fn batch_import_commits_accepted_additions_and_reports_per_file_rejection() {
    let base = migrated_job();
    let selections_before = match &base.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => settings.contours.len(),
        _ => panic!("profile"),
    };

    let AddArtworkOutcome { outcome, rejected } = add_artwork(
        &base,
        vec![
            file_input("lettering.svg", LETTERING, Point::new(30., 0.)),
            file_input("bad.svg", "not xml at all", Point::new(0., 0.)),
        ],
    )
    .unwrap();

    assert_eq!(rejected.len(), 1, "one per-file rejection");
    assert_eq!(rejected[0].filename, "bad.svg");
    assert_eq!(rejected[0].error.code, "SVG_XML");
    assert_eq!(outcome.job.artwork.len(), 2, "the valid file committed");
    assert_eq!(
        outcome.job.artwork[1].id,
        ArtworkItemId("lettering".into()),
        "IDs derive from the filename stem"
    );
    assert_eq!(
        outcome.job.artwork[1].name, "lettering.svg",
        "names default to the filename"
    );
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    assert_eq!(outcome.affected.len(), 1);
    // Adding an item never expands an existing selection.
    match &outcome.job.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => {
            assert_eq!(settings.contours.len(), selections_before);
            assert_eq!(
                settings.contours[0].geometry.artwork_item_id.0,
                MIGRATED_ARTWORK_ITEM_ID
            );
        }
        _ => panic!("profile"),
    }
    let json = outcome.job.to_json().unwrap();
    assert_eq!(v5::CamJobV5::from_json(&json).unwrap(), outcome.job);
}

#[test]
fn same_name_identical_byte_sources_stay_distinct() {
    let base = migrated_job();
    let AddArtworkOutcome { outcome, rejected } = add_artwork(
        &base,
        vec![
            file_input("art.svg", ARTWORK, Point::new(0., 0.)),
            file_input("art.svg", ARTWORK, Point::new(20., 0.)),
        ],
    )
    .unwrap();
    assert!(rejected.is_empty(), "{rejected:?}");
    let job = outcome.job;
    assert_eq!(
        job.artwork
            .iter()
            .map(|item| item.id.0.clone())
            .collect::<Vec<_>>(),
        vec![
            MIGRATED_ARTWORK_ITEM_ID.to_string(),
            "art".into(),
            "art-2".into()
        ],
        "identical filenames and bytes get distinct IDs"
    );

    // Wire IDs are owner-qualified, distinct across items and reversible,
    // including the colon-carrying `plate::0` component spelling.
    let catalogue = inspect_artwork(&job).unwrap();
    let mut wire_ids = std::collections::BTreeSet::new();
    for entry in catalogue.items.iter().flat_map(|item| item.entries.iter()) {
        assert!(wire_ids.insert(entry.wire_id.clone()), "distinct wire IDs");
        let expected = artwork::GeometryPick {
            artwork_item_id: entry.reference.artwork_item_id.clone(),
            kind: entry.kind,
            local_geometry_id: entry.reference.local_geometry_id.clone(),
        };
        assert_eq!(parse_wire_id(&entry.wire_id), Some(expected), "reversible");
        assert!(
            !entry.wire_id.contains('/'),
            "no unescaped filename material in identities"
        );
    }
    assert_eq!(
        catalogue
            .item(&ArtworkItemId("art".into()))
            .unwrap()
            .entries
            .len(),
        catalogue
            .item(&ArtworkItemId("art-2".into()))
            .unwrap()
            .entries
            .len()
    );

    // One profile can select contours from both identical items; the
    // selections stay distinct and resolve.
    let job = set_contour_selection(
        &job,
        "profile-1",
        &[
            ProfileContourPick {
                geometry: pick_of(&job, "art", GeometryRefKind::ClosedContour),
                side: ContourSide::Outside,
                traversal: None,
            },
            ProfileContourPick {
                geometry: pick_of(&job, "art-2", GeometryRefKind::ClosedContour),
                side: ContourSide::Outside,
                traversal: None,
            },
        ],
    )
    .unwrap()
    .job;
    let readiness = planning_readiness(&job, &ReadinessScope::AllEnabled).unwrap();
    assert!(
        readiness.operations.iter().all(|o| o.ready),
        "identical items resolve independently: {:?}",
        readiness.operations
    );

    // Duplication and artwork reorder keep them distinct; machining order
    // follows the operations vector, never the artwork rows.
    let job = duplicate_artwork(&job, &ArtworkItemId("art".into()), None)
        .unwrap()
        .job;
    assert_eq!(job.artwork.len(), 4);
    let job = reorder_artwork(
        &job,
        &[
            ArtworkItemId("art-2".into()),
            ArtworkItemId("art-copy".into()),
            ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
            ArtworkItemId("art".into()),
        ],
    )
    .unwrap()
    .job;
    assert_eq!(
        job.artwork
            .iter()
            .map(|i| i.id.0.clone())
            .collect::<Vec<_>>(),
        vec!["art-2", "art-copy", MIGRATED_ARTWORK_ITEM_ID, "art"],
        "the rows reordered"
    );
    assert_eq!(
        job.operations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>(),
        vec!["face-1", "carve-1", "profile-1", "knife-1"],
        "operation order never follows artwork order"
    );
    assert!(
        planning_readiness(&job, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
    let json = job.to_json().unwrap();
    assert_eq!(
        v5::CamJobV5::from_json(&json).unwrap().artwork.len(),
        4,
        "distinct after save/reopen"
    );
}

#[test]
fn moving_one_source_leaves_the_other_fixed() {
    let job = two_source_job();
    let before = inspect_artwork(&job).unwrap();
    let fixed_bounds = before
        .item(&ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()))
        .unwrap()
        .entries[0]
        .bounds;

    let outcome = place_artwork(
        &job,
        &ArtworkItemId("lettering".into()),
        Placement {
            origin_mm: Point::new(-5., 7.),
            scale: 2.,
            rotation_deg: 90.,
        },
    )
    .unwrap();
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    let after = inspect_artwork(&outcome.job).unwrap();
    assert_eq!(
        after
            .item(&ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()))
            .unwrap()
            .entries[0]
            .bounds,
        fixed_bounds,
        "the untouched source keeps its placed geometry"
    );
    assert_ne!(
        after
            .item(&ArtworkItemId("lettering".into()))
            .unwrap()
            .entries[0]
            .bounds,
        before
            .item(&ArtworkItemId("lettering".into()))
            .unwrap()
            .entries[0]
            .bounds,
        "the moved source actually moved"
    );

    // Placement edits preserve references and anchor fractions; the source
    // revision is untouched, so the moved item's own selection stays valid.
    assert!(
        planning_readiness(&outcome.job, &ReadinessScope::AllEnabled)
            .unwrap()
            .ready()
    );
    match &outcome.job.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => {
            let moved = &settings.contours[1].geometry;
            assert_eq!(moved.artwork_item_id.0, "lettering");
            assert_eq!(
                moved.source_revision,
                after
                    .item(&ArtworkItemId("lettering".into()))
                    .unwrap()
                    .revision
            );
            match &settings.tabs.as_ref().unwrap().placement {
                v5::TabPlacementV5::Manual { anchors } => {
                    assert_eq!(anchors[0].fraction_along_source_contour, 0.6)
                }
                _ => panic!("manual tabs"),
            }
        }
        _ => panic!("profile"),
    }
}

#[test]
fn replacement_with_changed_content_keeps_typed_dangling_references() {
    let base = migrated_job();
    let outcome = replace_artwork(
        &base,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        SourceSnapshot {
            filename: "art.svg".into(),
            svg: ARTWORK_EDITED.into(),
        },
        None,
    )
    .unwrap();
    // A sub-fingerprint-grid edit still changes the source revision; every
    // reference-bearing field reports its own located issue (the profile
    // owns three: contour, start anchor and tab anchor).
    let codes = issue_codes(&outcome);
    assert!(
        codes.iter().all(|code| code == "ARTWORK_REVISION_MISMATCH"),
        "{codes:?}"
    );
    let affected: std::collections::BTreeSet<_> = outcome
        .issues
        .iter()
        .filter_map(|issue| issue.operation_id.clone())
        .collect();
    assert_eq!(
        affected,
        ["carve-1", "knife-1", "profile-1"]
            .into_iter()
            .map(String::from)
            .collect(),
        "changed content blocks exactly the selecting operations"
    );
    assert_eq!(
        readiness(&outcome.job),
        vec![
            ("carve-1".to_string(), false),
            ("face-1".to_string(), true),
            ("knife-1".to_string(), false),
            ("profile-1".to_string(), false)
        ]
        .into_iter()
        .collect(),
        "changed content blocks exactly the selecting operations"
    );
    let json = outcome.job.to_json().unwrap();
    assert_eq!(v5::CamJobV5::from_json(&json).unwrap(), outcome.job);
}

#[test]
fn identical_replacement_retains_references() {
    let base = migrated_job();
    let outcome = replace_artwork(
        &base,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        SourceSnapshot {
            filename: "art.svg".into(),
            svg: ARTWORK.into(),
        },
        None,
    )
    .unwrap();
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    assert!(readiness(&outcome.job).values().all(|ready| *ready));
}

#[test]
fn failed_replacement_changes_nothing() {
    let base = migrated_job();
    let before = base.to_json().unwrap();
    let error = replace_artwork(
        &base,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        SourceSnapshot {
            filename: "art.svg".into(),
            svg: "not xml at all".into(),
        },
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "ARTWORK_COMMAND");
    assert!(
        error.message.contains("could not be replaced"),
        "{}",
        error.message
    );
    assert_eq!(base.to_json().unwrap(), before, "the old source survives");
}

#[test]
fn removal_retains_references_and_reassignment_repairs() {
    let base = migrated_job();
    let outcome = remove_artwork(&base, &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into())).unwrap();
    let codes = issue_codes(&outcome);
    for owner in ["carve-1", "profile-1", "knife-1"] {
        assert!(
            outcome.issues.iter().any(|issue| {
                issue.code == "ARTWORK_REFERENCE" && issue.operation_id.as_deref() == Some(owner)
            }),
            "{owner} keeps an actionable reference issue: {codes:?}"
        );
    }
    let json = outcome.job.to_json().unwrap();
    let reopened = v5::CamJobV5::from_json(&json).unwrap();
    assert!(
        reopened.artwork.is_empty(),
        "the broken draft round-trips without the item"
    );

    // The explicit repair route: re-add the content and reassign every
    // selection and anchor through the commands — no nearest-item guessing.
    let with_art = add_artwork(
        &outcome.job,
        vec![file_input("art.svg", ARTWORK, Point::new(3., 4.))],
    )
    .unwrap()
    .outcome
    .job;
    assert_eq!(with_art.artwork[0].id, ArtworkItemId("art".into()));
    let repaired = reassign_all(&with_art, "art");
    let inspection = v5::references::inspect_references(&repaired).unwrap();
    assert!(inspection.issues.is_empty(), "{:?}", inspection.issues);
    assert!(readiness(&repaired).values().all(|ready| *ready));
}

#[test]
fn assignment_commands_validate_picks() {
    let base = migrated_job();
    let job = add_artwork(
        &base,
        vec![file_input("lettering.svg", LETTERING, Point::new(0., 0.))],
    )
    .unwrap()
    .outcome
    .job;

    let unknown_item = artwork::GeometryPick {
        artwork_item_id: ArtworkItemId("ghost".into()),
        kind: GeometryRefKind::ClosedContour,
        local_geometry_id: "whatever".into(),
    };
    assert!(
        set_contour_selection(
            &job,
            "profile-1",
            &[ProfileContourPick {
                geometry: unknown_item,
                side: ContourSide::Outside,
                traversal: None,
            }]
        )
        .unwrap_err()
        .message
        .contains("unknown artwork item")
    );

    let unknown_local = artwork::GeometryPick {
        artwork_item_id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        kind: GeometryRefKind::ClosedContour,
        local_geometry_id: "ghost-contour".into(),
    };
    assert!(
        set_contour_selection(
            &job,
            "profile-1",
            &[ProfileContourPick {
                geometry: unknown_local,
                side: ContourSide::Outside,
                traversal: None,
            }]
        )
        .unwrap_err()
        .message
        .contains("unknown closed contour")
    );

    // Wrong kind: a filled component is not a profile contour.
    assert!(
        set_contour_selection(
            &job,
            "profile-1",
            &[ProfileContourPick {
                geometry: pick_of(
                    &job,
                    MIGRATED_ARTWORK_ITEM_ID,
                    GeometryRefKind::FilledComponent
                ),
                side: ContourSide::Outside,
                traversal: None,
            }]
        )
        .unwrap_err()
        .message
        .contains("must be a closed contour")
    );

    // On-contour rows require an explicit traversal direction.
    assert!(
        set_contour_selection(
            &job,
            "profile-1",
            &[ProfileContourPick {
                geometry: pick_of(
                    &job,
                    MIGRATED_ARTWORK_ITEM_ID,
                    GeometryRefKind::ClosedContour
                ),
                side: ContourSide::On,
                traversal: None,
            }]
        )
        .unwrap_err()
        .message
        .contains("traversal")
    );

    // Wrong operation kind.
    assert!(
        set_chain_selection(&job, "face-1", &[])
            .unwrap_err()
            .message
            .contains("not a drag-knife operation")
    );

    // Clearing a selection is an allowed incomplete state.
    let cleared = set_component_selection(&job, "carve-1", &[]).unwrap();
    assert!(cleared.issues.is_empty());
    match &cleared.job.operations[1].settings {
        v5::OperationSettingsV5::FlatVcarve(settings) => {
            assert!(settings.components.is_empty())
        }
        _ => panic!("carve"),
    }
}

#[test]
fn reattach_anchor_repairs_after_replacement() {
    let base = migrated_job();
    let replaced = replace_artwork(
        &base,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        SourceSnapshot {
            filename: "art.svg".into(),
            svg: ARTWORK_EDITED.into(),
        },
        None,
    )
    .unwrap()
    .job;

    // Reattach the start and tab anchors to the current geometry, keeping
    // the stored fractions; the fingerprint and revision are bound from the
    // catalogue, never from the stale anchor.
    let job = reattach_anchor(
        &replaced,
        "profile-1",
        AnchorTarget::Start,
        &pick_of(
            &replaced,
            MIGRATED_ARTWORK_ITEM_ID,
            GeometryRefKind::ClosedContour,
        ),
        None,
    )
    .unwrap()
    .job;
    let job = reattach_anchor(
        &job,
        "profile-1",
        AnchorTarget::Tab(0),
        &pick_of(
            &job,
            MIGRATED_ARTWORK_ITEM_ID,
            GeometryRefKind::ClosedContour,
        ),
        None,
    )
    .unwrap()
    .job;
    let catalogue = inspect_artwork(&job).unwrap();
    let contour = catalogue
        .item(&ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()))
        .unwrap()
        .entries
        .iter()
        .find(|entry| entry.kind == GeometryRefKind::ClosedContour)
        .unwrap();
    match &job.operations[2].settings {
        v5::OperationSettingsV5::Profile(settings) => {
            match &settings.start {
                v5::StartSelectionV5::Anchor(anchor) => {
                    assert_eq!(anchor.fraction_along_source_contour, 0.25);
                    assert_eq!(
                        anchor.source_geometry_fingerprint,
                        contour.source_fingerprint
                    );
                    assert_eq!(
                        anchor.geometry.source_revision,
                        contour.reference.source_revision
                    );
                }
                _ => panic!("anchor start"),
            }
            match &settings.tabs.as_ref().unwrap().placement {
                v5::TabPlacementV5::Manual { anchors } => {
                    assert_eq!(anchors[0].fraction_along_source_contour, 0.6);
                }
                _ => panic!("manual tabs"),
            }
        }
        _ => panic!("profile"),
    }

    // Errors: nonexistent tab anchor, out-of-range fraction, operations
    // without reattachable anchors.
    assert!(
        reattach_anchor(
            &job,
            "profile-1",
            AnchorTarget::Tab(9),
            &pick_of(
                &job,
                MIGRATED_ARTWORK_ITEM_ID,
                GeometryRefKind::ClosedContour
            ),
            None
        )
        .unwrap_err()
        .message
        .contains("no manual tab anchor 9")
    );
    assert!(
        reattach_anchor(
            &job,
            "profile-1",
            AnchorTarget::Start,
            &pick_of(
                &job,
                MIGRATED_ARTWORK_ITEM_ID,
                GeometryRefKind::ClosedContour
            ),
            Some(1.5)
        )
        .unwrap_err()
        .message
        .contains("[0,1)")
    );
    assert!(
        reattach_anchor(
            &job,
            "face-1",
            AnchorTarget::Start,
            &pick_of(
                &job,
                MIGRATED_ARTWORK_ITEM_ID,
                GeometryRefKind::ClosedContour
            ),
            None
        )
        .unwrap_err()
        .message
        .contains("no reattachable anchors")
    );
}

#[test]
fn fit_stock_proposes_bounds_and_applies_explicitly() {
    let job = two_source_job();
    let catalogue = inspect_artwork(&job).unwrap();
    // Union over every placed entry of each item (contours and centerline
    // chains alike: all of it is placed geometry the stock must cover).
    let union_bounds = |item: &str| -> artwork::SetupBounds {
        let mut bounds: Option<artwork::SetupBounds> = None;
        for entry in &catalogue.item(&ArtworkItemId(item.into())).unwrap().entries {
            bounds = Some(match bounds {
                Some(acc) => acc.union(entry.bounds),
                None => entry.bounds,
            });
        }
        bounds.unwrap()
    };
    let outline = union_bounds(MIGRATED_ARTWORK_ITEM_ID);
    let lettering = union_bounds("lettering");

    let request = FitStockRequest {
        item_ids: vec![
            ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
            ArtworkItemId("lettering".into()),
        ],
        margins: FitStockMargins {
            min_x_mm: 1.,
            max_x_mm: 2.,
            min_y_mm: 3.,
            max_y_mm: 4.,
        },
    };
    let proposal = propose_fit_stock(&job, &request).unwrap();
    let expected = RectXY {
        min_x_mm: outline.min_x_mm.min(lettering.min_x_mm) - 1.,
        min_y_mm: outline.min_y_mm.min(lettering.min_y_mm) - 3.,
        width_mm: outline.max_x_mm.max(lettering.max_x_mm)
            - outline.min_x_mm.min(lettering.min_x_mm)
            + 1.
            + 2.,
        length_mm: outline.max_y_mm.max(lettering.max_y_mm)
            - outline.min_y_mm.min(lettering.min_y_mm)
            + 3.
            + 4.,
    };
    assert_eq!(proposal.rect, expected);
    assert!(
        job.setup.stock.xy.is_none(),
        "proposing never changes the document"
    );

    // Fitting one item only bounds that item.
    let single = propose_fit_stock(
        &job,
        &FitStockRequest {
            item_ids: vec![ArtworkItemId("lettering".into())],
            margins: FitStockMargins::default(),
        },
    )
    .unwrap()
    .rect;
    assert_eq!(single.min_x_mm, lettering.min_x_mm);
    assert_eq!(single.width_mm, lettering.max_x_mm - lettering.min_x_mm);

    let outcome = apply_stock_rectangle(&job, proposal.rect).unwrap();
    assert_eq!(outcome.job.setup.stock.xy, Some(proposal.rect));
    assert_eq!(outcome.job.setup.stock.thickness_mm, Some(8.));
    assert_eq!(outcome.affected, vec![v5::commands::AffectedEntity::Setup]);

    // Invalid requests stay located errors.
    assert_eq!(
        propose_fit_stock(
            &job,
            &FitStockRequest {
                item_ids: vec![],
                margins: FitStockMargins::default(),
            }
        )
        .unwrap_err()
        .code,
        "ARTWORK_COMMAND"
    );
    assert!(
        propose_fit_stock(
            &job,
            &FitStockRequest {
                item_ids: vec![ArtworkItemId("ghost".into())],
                margins: FitStockMargins::default(),
            }
        )
        .unwrap_err()
        .message
        .contains("unknown artwork item")
    );
    assert!(
        propose_fit_stock(
            &job,
            &FitStockRequest {
                item_ids: vec![ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into())],
                margins: FitStockMargins {
                    min_x_mm: -1.,
                    ..Default::default()
                },
            }
        )
        .unwrap_err()
        .message
        .contains("nonnegative")
    );
}

#[test]
fn aggregate_document_limit_preserves_the_prior_document() {
    let mut job = migrated_job();
    // Per-item content sits exactly at the per-item limit, so structure
    // validation passes while the aggregate boundary must refuse.
    let oversized = v5::ArtworkContent::Svg(SourceSnapshot {
        filename: "art.svg".into(),
        svg: "x".repeat(cam_core::svg::MAX_SVG_BYTES),
    });
    job.artwork[0].content = oversized.clone();
    let mut second = job.artwork[0].clone();
    second.id = ArtworkItemId("art-2".into());
    job.artwork.push(second);
    assert!(job.validate_structure().is_ok());
    assert_eq!(
        job.to_json().unwrap_err().code,
        "PROJECT_RESOURCE_LIMIT",
        "the serialized collection exceeds the aggregate document budget"
    );

    // A command that would grow the aggregate fails with the prior document
    // preserved (commands never mutate their input).
    let error =
        duplicate_artwork(&job, &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()), None).unwrap_err();
    assert_eq!(error.code, "PROJECT_RESOURCE_LIMIT");
    assert_eq!(
        job.artwork.len(),
        2,
        "the failed command preserves the prior document"
    );
    let mut restored = job.clone();
    restored.artwork.clear();
    restored.artwork.push(v5::ArtworkItem {
        id: ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        name: "art.svg".into(),
        content: v5::ArtworkContent::Svg(SourceSnapshot {
            filename: "art.svg".into(),
            svg: ARTWORK.into(),
        }),
        import_settings: centerline_interpretation(),
        placement: placement(),
    });
    assert!(
        restored.to_json().is_ok(),
        "only content size, not the command path, broke the document"
    );
}

#[test]
fn rename_and_duplicate_do_not_disturb_machining_references() {
    let base = migrated_job();
    let revision_before = base.artwork[0].source_revision().unwrap();

    let renamed = rename_artwork(
        &base,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        "Renamed artwork",
    )
    .unwrap();
    assert!(renamed.issues.is_empty(), "{:?}", renamed.issues);
    assert_eq!(
        renamed.job.artwork[0].source_revision().unwrap(),
        revision_before,
        "renaming never invalidates references"
    );

    let duplicated = duplicate_artwork(
        &renamed.job,
        &ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()),
        Some("Backup"),
    )
    .unwrap();
    assert!(duplicated.issues.is_empty(), "{:?}", duplicated.issues);
    assert_eq!(duplicated.job.artwork.len(), 2);
    assert_eq!(
        duplicated.job.artwork[1].id,
        ArtworkItemId("artwork-1-copy".into())
    );
    assert_eq!(duplicated.job.artwork[1].name, "Backup");
    // Nothing selects the duplicate: every operation still references the
    // original item.
    match &duplicated.job.operations[1].settings {
        v5::OperationSettingsV5::FlatVcarve(settings) => {
            assert_eq!(
                settings.components[0].artwork_item_id.0,
                MIGRATED_ARTWORK_ITEM_ID
            );
        }
        _ => panic!("carve"),
    }
    let json = duplicated.job.to_json().unwrap();
    assert_eq!(v5::CamJobV5::from_json(&json).unwrap(), duplicated.job);
}

#[test]
fn unknown_targets_and_broken_reorders_are_located_errors() {
    let base = migrated_job();
    let ghost = ArtworkItemId("ghost".into());
    for message in [
        rename_artwork(&base, &ghost, "x").unwrap_err().message,
        remove_artwork(&base, &ghost).unwrap_err().message,
        place_artwork(&base, &ghost, placement())
            .unwrap_err()
            .message,
        duplicate_artwork(&base, &ghost, None).unwrap_err().message,
        replace_artwork(
            &base,
            &ghost,
            SourceSnapshot {
                filename: "a.svg".into(),
                svg: ARTWORK.into(),
            },
            None,
        )
        .unwrap_err()
        .message,
    ] {
        assert!(message.contains("unknown artwork item"), "{message}");
    }

    // A reorder missing items or listing an unknown one changes nothing.
    let error = reorder_artwork(&base, &[]).unwrap_err();
    assert_eq!(error.code, "ARTWORK_COMMAND");
    assert!(error.message.contains("missing item"), "{error}");
    let error = reorder_artwork(
        &base,
        &[ArtworkItemId(MIGRATED_ARTWORK_ITEM_ID.into()), ghost],
    )
    .unwrap_err();
    assert!(error.message.contains("exactly once"), "{error}");
    assert_eq!(base.artwork.len(), 1);
}
