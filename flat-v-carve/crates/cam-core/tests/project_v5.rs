//! H1 schema-5 collection model: artwork/ref/provenance shapes, the
//! integrity-vs-readiness validation split and scope-local blocking of
//! dangling references (plan sections 22.2, 22.3, 22.5 and 22.11). The
//! fixtures are stored schema-5 documents: nothing here converts a schema.
use cam_core::{
    geometry::Point,
    job::SourceSnapshot,
    project::v5::{
        self, AppliedMachineConfiguration, AppliedProfile, AppliedToolMapping, ArtworkContent,
        ArtworkItemId, CamJobV5, ConfigurationOrigin, CuttingBaseline, GeometryRef, LibraryOrigin,
        ReadinessScope,
        references::{inspect_references, planning_readiness},
    },
};

/// The single artwork item the collection fixtures carry.
const FIXTURE_ARTWORK: &str = "artwork-1";
/// The same artwork with a sub-fingerprint-grid edit (0.001 mm, below the
/// 0.01 mm contour fingerprint quantization).
const ARTWORK_EDITED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4.001"/></svg>"##;

/// The collection fixture: one artwork item with face, carve, profile
/// (anchor start + manual tab) and knife operations, stored as a schema-5
/// document. It was generated once from the schema-4 fixture the deleted
/// migration used to produce, so this suite tests the model, not a conversion.
fn full_v5_job() -> CamJobV5 {
    serde_json::from_str(include_str!("../../../fixtures/v5/full-job-machine.json")).unwrap()
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
fn dangling_references_round_trip_and_block_only_the_affected_scope() {
    let mut migrated = full_v5_job();
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
    let mut migrated = full_v5_job();
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
    let migrated = full_v5_job();

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
    let mut job = full_v5_job();
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
        artwork_item_id: ArtworkItemId(FIXTURE_ARTWORK.into()),
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
    let mut job = full_v5_job();
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
    let mut job = full_v5_job();
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
    let base = full_v5_job();

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
    assert!(cam_core::job::input_from_fixture_json(&json).is_err());
}

#[test]
fn unresolvable_items_report_import_failures_without_blocking_saves() {
    let mut job = full_v5_job();
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
