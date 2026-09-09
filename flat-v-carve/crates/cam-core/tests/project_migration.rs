//! Migration of legacy schema-1/2/3 jobs into the canonical schema-4 model.
use cam_core::{
    geometry::Point,
    job::Job as LegacyJob,
    project::{
        CamJob, FlatVcarveMode, OperationSettings, ToolGeometry, migrate::migrate_job,
        migrate::migrate_legacy_json,
    },
};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
const M4_CONTACT_LINE: &str = include_str!("../../../fixtures/m4/contact-line.json");
const FLOWER_COMBINED: &str = include_str!("../../../../real_data/flower_box-svg.job-real.json");

fn flat_vcarve(job: &CamJob) -> &cam_core::project::FlatVcarveSettings {
    let OperationSettings::FlatVcarve(settings) = &job.operations[0].settings else {
        panic!("migrated job must contain one Flat V-carve operation");
    };
    settings
}

#[test]
fn complete_endmill_only_job_migrates_with_all_values() {
    // Native schema-2 document: exercises the legacy upgrade path too.
    let legacy = LegacyJob::from_json(M3_RECTANGLE).unwrap();
    assert!(legacy.vbit_planning.is_none());
    let job = migrate_job(&legacy).unwrap();

    assert_eq!(job.schema_version, 4);
    assert_eq!(job.name, legacy.name);
    assert_eq!(job.source.as_ref().unwrap().svg, legacy.source.svg);
    assert_eq!(
        job.source.as_ref().unwrap().filename,
        legacy.source.filename
    );
    assert_eq!(job.import, legacy.import);
    assert_eq!(job.setup.stock.thickness_mm, legacy.stock.thickness_mm);
    assert_eq!(job.setup.stock.xy, None, "legacy jobs have no physical XY");

    let planning = legacy.endmill_planning.as_ref().unwrap();
    assert_eq!(
        job.setup.clearance_above_stock_mm,
        Some(planning.clearance_z_mm)
    );
    assert_eq!(
        job.setup.start_xy_mm,
        Some(Point::new(planning.start_xy_mm.x, planning.start_xy_mm.y))
    );
    assert_eq!(job.tolerances, legacy.tolerances);
    assert_eq!(job.legacy_machine_profile, legacy.machine_profile);

    assert_eq!(job.operations.len(), 1);
    assert_eq!(job.operations[0].id, legacy.operation.id);
    assert!(job.operations[0].enabled);
    let settings = flat_vcarve(&job);
    assert_eq!(settings.mode, FlatVcarveMode::EndmillOnly);
    assert_eq!(settings.finish, None);
    assert_eq!(settings.component_ids, legacy.selected_region_ids);
    assert_eq!(settings.max_depth_mm, legacy.operation.max_depth_mm);
    assert_eq!(
        settings.wall_allowance_mm,
        legacy.operation.wall_allowance_mm
    );
    assert_eq!(
        settings.max_floor_ridge_mm,
        legacy.operation.max_floor_ridge_mm
    );
    assert_eq!(
        settings.max_detail_residual_mm,
        legacy.operation.max_detail_residual_mm
    );

    let rough = settings.rough.as_ref().unwrap();
    let planning = legacy.endmill_planning.as_ref().unwrap();
    assert_eq!(rough.strategy, planning.strategy);
    assert_eq!(rough.entry, planning.entry);
    assert_eq!(rough.max_layers, planning.max_layers);
    assert_eq!(rough.max_loops_per_layer, planning.max_loops_per_layer);
    assert_eq!(rough.max_motions, planning.max_motions);

    // Tool snapshots: one authoritative plunge capability, values preserved.
    let endmill = job.tools.iter().find(|t| t.id == "endmill").unwrap();
    assert_eq!(
        endmill.geometry,
        Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: 4.,
            cutting_length_mm: 6.,
        }))
    );
    assert_eq!(endmill.capabilities.plunge_capable, Some(true));
    assert_eq!(endmill.capabilities.ramp_capable, Some(false));
    let vbit = job.tools.iter().find(|t| t.id == "vbit").unwrap();
    assert!(matches!(vbit.geometry, Some(ToolGeometry::Vbit(_))));
    // The slot-level value stays unset; nothing is invented for it.
    assert_eq!(vbit.capabilities.plunge_capable, None);

    let legacy_endmill = legacy.tools.iter().find(|t| t.id == "endmill").unwrap();
    assert_eq!(settings.endmill.tool_id, legacy_endmill.id);
    assert_eq!(settings.endmill.spindle_rpm, legacy_endmill.spindle_rpm);
    assert_eq!(
        settings.endmill.cutting_feed_mm_min,
        legacy_endmill.cutting_feed_mm_min
    );
    assert_eq!(
        settings.endmill.plunge_feed_mm_min,
        legacy_endmill.plunge_feed_mm_min
    );
    assert_eq!(
        settings.endmill.max_stepdown_mm,
        legacy_endmill.max_stepdown_mm
    );
    assert_eq!(settings.endmill.stepover_mm, legacy_endmill.stepover_mm);
    assert_eq!(settings.endmill.spindle_direction, None);
    assert_eq!(settings.vbit.spindle_direction, None);
}

#[test]
fn combined_flower_job_migrates_with_finish_settings() {
    let legacy = LegacyJob::from_json(FLOWER_COMBINED).unwrap();
    let job = migrate_job(&legacy).unwrap();
    let settings = flat_vcarve(&job);
    assert_eq!(settings.mode, FlatVcarveMode::Combined);
    let finish = settings.finish.as_ref().unwrap();
    let legacy_finish = legacy.vbit_planning.as_ref().unwrap();
    assert_eq!(finish, legacy_finish);
    assert!(!settings.component_ids.is_empty());
    assert_eq!(settings.component_ids, legacy.selected_region_ids);
}

#[test]
fn migrated_jobs_round_trip_through_schema_four() {
    for json in [M3_RECTANGLE, M4_CONTACT_LINE, FLOWER_COMBINED] {
        let job = migrate_legacy_json(json).unwrap();
        let serialized = job.to_json().unwrap();
        let reloaded = CamJob::from_json(&serialized).unwrap();
        assert_eq!(reloaded, job, "schema-4 round trip must preserve the job");
        assert_eq!(reloaded.to_json().unwrap(), serialized);
    }
}

#[test]
fn incomplete_legacy_job_migrates_with_unset_values_preserved() {
    let mut value: serde_json::Value = serde_json::from_str(M3_RECTANGLE).unwrap();
    // Strip every optional machining value but keep the document structural.
    value.as_object_mut().unwrap().remove("endmill_planning");
    value["stock"]["thickness_mm"] = serde_json::Value::Null;
    value["operation"]["max_depth_mm"] = serde_json::Value::Null;
    value["tools"][0]["spindle_rpm"] = serde_json::Value::Null;
    value["tools"][1]["stepover_mm"] = serde_json::Value::Null;
    value["tools"][1]["plunge_capable"] = serde_json::Value::Null;
    let json = serde_json::to_string(&value).unwrap();
    let legacy = LegacyJob::from_json(&json).unwrap();
    let job = migrate_job(&legacy).unwrap();

    assert_eq!(job.setup.clearance_above_stock_mm, None);
    assert_eq!(job.setup.start_xy_mm, None);
    assert_eq!(job.setup.stock.thickness_mm, None);
    let settings = flat_vcarve(&job);
    assert_eq!(settings.max_depth_mm, None);
    assert_eq!(settings.rough, None);
    // Endmill-only mode is preserved from the absent V-bit planning block.
    assert_eq!(settings.mode, FlatVcarveMode::EndmillOnly);
    assert_eq!(settings.endmill.spindle_rpm, None);
    assert_eq!(settings.vbit.stepover_mm, None);
    // The V-bit slot plunge capability stays unset, not invented.
    let vbit = job.tools.iter().find(|t| t.id == "vbit").unwrap();
    assert_eq!(vbit.capabilities.plunge_capable, None);
    job.validate().unwrap();
    // The migrated incomplete document still round-trips.
    let reloaded = CamJob::from_json(&job.to_json().unwrap()).unwrap();
    assert_eq!(reloaded, job);
}

#[test]
fn legacy_schema_two_document_loads_through_existing_upgrade_path() {
    // M3 fixtures are stored as schema 2 and upgraded by the legacy loader.
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    assert_eq!(job.schema_version, 4);
    assert_eq!(
        flat_vcarve(&job).component_ids,
        vec!["pocket::0".to_string()]
    );
}

#[test]
fn conflicting_legacy_plunge_capability_is_rejected_before_migration() {
    let mut value: serde_json::Value = serde_json::from_str(M3_RECTANGLE).unwrap();
    // Endmill dimensions declare plunge_capable=true; contradict the slot.
    value["tools"][0]["plunge_capable"] = serde_json::json!(false);
    let json = serde_json::to_string(&value).unwrap();
    let err = LegacyJob::from_json(&json).unwrap_err();
    assert_eq!(err.code, "JOB_TOOL_CAPABILITY");
}

#[test]
fn migrated_work_zero_preserves_legacy_coordinates() {
    let job = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let work_zero = &job.setup.work_zero;
    assert!(matches!(
        work_zero.xy,
        cam_core::project::WorkZeroXY::SetupOrigin
    ));
    assert!(matches!(
        work_zero.z,
        cam_core::project::WorkZeroZ::StockTop
    ));
}
