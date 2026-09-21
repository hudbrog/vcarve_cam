use cam_core::{
    geometry::Point,
    project::v5::{
        self, CamJobV5, GeometryPick, GeometryRefKind, OperationSettingsV5, PocketEntry,
        commands::{self, NewOperationKind},
        resources::{self, AssignmentRole},
    },
    sequence::OperationPlanV5,
};

const DRAWING: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="40mm" viewBox="0 0 80 40"><path id="island" fill-rule="evenodd" d="M2 2H32V32H2Z M12 12H22V22H12Z"/><rect id="second" x="45" y="5" width="25" height="25"/></svg>"#;

fn job() -> CamJobV5 {
    let job = v5::authoring::from_svg("pockets.svg".into(), DRAWING.into(), 0.001).unwrap();
    commands::add_operation(&job, NewOperationKind::Pocket, "pocket-1", "Pockets")
        .unwrap()
        .job
}
fn settings(job: &mut CamJobV5) -> &mut v5::PocketSettingsV5 {
    let OperationSettingsV5::Pocket(s) = &mut job.operations[0].settings else {
        panic!()
    };
    s
}
fn selected() -> CamJobV5 {
    let job = job();
    let catalogue = v5::inspect_artwork(&job).unwrap();
    let item = &catalogue.items[0];
    let picks: Vec<_> = item
        .catalogue
        .as_ref()
        .unwrap()
        .contours
        .iter()
        .filter(|c| c.role == cam_core::contours::ContourRole::Outer)
        .map(|c| GeometryPick {
            artwork_item_id: item.id.clone(),
            kind: GeometryRefKind::FilledComponent,
            local_geometry_id: c.component_id.clone(),
        })
        .collect();
    commands::set_component_selection(&job, "pocket-1", &picks)
        .unwrap()
        .job
}

#[test]
fn incomplete_pocket_round_trips_without_a_vbit() {
    let job = job();
    assert_eq!(job.tools.len(), 1);
    assert_eq!(CamJobV5::from_json(&job.to_json().unwrap()).unwrap(), job);
    let missing = v5::inspection::inspect_operation_fields(&job, "pocket-1").unwrap();
    assert!(
        missing
            .iter()
            .any(|m| m.field_path.as_deref() == Some("components"))
    );
    assert!(!missing.iter().any(|m| m.message.contains("vbit")));
    assert_eq!(
        resources::assignments_of(&job),
        vec![("pocket-1".into(), AssignmentRole::Milling)]
    );
}

#[test]
fn pocket_copies_library_geometry_and_preset_and_resets_without_the_library() {
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/gui5/library.json")).unwrap();
    let library: cam_core::tool_library::ToolLibrary =
        serde_json::from_value(document["library"].clone()).unwrap();
    let (imported, tool_id) =
        resources::add_library_tool(&job(), &library, "test-library", "endmill").unwrap();
    let mut job =
        resources::use_job_tool(&imported.job, "pocket-1", AssignmentRole::Milling, &tool_id)
            .unwrap()
            .job;
    job = resources::apply_cutting_profile(
        &job,
        "pocket-1",
        AssignmentRole::Milling,
        &library,
        "test-library",
        "endmill",
        "rough",
    )
    .unwrap()
    .job;
    let baseline = settings(&mut job).assignment.clone();
    assert_eq!(baseline.cutting_feed_mm_min, Some(1200.));
    assert!(
        job.tools
            .iter()
            .find(|t| t.id == tool_id)
            .unwrap()
            .geometry
            .is_some()
    );
    settings(&mut job).assignment.cutting_feed_mm_min = Some(900.);
    job = CamJobV5::from_json(&job.to_json().unwrap()).unwrap();
    job = resources::reset_assignment(&job, "pocket-1", AssignmentRole::Milling)
        .unwrap()
        .job;
    assert_eq!(settings(&mut job).assignment, baseline);
}

#[test]
fn multi_selection_preserves_islands_and_used_by_references() {
    let mut job = selected();
    let components = settings(&mut job).components.clone();
    assert_eq!(components.len(), 2);
    let region =
        v5::resolve::resolve_filled_region(&job, &components, &v5::inspect_artwork(&job).unwrap())
            .unwrap();
    assert_eq!(region.region.component_count(), 2);
    assert_eq!(region.region.hole_count(), 1);
    assert!((region.region.area_mm2() - 1425.).abs() < 0.01);
    assert_eq!(v5::inspection::used_by_index(&job).geometries.len(), 2);
    job.artwork[0].placement.origin_mm = Point::new(1., 2.);
    let moved =
        v5::resolve::resolve_filled_region(&job, &components, &v5::inspect_artwork(&job).unwrap())
            .unwrap();
    assert_eq!(moved.bounds.unwrap().min, Point::new(1., 6.));
}

#[test]
fn placed_overlapping_artwork_unions_on_the_finest_selected_grid() {
    let original = selected();
    let mut job = commands::duplicate_artwork(&original, &original.artwork[0].id, None)
        .unwrap()
        .job;
    job.artwork[1].placement.origin_mm.x = 5.;
    job.artwork[1].import_settings.geometry_tolerance_mm = 0.0005;
    let catalogue = v5::inspect_artwork(&job).unwrap();
    let picks: Vec<_> = catalogue
        .items
        .iter()
        .flat_map(|item| &item.entries)
        .filter(|e| e.kind == GeometryRefKind::FilledComponent)
        .map(|e| GeometryPick {
            artwork_item_id: e.reference.artwork_item_id.clone(),
            kind: e.kind,
            local_geometry_id: e.reference.local_geometry_id.clone(),
        })
        .collect();
    job = commands::set_component_selection(&job, "pocket-1", &picks)
        .unwrap()
        .job;
    let refs = settings(&mut job).components.clone();
    let resolved = v5::resolve::resolve_filled_region(&job, &refs, &catalogue).unwrap();
    assert_eq!(refs.len(), 4);
    assert_eq!(resolved.region.component_count(), 2);
    assert_eq!(resolved.region.hole_count(), 1);
    assert!((resolved.region.area_mm2() - 1750.).abs() < 0.01);
    assert_eq!(resolved.grid_tolerance_mm, 0.0005);
}

#[test]
fn pocket_identity_tracks_geometry_depth_and_entry_but_not_labels() {
    let mut job = selected();
    let identity = |j: &CamJobV5| {
        OperationPlanV5::machining_identity(j, &v5::ReadinessScope::AllEnabled).unwrap()
    };
    let base = identity(&job);
    job.operations[0].name = "Renamed".into();
    assert_eq!(base, identity(&job));
    settings(&mut job).bottom.offset_mm = -3.;
    let deeper = identity(&job);
    assert_ne!(base, deeper);
    settings(&mut job).entry = PocketEntry::Helix {
        radius_mm: Some(1.),
        max_angle_deg: Some(3.),
        feed_mm_min: Some(150.),
    };
    assert_ne!(deeper, identity(&job));
    let helix = identity(&job);
    job.artwork[0].placement.origin_mm.x = 2.;
    assert_ne!(helix, identity(&job));
}

#[test]
fn malformed_entry_and_wrong_geometry_kind_are_rejected() {
    let mut job = selected();
    settings(&mut job).entry = PocketEntry::Helix {
        radius_mm: Some(-1.),
        max_angle_deg: Some(3.),
        feed_mm_min: Some(150.),
    };
    assert!(job.to_json().is_err());
    settings(&mut job).entry = PocketEntry::Ramp {
        max_angle_deg: Some(90.),
        feed_mm_min: Some(150.),
    };
    assert!(job.to_json().is_err());
    settings(&mut job).entry = PocketEntry::Plunge;
    settings(&mut job).components[0].kind = GeometryRefKind::Point;
    assert!(job.to_json().is_err());
}

#[test]
fn stale_and_deleted_sources_remain_saveable_but_block_readiness() {
    let mut job = selected();
    settings(&mut job).components[0]
        .source_revision
        .content_digest = "0".repeat(64);
    assert!(job.to_json().is_ok());
    let issues = v5::references::inspect_references(&job).unwrap().issues;
    assert!(issues.iter().any(|d| d.code == "ARTWORK_REVISION_MISMATCH"));
    job.artwork.clear();
    assert!(job.to_json().is_ok());
    let readiness =
        v5::references::planning_readiness(&job, &v5::ReadinessScope::AllEnabled).unwrap();
    assert!(!readiness.operations[0].ready);
}
