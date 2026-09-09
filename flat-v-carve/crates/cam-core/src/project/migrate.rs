//! Migration of validated legacy schema-3 jobs into the canonical schema-4 model.
//!
//! Legacy jobs are loaded and validated by [`crate::job::Job`] (including its
//! schema-1/2 upgrade path) before migration. Missing machining values stay
//! unset; migration never invents feeds, spindle data, stock bounds or knife
//! offsets. See plan section 16.1 for the field mapping.
use crate::job::{
    Job as LegacyJob, ToolGeometry as LegacyToolGeometry, ToolSettings as LegacyToolSettings,
};
use crate::{
    geometry::Result,
    pocket::EndmillPlanningSettings,
    project::{
        CAM_JOB_SCHEMA_VERSION, CamJob, EndmillGeometry, FlatVcarveMode, FlatVcarveRoughSettings,
        FlatVcarveSettings, JobTool, MillingAssignment, Operation, OperationSettings,
        SetupSettings, StockSetup, ToolCapabilities, ToolGeometry,
    },
};

fn migrated_geometry(geometry: &LegacyToolGeometry) -> ToolGeometry {
    match geometry {
        LegacyToolGeometry::Endmill(spec) => ToolGeometry::Endmill(EndmillGeometry {
            diameter_mm: spec.diameter_mm,
            cutting_length_mm: spec.cutting_length_mm,
        }),
        LegacyToolGeometry::Vbit(spec) => ToolGeometry::Vbit(spec.clone()),
    }
}

fn migrated_capabilities(tool: &LegacyToolSettings) -> ToolCapabilities {
    let plunge_capable = match &tool.geometry {
        // The endmill spec is the single authoritative plunge value; the legacy
        // validator has already rejected conflicting slot values.
        Some(LegacyToolGeometry::Endmill(spec)) => Some(spec.plunge_capable),
        _ => tool.plunge_capable,
    };
    ToolCapabilities {
        plunge_capable,
        ramp_capable: tool.ramp_capable,
    }
}

fn migrated_assignment(tool: &LegacyToolSettings) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool.id.clone(),
        // Legacy jobs stored no spindle direction; it stays unset until a
        // machine profile is explicitly imported and applied.
        spindle_direction: None,
        spindle_rpm: tool.spindle_rpm,
        cutting_feed_mm_min: tool.cutting_feed_mm_min,
        plunge_feed_mm_min: tool.plunge_feed_mm_min,
        max_stepdown_mm: tool.max_stepdown_mm,
        stepover_mm: tool.stepover_mm,
    }
}

fn migrated_rough(settings: &EndmillPlanningSettings) -> FlatVcarveRoughSettings {
    FlatVcarveRoughSettings {
        strategy: settings.strategy,
        entry: settings.entry.clone(),
        max_layers: settings.max_layers,
        max_loops_per_layer: settings.max_loops_per_layer,
        max_motions: settings.max_motions,
    }
}

/// Migrate a validated legacy job. The result describes the same machining
/// intent; legacy planning controls move into the single Flat V-carve
/// operation, and shared travel settings move into setup.
pub fn migrate_job(legacy: &LegacyJob) -> Result<CamJob> {
    legacy.validate_settings()?;
    let operation_id = legacy.operation.id.clone();
    let endmill_tool = legacy
        .tools
        .iter()
        .find(|t| t.id == legacy.operation.endmill_id)
        .expect("validated legacy jobs reference existing tools");
    let vbit_tool = legacy
        .tools
        .iter()
        .find(|t| t.id == legacy.operation.vbit_id)
        .expect("validated legacy jobs reference existing tools");
    let mode = if legacy.vbit_planning.is_some() {
        FlatVcarveMode::Combined
    } else {
        FlatVcarveMode::EndmillOnly
    };
    let setup = match &legacy.endmill_planning {
        Some(planning) => SetupSettings {
            stock: StockSetup {
                thickness_mm: legacy.stock.thickness_mm,
                xy: None,
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(planning.clearance_z_mm),
            start_xy_mm: Some(planning.start_xy_mm),
        },
        None => SetupSettings {
            stock: StockSetup {
                thickness_mm: legacy.stock.thickness_mm,
                xy: None,
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: None,
            start_xy_mm: None,
        },
    };
    let job = CamJob {
        schema_version: CAM_JOB_SCHEMA_VERSION,
        name: legacy.name.clone(),
        source: Some(legacy.source.clone()),
        import: legacy.import.clone(),
        setup,
        tools: legacy
            .tools
            .iter()
            .map(|t| JobTool {
                id: t.id.clone(),
                name: t.id.clone(),
                geometry: t.geometry.as_ref().map(migrated_geometry),
                capabilities: migrated_capabilities(t),
            })
            .collect(),
        operations: vec![Operation {
            id: operation_id,
            name: "Flat V-carve".into(),
            enabled: true,
            settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
                component_ids: legacy.selected_region_ids.clone(),
                mode,
                endmill: migrated_assignment(endmill_tool),
                vbit: migrated_assignment(vbit_tool),
                max_depth_mm: legacy.operation.max_depth_mm,
                wall_allowance_mm: legacy.operation.wall_allowance_mm,
                max_floor_ridge_mm: legacy.operation.max_floor_ridge_mm,
                max_detail_residual_mm: legacy.operation.max_detail_residual_mm,
                rough: legacy.endmill_planning.as_ref().map(migrated_rough),
                finish: legacy.vbit_planning.clone(),
            }),
        }],
        tolerances: legacy.tolerances.clone(),
        legacy_machine_profile: legacy.machine_profile.clone(),
    };
    job.validate()?;
    Ok(job)
}

/// Load a legacy job JSON document (schema 1/2/3) and migrate it to schema 4.
pub fn migrate_legacy_json(json: &str) -> Result<CamJob> {
    let legacy = LegacyJob::from_json(json)?;
    migrate_job(&legacy)
}
