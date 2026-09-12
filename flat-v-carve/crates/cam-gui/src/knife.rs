//! Passive-knife GUI adapter. Geometry, compensation and replay stay in core.
use crate::compute::{Package, SceneMeta, SimPackage, package, vertex};
use cam_core::{
    project::{self, v5::*},
    svg::ImportMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn settings(job: &CamJobV5) -> Option<&DragKnifeSettingsV5> {
    match &job.operations.first()?.settings {
        OperationSettingsV5::DragKnife(s) => Some(s),
        _ => None,
    }
}

pub fn interpretation() -> SvgInterpretation {
    SvgInterpretation {
        mode: ImportMode::Centerline,
        ..Default::default()
    }
}

pub fn value(job: &CamJobV5, field: usize) -> Option<f64> {
    let s = settings(job)?;
    let a = &s.assignment;
    match field {
        6 => job.setup.stock.thickness_mm,
        7 => job.setup.clearance_above_stock_mm,
        23 => job.tolerances.motion_tolerance_mm,
        25 => job.tolerances.verification_tolerance_mm,
        26..=29 => {
            let p = &job.artwork.first()?.placement;
            Some(match field {
                26 => p.origin_mm.x,
                27 => p.origin_mm.y,
                28 => p.rotation_deg,
                _ => p.scale,
            })
        }
        30 | 31 => job
            .setup
            .start_xy_mm
            .map(|p| if field == 30 { p.x } else { p.y }),
        32 | 33 => {
            let row = job
                .machine_configuration
                .as_ref()?
                .tools
                .iter()
                .find(|t| t.job_tool_id == a.tool_id)?;
            if field == 32 {
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
            .map(|n| n as f64),
        36 => match job.machine_configuration.as_ref()?.path_control? {
            cam_core::post::PathControl::Blend { tolerance_mm, .. } => Some(tolerance_mm),
            _ => None,
        },
        40..=43 => job.setup.stock.xy.map(|s| match field {
            40 => s.min_x_mm,
            41 => s.min_y_mm,
            42 => s.width_mm,
            _ => s.length_mm,
        }),
        44 | 45 => match job.setup.work_zero.xy {
            project::WorkZeroXY::CustomPoint { x_mm, y_mm } => {
                Some(if field == 44 { x_mm } else { y_mm })
            }
            _ => None,
        },
        61 | 62 => {
            let Some(project::ToolGeometry::DragKnife(t)) =
                &job.tools.iter().find(|t| t.id == a.tool_id)?.geometry
            else {
                return None;
            };
            Some(if field == 61 {
                t.blade_offset_mm
            } else {
                t.max_cut_depth_mm
            })
        }
        63 => a.cutting_feed_mm_min,
        64 => a.plunge_feed_mm_min,
        65 => a.swivel_feed_mm_min,
        66 => a.max_stepdown_mm,
        67 => s.stepdown_mm,
        68 => s.swivel_depth_mm,
        69 => s.corner_threshold_deg,
        70 => s.through_cut_allowance_mm,
        71 => s.closure_overlap_mm,
        72 => s.alignment.initial_heading_deg,
        73 => Some(s.top.offset_mm),
        74 => Some(s.bottom.offset_mm),
        _ => None,
    }
}

pub fn set(job: &mut CamJobV5, field: usize, value: Option<f64>) -> Result<(), String> {
    if field == 6 {
        job.setup.stock.thickness_mm = value;
        return Ok(());
    }
    if field < 61 {
        return crate::authoring::set(job, field, value);
    }
    let OperationSettingsV5::DragKnife(s) = &mut job.operations[0].settings else {
        return Err("Expected knife operation".into());
    };
    let a = &mut s.assignment;
    match field {
        61 | 62 => return Err("Complete both blade dimensions".into()),
        63 => a.cutting_feed_mm_min = value,
        64 => a.plunge_feed_mm_min = value,
        65 => a.swivel_feed_mm_min = value,
        66 => a.max_stepdown_mm = value,
        67 => s.stepdown_mm = value,
        68 => s.swivel_depth_mm = value,
        69 => s.corner_threshold_deg = value,
        70 => s.through_cut_allowance_mm = value,
        71 => s.closure_overlap_mm = value,
        72 => s.alignment.initial_heading_deg = value,
        73 => s.top.offset_mm = value.ok_or("Top offset cannot be unset")?,
        74 => s.bottom.offset_mm = value.ok_or("Bottom offset cannot be unset")?,
        _ => return Err("Unknown knife field".into()),
    }
    Ok(())
}

/// Explicitly chosen new knife job; no cutting, tool or machine preset is invented.
pub fn import_svg(filename: String, svg: String) -> Result<CamJobV5, String> {
    if svg.len() > 8_000_000 {
        return Err("SVG exceeds 8 MB".into());
    }
    let item = ArtworkItem {
        id: ArtworkItemId("artwork-1".into()),
        name: filename.clone(),
        content: ArtworkContent::Svg(cam_core::job::SourceSnapshot {
            filename: filename.clone(),
            svg,
        }),
        import_settings: interpretation(),
        placement: Default::default(),
    };
    let catalogue = artwork::resolve_artwork_item(&item).map_err(|e| e.to_string())?;
    if let Some(error) = catalogue.import_error {
        return Err(error);
    }
    if !catalogue
        .entries
        .iter()
        .any(|e| e.kind == GeometryRefKind::Centerline)
    {
        return Err("SVG has no knife centerlines. Use open or closed paths with a visible stroke and fill=none; filled regions are not knife selections. Convert text to paths first.".into());
    }
    let job = CamJobV5 {
        schema_version: 5,
        name: filename,
        setup: project::SetupSettings {
            stock: project::StockSetup {
                thickness_mm: None,
                xy: Some(crate::authoring::svg_page_stock(&item)?),
            },
            ..Default::default()
        },
        artwork: vec![item],
        tools: vec![JobToolV5 {
            id: "knife-tool".into(),
            name: "Drag knife".into(),
            geometry: None,
            capabilities: Default::default(),
            library_origin: None,
        }],
        operations: vec![OperationV5 {
            id: "knife".into(),
            name: "Drag knife".into(),
            enabled: true,
            settings: OperationSettingsV5::DragKnife(DragKnifeSettingsV5 {
                chains: vec![],
                assignment: KnifeAssignmentV5 {
                    tool_id: "knife-tool".into(),
                    cutting_feed_mm_min: None,
                    plunge_feed_mm_min: None,
                    swivel_feed_mm_min: None,
                    max_stepdown_mm: None,
                    applied_profile: None,
                },
                top: Default::default(),
                bottom: Default::default(),
                stepdown_mm: None,
                swivel_depth_mm: None,
                corner_threshold_deg: None,
                through_cut_allowance_mm: None,
                start: Default::default(),
                closure_overlap_mm: None,
                alignment: Default::default(),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chain {
    pub reference: GeometryRef,
    pub closed: bool,
    pub vertices: Vec<[f64; 2]>,
}
pub fn chains(job: &CamJobV5) -> Result<Vec<Chain>, String> {
    let catalogue = artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    Ok(catalogue
        .items
        .iter()
        .flat_map(|item| {
            item.entries.iter().filter_map(|entry| {
                if entry.kind != GeometryRefKind::Centerline {
                    return None;
                }
                let contour = item
                    .catalogue
                    .as_ref()?
                    .chain(&entry.reference.local_geometry_id)?;
                Some(Chain {
                    reference: entry.reference.clone(),
                    closed: contour.closed,
                    vertices: contour.vertices.iter().map(|p| [p.x, p.y]).collect(),
                })
            })
        })
        .collect())
}

/// No selection is inferred from imports; bind only the explicit current picks.
pub fn select(job: &CamJobV5, references: &[GeometryRef]) -> Result<CamJobV5, String> {
    settings(job).ok_or("Add a knife operation before selecting chains")?;
    let available = chains(job)?;
    if references
        .iter()
        .any(|r| !available.iter().any(|c| &c.reference == r))
    {
        return Err("Artwork changed; select the knife chain again".into());
    }
    let picks = references
        .iter()
        .map(|r| artwork::GeometryPick {
            artwork_item_id: r.artwork_item_id.clone(),
            kind: r.kind,
            local_geometry_id: r.local_geometry_id.clone(),
        })
        .collect::<Vec<_>>();
    commands::set_chain_selection(job, &job.operations[0].id, &picks)
        .map(|o| o.job)
        .map_err(|e| e.to_string())
}

/// Bind a source-length start anchor using the current core catalogue.
pub fn set_start(
    job: &CamJobV5,
    reference: Option<&GeometryRef>,
    fraction: f64,
) -> Result<CamJobV5, String> {
    let s = settings(job).ok_or("Expected knife operation")?;
    let start = if let Some(reference) = reference {
        if !s.chains.contains(reference) {
            return Err("Choose a selected knife chain for the start".into());
        }
        let available = chains(job)?;
        let chain = available
            .iter()
            .find(|c| &c.reference == reference)
            .ok_or("Artwork changed; select the start again")?;
        if !chain.closed && fraction != 0. {
            return Err("Open chains start at their source endpoint".into());
        }
        let item = job
            .artwork
            .iter()
            .find(|i| i.id == reference.artwork_item_id)
            .ok_or("Missing artwork")?;
        let catalogue = artwork::item_catalogue(item).map_err(|e| e.to_string())?;
        let contour = catalogue
            .chain(&reference.local_geometry_id)
            .ok_or("Missing chain")?;
        StartSelectionV5::Anchor(Box::new(ContourAnchorV5 {
            geometry: reference.clone(),
            source_geometry_fingerprint: contour.source_fingerprint.clone(),
            fraction_along_source_contour: fraction,
        }))
    } else {
        StartSelectionV5::Automatic
    };
    let mut candidate = job.clone();
    let OperationSettingsV5::DragKnife(s) = &mut candidate.operations[0].settings else {
        unreachable!()
    };
    s.start = start;
    candidate.validate_structure().map_err(|e| e.to_string())?;
    Ok(candidate)
}

/// Display geometry uses core chain vertices and modeled headings. Emitted
/// replay arrives separately from the exact checked output bundle.
pub fn scene(
    job: &CamJobV5,
    plan: Option<&cam_core::sequence::OperationPlanV5>,
    mut report: Value,
) -> Result<(SceneMeta, Vec<u8>), String> {
    let settings = settings(job).ok_or("Expected knife operation")?;
    let chains = chains(job)?;
    let xy = job.setup.stock.xy;
    let mut bounds = xy
        .map(|s| {
            [
                s.min_x_mm,
                s.min_y_mm,
                s.min_x_mm + s.width_mm,
                s.min_y_mm + s.length_mm,
            ]
        })
        .unwrap_or([0., 0., 1., 1.]);
    for p in chains.iter().flat_map(|c| c.vertices.iter()) {
        bounds[0] = bounds[0].min(p[0]);
        bounds[1] = bounds[1].min(p[1]);
        bounds[2] = bounds[2].max(p[0]);
        bounds[3] = bounds[3].max(p[1]);
    }
    if let Some(plan) = plan {
        for m in &plan.motions {
            for p in [m.start, m.end] {
                bounds[0] = bounds[0].min(p.x);
                bounds[1] = bounds[1].min(p.y);
                bounds[2] = bounds[2].max(p.x);
                bounds[3] = bounds[3].max(p.y);
            }
        }
    }
    let mut vertices = vec![];
    let mut spans = vec![];
    for chain in &chains {
        let start = vertices.len();
        let color = if settings.chains.contains(&chain.reference) {
            [0.25, 0.8, 0.85, 1.]
        } else {
            [0.5, 0.55, 0.6, 1.]
        };
        let segments = chain
            .vertices
            .len()
            .saturating_sub(usize::from(!chain.closed));
        for i in 0..segments {
            for p in [
                chain.vertices[i],
                chain.vertices[(i + 1) % chain.vertices.len()],
            ] {
                vertices.push(vertex([p[0], p[1], 0.02], bounds, color));
            }
        }
        spans.push(json!([
            chain.reference.artwork_item_id,
            start,
            vertices.len()
        ]));
    }
    report["chains"] = json!(chains);
    report["knifeSelection"] = json!(settings.chains);
    report["components"] = json!([]);
    report["knife"] = json!(true);
    report["artworkSpans"] = json!(spans);
    if report["issues"].is_null() {
        report["issues"] = json!(
            references::inspect_references(job)
                .map_err(|e| e.to_string())?
                .issues
        );
    }
    if let Some(xy) = xy {
        report["stockRect"] = json!([
            xy.min_x_mm,
            xy.min_y_mm,
            xy.width_mm,
            xy.length_mm,
            job.setup.stock.thickness_mm.unwrap_or(0.)
        ]);
    }
    let contour_vertices = vertices.len();
    let mut sim = None;
    let mut preview = None;
    if let Some(plan) = plan {
        let tool = job
            .tools
            .iter()
            .find(|t| t.id == settings.assignment.tool_id)
            .ok_or("Missing knife tool")?;
        let Some(project::ToolGeometry::DragKnife(tool)) = &tool.geometry else {
            return Err("Missing knife geometry".into());
        };
        let xy = xy.ok_or("Knife playback needs explicit stock bounds")?;
        let stock = crate::sim::Stock {
            x0: xy.min_x_mm,
            y0: xy.min_y_mm,
            x1: xy.min_x_mm + xy.width_mm,
            y1: xy.min_y_mm + xy.length_mm,
            thickness_mm: job
                .setup
                .stock
                .thickness_mm
                .ok_or("Missing stock thickness")?,
        };
        let tools = vec![crate::sim::ToolSpec::Knife {
            offset: tool.blade_offset_mm,
        }];
        let resolution =
            crate::sim::choose_resolution(xy.width_mm, xy.length_mm, 0.4, 8192., 64_000_000.)?;
        let motions = plan
            .motions
            .iter()
            .map(|m| crate::sim::Motion {
                kind: "rapid_xy".into(),
                tool: 0,
                x0: m.start.x,
                y0: m.start.y,
                z0: m.start.z,
                x1: m.end.x,
                y1: m.end.y,
                z1: m.end.z,
            })
            .collect::<Vec<_>>();
        report["inspection"] = json!(inspection::inspect_plan(plan).map_err(|e| e.to_string())?);
        report["knifeMotions"] = json!(plan.motions.iter().map(|m| json!({
            "heading":m.blade_heading_deg,"purpose":m.purpose,"pass":m.pass_id,"layer":m.layer,
            "start":[m.start.x,m.start.y,m.start.z],"end":[m.end.x,m.end.y,m.end.z],
            "tipStart":m.blade_heading_deg.map(|h|cam_core::toolpath::knife_tip(m.start.xy(),h.0,tool.blade_offset_mm)),
            "tipEnd":m.blade_heading_deg.map(|h|cam_core::toolpath::knife_tip(m.end.xy(),h.1,tool.blade_offset_mm)),
            "contact":m.effect == cam_core::toolpath::MotionEffect::KnifeTrace,
        })).collect::<Vec<_>>());
        for m in &plan.motions {
            let color = if m.effect == cam_core::toolpath::MotionEffect::KnifeTrace {
                [1., 0.62, 0.2, 1.]
            } else {
                [0.3, 0.36, 0.44, 0.45]
            };
            for p in [m.start, m.end] {
                vertices.push(vertex([p.x, p.y, p.z], bounds, color));
            }
        }
        preview = Some(crate::stock_preview::build(
            &crate::sim::Input {
                stock,
                tools: tools.clone(),
                resolution,
                motions: motions.clone(),
                prefixes: vec![],
            },
            0,
        )?);
        sim = Some(SimPackage {
            stock,
            tools,
            resolution,
            motions,
        });
    }
    // Bounded detail is separate from the paged pivot geometry. Refuse an
    // oversized inspection artifact explicitly rather than truncate a job.
    if serde_json::to_vec(&report)
        .map_err(|e| e.to_string())?
        .len()
        > 16_000_000
    {
        return Err(
            "Knife inspection exceeds the 16 MB display-detail budget; reduce the selected scope"
                .into(),
        );
    }
    package(Package {
        name: job.name.clone(),
        job: job.to_json().map_err(|e| e.to_string())?,
        report: json!({"gui2":report,"protocol":crate::session::PROTOCOL,"roughingMotions":0,"finishingMotions":0}),
        programs: vec![],
        bounds,
        contour_vertices,
        rough_vertices: 0,
        vertices,
        preview,
        sim,
    })
}
