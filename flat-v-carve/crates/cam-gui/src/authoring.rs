//! Single-source authoring over the canonical model. No example-job defaults.
use cam_core::{
    job::SourceSnapshot,
    project::{self, FlatVcarveMode, ToolGeometry, WorkZeroXY, v5::*},
};
use serde::{Deserialize, Serialize};

pub fn import_svg(filename: String, svg: String) -> Result<CamJobV5, String> {
    if svg.len() > 8_000_000 {
        return Err("SVG exceeds 8 MB".into());
    }
    let item = ArtworkItem {
        id: ArtworkItemId("artwork-1".into()),
        name: filename.clone(),
        content: ArtworkContent::Svg(SourceSnapshot {
            filename: filename.clone(),
            svg,
        }),
        import_settings: SvgInterpretation::default(),
        placement: Default::default(),
    };
    // Actual import admission belongs to the core, including units and fills.
    let catalogue = artwork::resolve_artwork_item(&item).map_err(|e| e.to_string())?;
    if let Some(error) = catalogue.import_error {
        return Err(error);
    }
    if !catalogue
        .entries
        .iter()
        .any(|c| c.kind == GeometryRefKind::FilledComponent)
    {
        return Err(
            "SVG has no supported filled components. Convert text and strokes to paths.".into(),
        );
    }
    let assignment = |id: &str| MillingAssignmentV5 {
        tool_id: id.into(),
        spindle_rpm: None,
        spindle_direction: None,
        cutting_feed_mm_min: None,
        plunge_feed_mm_min: None,
        max_stepdown_mm: None,
        stepover_mm: None,
        applied_profile: None,
    };
    let job = CamJobV5 {
        schema_version: 5,
        name: filename,
        setup: Default::default(),
        artwork: vec![item],
        tools: [("endmill", "Endmill"), ("vbit", "V-bit target")]
            .into_iter()
            .map(|(id, name)| JobToolV5 {
                id: id.into(),
                name: name.into(),
                geometry: None,
                capabilities: Default::default(),
                library_origin: None,
            })
            .collect(),
        operations: vec![OperationV5 {
            id: "carving".into(),
            name: "Flat V-carve".into(),
            enabled: true,
            settings: OperationSettingsV5::FlatVcarve(FlatVcarveSettingsV5 {
                components: vec![],
                mode: FlatVcarveMode::EndmillOnly,
                endmill: assignment("endmill"),
                vbit: assignment("vbit"),
                top: Default::default(),
                max_depth_mm: None,
                wall_allowance_mm: None,
                max_floor_ridge_mm: None,
                max_detail_residual_mm: None,
                rough: None,
                finish: None,
            }),
        }],
        tolerances: Default::default(),
        machine_configuration: None,
        legacy_machine_profile: None,
    };
    job.validate_structure().map_err(|e| e.to_string())?;
    if job.to_json().map_err(|e| e.to_string())?.len() > 8_000_000 {
        return Err("Embedded SVG exceeds the 8 MB portable job limit".into());
    }
    Ok(job)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Component {
    pub reference: GeometryRef,
    pub bounds: [f64; 4],
}
pub fn settings_mut(job: &mut CamJobV5) -> &mut FlatVcarveSettingsV5 {
    let OperationSettingsV5::FlatVcarve(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s
}

/// Resource/algorithm policy selected explicitly with Combined mode. Cutting
/// assignments, cutter geometry, stock and machine values remain untouched.
pub fn set_mode(job: &mut CamJobV5, mode: FlatVcarveMode) {
    let s = settings_mut(job);
    s.mode = mode;
    s.finish = match mode {
        FlatVcarveMode::EndmillOnly => None,
        FlatVcarveMode::Combined => Some(s.finish.clone().unwrap_or(
            cam_core::vcarve::VBitPlanningSettings {
                max_paths: 65536,
                max_motions: 100000,
                max_curve_segments: 1000000,
                max_depth_passes: 256,
                max_cleanup_iterations: 2,
                quality_sample_spacing_mm: 1.,
                max_quality_samples: 1000000,
                reachability_max_cells: 100000,
                stock_slices: 8,
            },
        )),
    };
}
pub fn tool(job: &CamJobV5, finishing: bool) -> Option<&JobToolV5> {
    let s = crate::session::settings(job);
    let id = if finishing {
        &s.vbit.tool_id
    } else {
        &s.endmill.tool_id
    };
    job.tools.iter().find(|t| &t.id == id)
}
pub fn tool_mut(job: &mut CamJobV5, finishing: bool) -> Result<&mut JobToolV5, String> {
    let s = crate::session::settings(job);
    let id = if finishing {
        s.vbit.tool_id.clone()
    } else {
        s.endmill.tool_id.clone()
    };
    job.tools
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or("Assignment refers to a missing job tool".into())
}
pub const FIELDS: &[usize] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 25, 26, 27, 28,
    29, 30, 31, 32, 33, 38, 39, 40, 41, 42, 43, 44, 45, 46,
];
pub fn active(job: &CamJobV5, field: usize) -> bool {
    crate::session::settings(job).mode == FlatVcarveMode::Combined
        || !matches!(field, 3 | 5 | 20 | 21 | 22 | 38 | 39 | 46)
}
pub fn value(job: &CamJobV5, field: usize) -> Option<f64> {
    let s = crate::session::settings(job);
    match field {
        7 => job.setup.clearance_above_stock_mm.or_else(|| {
            job.machine_configuration
                .as_ref()
                .and_then(|m| m.clearance_z_mm)
        }),
        11 => s.endmill.spindle_rpm,
        12 | 13 => match &tool(job, false)?.geometry {
            Some(ToolGeometry::Endmill(g)) => Some(if field == 12 {
                g.diameter_mm
            } else {
                g.cutting_length_mm
            }),
            _ => None,
        },
        16..=19 => match &tool(job, true)?.geometry {
            Some(ToolGeometry::Vbit(g)) => Some(match field {
                16 => g.included_angle_deg,
                17 => g.tip_diameter_mm,
                18 => g.max_cutting_diameter_mm,
                _ => g.cutting_height_mm,
            }),
            _ => None,
        },
        20 => s.vbit.max_stepdown_mm,
        21 => s.vbit.plunge_feed_mm_min,
        22 => s.vbit.spindle_rpm,
        46 => s.vbit.stepover_mm,
        23 => job.tolerances.motion_tolerance_mm,
        25 => job.tolerances.verification_tolerance_mm,
        26 => Some(job.artwork[0].placement.origin_mm.x),
        27 => Some(job.artwork[0].placement.origin_mm.y),
        28 => Some(job.artwork[0].placement.rotation_deg),
        29 => Some(job.artwork[0].placement.scale),
        30 => job.setup.start_xy_mm.map(|p| p.x),
        31 => job.setup.start_xy_mm.map(|p| p.y),
        32 | 33 | 38 | 39 => {
            let id = &tool(job, field >= 38)?.id;
            let row = job
                .machine_configuration
                .as_ref()?
                .tools
                .iter()
                .find(|m| &m.job_tool_id == id)?;
            if matches!(field, 32 | 38) {
                row.tool_number
            } else {
                row.length_offset_number
            }
            .map(f64::from)
        }
        40..=43 => job.setup.stock.xy.map(|xy| match field {
            40 => xy.min_x_mm,
            41 => xy.min_y_mm,
            42 => xy.width_mm,
            _ => xy.length_mm,
        }),
        44 | 45 => match job.setup.work_zero.xy {
            WorkZeroXY::CustomPoint { x_mm, y_mm } => Some(if field == 44 { x_mm } else { y_mm }),
            _ => None,
        },
        _ => None,
    }
}

pub fn set(job: &mut CamJobV5, field: usize, v: Option<f64>) -> Result<(), String> {
    match field {
        7 => {
            job.setup.clearance_above_stock_mm = v;
            if let Some(m) = &mut job.machine_configuration {
                m.clearance_z_mm = v;
            }
        }
        11 => settings_mut(job).endmill.spindle_rpm = v,
        20 => settings_mut(job).vbit.max_stepdown_mm = v,
        21 => settings_mut(job).vbit.plunge_feed_mm_min = v,
        22 => settings_mut(job).vbit.spindle_rpm = v,
        46 => settings_mut(job).vbit.stepover_mm = v,
        23 => job.tolerances.motion_tolerance_mm = v,
        25 => job.tolerances.verification_tolerance_mm = v,
        26..=29 => {
            let n = v.ok_or("Placement values cannot be unset")?;
            let p = &mut job.artwork[0].placement;
            match field {
                26 => p.origin_mm.x = n,
                27 => p.origin_mm.y = n,
                28 => p.rotation_deg = n,
                _ => p.scale = n,
            }
        }
        44 | 45 => {
            let n = v.ok_or("Work zero coordinate cannot be unset")?;
            let WorkZeroXY::CustomPoint { x_mm, y_mm } = &mut job.setup.work_zero.xy else {
                return Err("Choose custom work zero first".into());
            };
            if field == 44 { *x_mm = n } else { *y_mm = n }
        }
        32 | 33 | 38 | 39 => {
            let n = v
                .map(|n| {
                    if n >= 0. && n <= u32::MAX as f64 && n.fract() == 0. {
                        Ok(n as u32)
                    } else {
                        Err("Mapping must be a whole nonnegative number")
                    }
                })
                .transpose()?;
            let id = tool(job, field >= 38).ok_or("Missing tool")?.id.clone();
            let row = job
                .machine_configuration
                .as_ref()
                .ok_or("Apply a machine profile first")?
                .tools
                .iter()
                .find(|m| m.job_tool_id == id);
            let t = if matches!(field, 32 | 38) {
                n
            } else {
                row.and_then(|r| r.tool_number)
            };
            let h = if matches!(field, 33 | 39) {
                n
            } else {
                row.and_then(|r| r.length_offset_number)
            };
            *job = machine::set_tool_mapping(job, &id, t, h)
                .map_err(|e| e.to_string())?
                .job;
        }
        _ => return Err("Unknown authoring field".into()),
    }
    Ok(())
}

pub fn group(field: usize) -> &'static [usize] {
    match field {
        12 | 13 => &[12, 13],
        16..=19 => &[16, 17, 18, 19],
        30 | 31 => &[30, 31],
        40..=43 => &[40, 41, 42, 43],
        _ => &[],
    }
}
pub fn set_group(job: &mut CamJobV5, field: usize, values: &[f64]) -> Result<(), String> {
    match field {
        12 | 13 => {
            tool_mut(job, false)?.geometry = Some(ToolGeometry::Endmill(project::EndmillGeometry {
                diameter_mm: values[0],
                cutting_length_mm: values[1],
            }))
        }
        16..=19 => {
            tool_mut(job, true)?.geometry = Some(ToolGeometry::Vbit(cam_core::model::VBitSpec {
                included_angle_deg: values[0],
                tip_diameter_mm: values[1],
                max_cutting_diameter_mm: values[2],
                cutting_height_mm: values[3],
            }))
        }
        30 | 31 => {
            job.setup.start_xy_mm = Some(cam_core::geometry::Point::new(values[0], values[1]))
        }
        40..=43 => {
            job.setup.stock.xy = Some(project::RectXY {
                min_x_mm: values[0],
                min_y_mm: values[1],
                width_mm: values[2],
                length_mm: values[3],
            })
        }
        _ => return Err("Unknown field group".into()),
    }
    Ok(())
}
