//! Artwork and cutting authoring over the canonical model.
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
        setup: project::SetupSettings {
            stock: project::StockSetup {
                thickness_mm: Some(18.),
                xy: Some(svg_page_stock(&item)?),
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(cam_core::geometry::Point::new(0., 0.)),
            ..Default::default()
        },
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
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    };
    job.validate_structure().map_err(|e| e.to_string())?;
    if job.to_json().map_err(|e| e.to_string())?.len() > 8_000_000 {
        return Err("Embedded SVG exceeds the 8 MB portable job limit".into());
    }
    Ok(job)
}

/// Capture the selected SVG's full page, including its placement, rather than
/// the smaller bounds of the paths drawn on it. Runs on the compute worker.
pub fn svg_page_stock(item: &ArtworkItem) -> Result<project::RectXY, String> {
    let ArtworkContent::Svg(source) = &item.content;
    let geometry = cam_core::svg::import_svg(&source.svg, &item.import_options(), None)
        .map_err(|e| e.to_string())?;
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for (x, y) in [
        (0., 0.),
        (geometry.page_width_mm, 0.),
        (0., geometry.page_height_mm),
        (geometry.page_width_mm, geometry.page_height_mm),
    ] {
        let p = item
            .placement
            .to_setup(cam_core::geometry::Point { x, y })
            .map_err(|e| e.to_string())?;
        min[0] = min[0].min(p.x);
        min[1] = min[1].min(p.y);
        max[0] = max[0].max(p.x);
        max[1] = max[1].max(p.y);
    }
    Ok(project::RectXY {
        min_x_mm: min[0],
        min_y_mm: min[1],
        width_mm: max[0] - min[0],
        length_mm: max[1] - min[1],
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Component {
    pub reference: GeometryRef,
    pub bounds: [f64; 4],
    /// Canonical imported boundaries in setup millimeters, including holes.
    pub rings: Vec<Vec<[f64; 2]>>,
}
pub fn catalogue_components(catalogue: &artwork::CombinedCatalogue) -> Vec<Component> {
    catalogue
        .items
        .iter()
        .flat_map(|item| {
            item.entries
                .iter()
                .filter(|e| e.kind == GeometryRefKind::FilledComponent)
                .map(|entry| Component {
                    reference: entry.reference.clone(),
                    bounds: [
                        entry.bounds.min_x_mm,
                        entry.bounds.min_y_mm,
                        entry.bounds.max_x_mm,
                        entry.bounds.max_y_mm,
                    ],
                    rings: item
                        .catalogue
                        .as_ref()
                        .map(|catalogue| {
                            catalogue
                                .contours
                                .iter()
                                .filter(|c| c.component_id == entry.reference.local_geometry_id)
                                .map(|c| c.vertices.iter().map(|p| [p.x, p.y]).collect())
                                .collect()
                        })
                        .unwrap_or_default(),
                })
        })
        .collect()
}
/// The Flat V-carve settings of one explicit operation.
pub fn settings_mut_in<'a>(
    job: &'a mut CamJobV5,
    operation_id: &str,
) -> Option<&'a mut FlatVcarveSettingsV5> {
    crate::session::operation_mut(job, operation_id).and_then(|operation| {
        match &mut operation.settings {
            OperationSettingsV5::FlatVcarve(s) => Some(s),
            _ => None,
        }
    })
}

/// The first Flat V-carve operation. Established GUI2–GUI6 call sites use
/// this; GUI7 code addresses an explicit operation ID.
pub fn settings_mut(job: &mut CamJobV5) -> &mut FlatVcarveSettingsV5 {
    let id = job
        .operations
        .iter()
        .find(|operation| matches!(operation.settings, OperationSettingsV5::FlatVcarve(_)))
        .map(|operation| operation.id.clone())
        .expect("job carries a Flat V-carve operation");
    settings_mut_in(job, &id).expect("operation is a Flat V-carve")
}

/// The explicit picks a selection command sends for a stored reference list.
/// References are never inferred from names or positions: the command still
/// binds every pick to its item's current source revision.
pub fn picks(references: &[GeometryRef]) -> Vec<artwork::GeometryPick> {
    references
        .iter()
        .map(|reference| artwork::GeometryPick {
            artwork_item_id: reference.artwork_item_id.clone(),
            kind: reference.kind,
            local_geometry_id: reference.local_geometry_id.clone(),
        })
        .collect()
}

/// Resource/algorithm policy selected explicitly with Combined mode. Cutting
/// assignments, cutter geometry, stock and machine values remain untouched.
pub fn set_mode(job: &mut CamJobV5, mode: FlatVcarveMode) {
    let id = job
        .operations
        .iter()
        .find(|operation| matches!(operation.settings, OperationSettingsV5::FlatVcarve(_)))
        .map(|operation| operation.id.clone())
        .expect("job carries a Flat V-carve operation");
    set_mode_in(job, &id, mode);
}

pub fn set_mode_in(job: &mut CamJobV5, operation_id: &str, mode: FlatVcarveMode) {
    let Some(s) = settings_mut_in(job, operation_id) else {
        return;
    };
    s.mode = mode;
    s.finish = match mode {
        FlatVcarveMode::EndmillOnly => s.finish.clone(),
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

/// The job tool one operation's assignment names.
pub fn tool_in<'a>(
    job: &'a CamJobV5,
    operation_id: &str,
    finishing: bool,
) -> Option<&'a JobToolV5> {
    let id = match crate::session::kind(job, operation_id)? {
        crate::session::OperationKind::FlatVcarve => {
            let s = crate::session::settings_in(job, operation_id)?;
            if finishing {
                &s.vbit.tool_id
            } else {
                &s.endmill.tool_id
            }
        }
        crate::session::OperationKind::Face => {
            if finishing {
                return None;
            }
            &crate::session::face(job, operation_id)?.assignment.tool_id
        }
        crate::session::OperationKind::Profile => {
            if finishing {
                return None;
            }
            match &crate::session::operation(job, operation_id)?.settings {
                OperationSettingsV5::Profile(s) => &s.assignment.tool_id,
                _ => return None,
            }
        }
        crate::session::OperationKind::DragKnife => {
            if finishing {
                return None;
            }
            &crate::knife::settings_in(job, operation_id)?
                .assignment
                .tool_id
        }
    };
    job.tools.iter().find(|tool| &tool.id == id)
}

pub fn tool(job: &CamJobV5, finishing: bool) -> Option<&JobToolV5> {
    tool_in(job, &job.operations.first()?.id, finishing)
}

pub fn clear_assignment_in(job: &mut CamJobV5, operation_id: &str, finishing: bool) {
    use crate::session::OperationKind;
    match crate::session::kind(job, operation_id) {
        Some(OperationKind::Face) => {
            if let Some(face) = crate::session::face_mut(job, operation_id) {
                clear_milling(&mut face.assignment);
            }
            return;
        }
        Some(OperationKind::Profile) => {
            if let Some(operation) = crate::session::operation_mut(job, operation_id)
                && let OperationSettingsV5::Profile(profile) = &mut operation.settings
            {
                clear_milling(&mut profile.assignment);
            }
            return;
        }
        Some(OperationKind::DragKnife) => {
            crate::knife::clear_assignment_in(job, operation_id);
            return;
        }
        _ => {}
    }
    let Some(s) = settings_mut_in(job, operation_id) else {
        return;
    };
    let a = if finishing {
        &mut s.vbit
    } else {
        &mut s.endmill
    };
    clear_milling(a);
}

fn clear_milling(a: &mut MillingAssignmentV5) {
    a.spindle_rpm = None;
    a.spindle_direction = None;
    a.cutting_feed_mm_min = None;
    a.plunge_feed_mm_min = None;
    a.max_stepdown_mm = None;
    a.stepover_mm = None;
    a.applied_profile = None;
}

pub fn clear_assignment(job: &mut CamJobV5, finishing: bool) {
    let id = job
        .operations
        .first()
        .map(|operation| operation.id.clone())
        .unwrap_or_default();
    clear_assignment_in(job, &id, finishing);
}

pub fn assign_tool_in(
    job: &mut CamJobV5,
    operation_id: &str,
    finishing: bool,
    id: &str,
) -> Result<(), String> {
    use crate::session::OperationKind;
    let tool = job
        .tools
        .iter()
        .find(|t| t.id == id)
        .ok_or("Unknown job tool")?;
    if !matches!(
        (&tool.geometry, finishing),
        (None, _) | (Some(ToolGeometry::Endmill(_)), false) | (Some(ToolGeometry::Vbit(_)), true)
    ) {
        return Err("Tool geometry does not match this assignment".into());
    }
    match crate::session::kind(job, operation_id) {
        Some(OperationKind::Face) => {
            let face = crate::session::face_mut(job, operation_id).ok_or("Expected Face")?;
            if face.assignment.tool_id == id {
                return Ok(());
            }
            face.assignment.tool_id = id.into();
            clear_milling(&mut face.assignment);
            return Ok(());
        }
        Some(OperationKind::Profile) => {
            let profile =
                crate::profile::settings_in_mut(job, operation_id).ok_or("Expected Profile")?;
            if profile.assignment.tool_id == id {
                return Ok(());
            }
            profile.assignment.tool_id = id.into();
            clear_milling(&mut profile.assignment);
            return Ok(());
        }
        Some(OperationKind::DragKnife) => {
            return crate::knife::assign_tool_in(job, operation_id, id);
        }
        _ => {}
    }
    let Some(s) = settings_mut_in(job, operation_id) else {
        return Err("The selected operation has no milling assignment".into());
    };
    let a = if finishing {
        &mut s.vbit
    } else {
        &mut s.endmill
    };
    if a.tool_id == id {
        return Ok(());
    }
    a.tool_id = id.into();
    clear_milling(a);
    Ok(())
}

pub fn assign_tool(job: &mut CamJobV5, finishing: bool, id: &str) -> Result<(), String> {
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.clone())
        .unwrap_or_default();
    assign_tool_in(job, &operation_id, finishing, id)
}

pub fn tool_mut_in<'a>(
    job: &'a mut CamJobV5,
    operation_id: &str,
    finishing: bool,
) -> Result<&'a mut JobToolV5, String> {
    let id = tool_in(job, operation_id, finishing)
        .ok_or("This operation has no such assignment")?
        .id
        .clone();
    job.tools
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or("Assignment refers to a missing job tool".into())
}

pub fn tool_mut(job: &mut CamJobV5, finishing: bool) -> Result<&mut JobToolV5, String> {
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.clone())
        .unwrap_or_default();
    tool_mut_in(job, &operation_id, finishing)
}
pub const FIELDS: &[usize] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 25, 26, 27, 28,
    29, 30, 31, 32, 33, 38, 39, 40, 41, 42, 43, 44, 45, 46, 14, 47, 48, 49, 50, 51, 52, 53, 54, 55,
    56, 57, 58, 59, 60, 34, 35, 36, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76,
    77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
    100, 101, 102, 103, 104, 105, 106, 107, 108,
];

/// Face-editor field IDs (GUI7a/b). Reused IDs 2/8/9/10/11 are the operation's
/// own assignment values, exactly as they are for a Flat V-carve operation.
pub fn active(job: &CamJobV5, field: usize) -> bool {
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.as_str())
        .unwrap_or("");
    active_in(job, operation_id, field)
}

pub fn active_in(job: &CamJobV5, operation_id: &str, field: usize) -> bool {
    if job.operations.is_empty() || operation_id.is_empty() {
        return matches!(field, 6 | 7 | 23 | 25..=31 | 34..=36 | 40..=45);
    }
    match crate::session::kind(job, operation_id) {
        Some(crate::session::OperationKind::DragKnife) => {
            return matches!(field, 6 | 7 | 23 | 25..=36 | 40..=45 | 61..=74);
        }
        Some(crate::session::OperationKind::Face) => return crate::face::active(field),
        Some(crate::session::OperationKind::Profile) => return crate::profile::active(field),
        _ => {}
    }
    if field >= 61 {
        return false;
    }
    let Some(s) = crate::session::settings_in(job, operation_id) else {
        return matches!(field, 6 | 7 | 23 | 25..=31 | 34..=36 | 40..=45);
    };
    if matches!(field, 33 | 39) {
        return (field == 33 || s.mode == FlatVcarveMode::Combined)
            && job.machine_configuration.as_ref().is_some_and(|m| {
                m.length_compensation == Some(cam_core::post::LengthCompensation::ToolTable)
            });
    }
    if matches!(field, 34..=36) {
        return job.machine_configuration.as_ref().is_some_and(|m| {
            field != 36
                || matches!(
                    m.path_control,
                    Some(cam_core::post::PathControl::Blend { .. })
                )
        });
    }
    if matches!(field, 14 | 51) {
        return s
            .rough
            .as_ref()
            .is_some_and(|r| matches!(r.entry, cam_core::pocket::EntryStrategy::Ramp { .. }));
    }
    if matches!(field, 48..=50) {
        return s.rough.is_some();
    }
    s.mode == FlatVcarveMode::Combined
        || !matches!(field, 3 | 4 | 5 | 20 | 21 | 22 | 38 | 39 | 46 | 52..=60)
}
pub fn value(job: &CamJobV5, field: usize) -> Option<f64> {
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.as_str())
        .unwrap_or("");
    value_in(job, operation_id, field)
}

pub fn value_in(job: &CamJobV5, operation_id: &str, field: usize) -> Option<f64> {
    // Shared setup/machine fields plus the Flat V-carve operation fields. The
    // caller dispatches Face and drag-knife fields to their own modules, which
    // fall back to this function for everything they do not own.
    let s = crate::session::settings_in(job, operation_id);
    match field {
        47 => Some(s?.top.offset_mm),
        48 => Some(s?.rough.as_ref()?.max_layers as f64),
        49 => Some(s?.rough.as_ref()?.max_loops_per_layer as f64),
        50 => Some(s?.rough.as_ref()?.max_motions as f64),
        14 | 51 => match s?.rough.as_ref()?.entry {
            cam_core::pocket::EntryStrategy::Ramp {
                max_angle_deg,
                feed_mm_min,
            } => Some(if field == 14 {
                max_angle_deg
            } else {
                feed_mm_min
            }),
            _ => None,
        },
        52..=60 => {
            let f = s?.finish.as_ref()?;
            Some(match field {
                52 => f.max_paths as f64,
                53 => f.max_motions as f64,
                54 => f.max_curve_segments as f64,
                55 => f.max_depth_passes as f64,
                56 => f.max_cleanup_iterations as f64,
                57 => f.quality_sample_spacing_mm,
                58 => f.max_quality_samples as f64,
                59 => f.reachability_max_cells as f64,
                _ => f.stock_slices as f64,
            })
        }
        7 => job.setup.clearance_above_stock_mm.or_else(|| {
            job.machine_configuration
                .as_ref()
                .and_then(|m| m.clearance_z_mm)
        }),
        6 => job.setup.stock.thickness_mm,
        11 => s?.endmill.spindle_rpm,
        12 | 13 => match &tool_in(job, operation_id, false)?.geometry {
            Some(ToolGeometry::Endmill(g)) => Some(if field == 12 {
                g.diameter_mm
            } else {
                g.cutting_length_mm
            }),
            _ => None,
        },
        16..=19 => match &tool_in(job, operation_id, true)?.geometry {
            Some(ToolGeometry::Vbit(g)) => Some(match field {
                16 => g.included_angle_deg,
                17 => g.tip_diameter_mm,
                18 => g.max_cutting_diameter_mm,
                _ => g.cutting_height_mm,
            }),
            _ => None,
        },
        20 => s?.vbit.max_stepdown_mm,
        21 => s?.vbit.plunge_feed_mm_min,
        22 => s?.vbit.spindle_rpm,
        46 => s?.vbit.stepover_mm,
        23 => job.tolerances.motion_tolerance_mm,
        25 => job.tolerances.verification_tolerance_mm,
        26 => Some(job.artwork.first()?.placement.origin_mm.x),
        27 => Some(job.artwork.first()?.placement.origin_mm.y),
        28 => Some(job.artwork.first()?.placement.rotation_deg),
        29 => Some(job.artwork.first()?.placement.scale),
        30 => job.setup.start_xy_mm.map(|p| p.x),
        31 => job.setup.start_xy_mm.map(|p| p.y),
        32 | 33 | 38 | 39 => {
            let id = &tool_in(job, operation_id, field >= 38)?.id;
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
        34 => job.machine_configuration.as_ref()?.spindle_spinup_seconds,
        35 => job
            .machine_configuration
            .as_ref()?
            .decimal_places
            .map(|v| v as f64),
        36 => match job.machine_configuration.as_ref()?.path_control? {
            cam_core::post::PathControl::Blend { tolerance_mm, .. } => Some(tolerance_mm),
            _ => None,
        },
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
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.clone())
        .unwrap_or_default();
    set_in(job, &operation_id, field, v)
}

pub fn set_in(
    job: &mut CamJobV5,
    operation_id: &str,
    field: usize,
    v: Option<f64>,
) -> Result<(), String> {
    match field {
        34..=36 => {
            let m = job
                .machine_configuration
                .as_mut()
                .ok_or("Apply a machine configuration first")?;
            match field {
                34 => m.spindle_spinup_seconds = v,
                35 => {
                    m.decimal_places = match v {
                        None => None,
                        Some(n) if n.fract() == 0. && (0.0..=9.).contains(&n) => Some(n as usize),
                        _ => return Err("Precision must be an integer from 0 to 9".into()),
                    }
                }
                _ => {
                    let cam_core::post::PathControl::Blend { tolerance_mm, .. } =
                        m.path_control.as_mut().ok_or("Choose blend path control")?
                    else {
                        return Err("Choose blend path control".into());
                    };
                    *tolerance_mm = v.ok_or("Blend tolerance cannot be unset")?;
                }
            }
        }
        47 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .top
                .offset_mm = v.ok_or("Top offset cannot be unset")?
        }
        48..=50 => {
            let n = whole(v)?;
            let r = settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .rough
                .as_mut()
                .ok_or("Choose a clearing strategy first")?;
            match field {
                48 => r.max_layers = n,
                49 => r.max_loops_per_layer = n,
                _ => r.max_motions = n,
            }
        }
        52..=60 => {
            let f = settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .finish
                .as_mut()
                .ok_or("Choose Combined mode first")?;
            if field == 57 {
                f.quality_sample_spacing_mm = v.ok_or("Sample spacing cannot be unset")?;
            } else {
                let n = whole(v)?;
                match field {
                    52 => f.max_paths = n,
                    53 => f.max_motions = n,
                    54 => f.max_curve_segments = n,
                    55 => f.max_depth_passes = n,
                    56 => f.max_cleanup_iterations = n,
                    58 => f.max_quality_samples = n,
                    59 => f.reachability_max_cells = n,
                    _ => f.stock_slices = n,
                }
            }
        }
        7 => {
            job.setup.clearance_above_stock_mm = v;
            if let Some(m) = &mut job.machine_configuration {
                m.clearance_z_mm = v;
            }
        }
        11 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .endmill
                .spindle_rpm = v
        }
        20 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .vbit
                .max_stepdown_mm = v
        }
        21 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .vbit
                .plunge_feed_mm_min = v
        }
        22 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .vbit
                .spindle_rpm = v
        }
        46 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .vbit
                .stepover_mm = v
        }
        23 => job.tolerances.motion_tolerance_mm = v,
        25 => job.tolerances.verification_tolerance_mm = v,
        26..=29 => {
            let n = v.ok_or("Placement values cannot be unset")?;
            let p = &mut job
                .artwork
                .first_mut()
                .ok_or("No artwork selected")?
                .placement;
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
            let id = tool_in(job, operation_id, field >= 38)
                .ok_or("Missing tool")?
                .id
                .clone();
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

fn whole(v: Option<f64>) -> Result<usize, String> {
    let n = v.ok_or("Planner limits cannot be unset")?;
    if !n.is_finite() || !(0.0..=1_000_000.0).contains(&n) || n.fract() != 0. {
        return Err("Planner limits require a whole number in the supported range".into());
    }
    Ok(n as usize)
}

pub fn group(field: usize) -> &'static [usize] {
    match field {
        61 | 62 => &[61, 62],
        14 | 51 => &[14, 51],
        12 | 13 => &[12, 13],
        16..=19 => &[16, 17, 18, 19],
        30 | 31 => &[30, 31],
        40..=43 => &[40, 41, 42, 43],
        _ => &[],
    }
}
pub fn set_group(job: &mut CamJobV5, field: usize, values: &[f64]) -> Result<(), String> {
    let operation_id = job
        .operations
        .first()
        .map(|operation| operation.id.clone())
        .unwrap_or_default();
    set_group_in(job, &operation_id, field, values)
}

pub fn set_group_in(
    job: &mut CamJobV5,
    operation_id: &str,
    field: usize,
    values: &[f64],
) -> Result<(), String> {
    if matches!(field, 82..=85)
        && crate::session::kind(job, operation_id) == Some(crate::session::OperationKind::Face)
    {
        return crate::face::set_group(job, operation_id, field, values);
    }
    match field {
        61 | 62 => {
            let id = crate::knife::settings_in(job, operation_id)
                .ok_or("Expected knife operation")?
                .assignment
                .tool_id
                .clone();
            job.tools
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Missing knife tool")?
                .geometry = Some(ToolGeometry::DragKnife(project::DragKnifeSpec {
                blade_offset_mm: values[0],
                max_cut_depth_mm: values[1],
            }));
        }
        14 | 51 => {
            settings_mut_in(job, operation_id)
                .ok_or("This operation is not a Flat V-carve")?
                .rough
                .as_mut()
                .ok_or("Choose a clearing strategy first")?
                .entry = cam_core::pocket::EntryStrategy::Ramp {
                max_angle_deg: values[0],
                feed_mm_min: values[1],
            };
        }
        12 | 13 => {
            tool_mut_in(job, operation_id, false)?.geometry =
                Some(ToolGeometry::Endmill(project::EndmillGeometry {
                    diameter_mm: values[0],
                    cutting_length_mm: values[1],
                }))
        }
        16..=19 => {
            tool_mut_in(job, operation_id, true)?.geometry =
                Some(ToolGeometry::Vbit(cam_core::model::VBitSpec {
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
