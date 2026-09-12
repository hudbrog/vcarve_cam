//! One scene projection for every supported operation, in document order.
//!
//! GUI6 had a milling scene and a separate knife scene because each workspace
//! held exactly one operation. GUI7 orders operations, so the same builder now
//! projects the whole executed prefix: artwork (filled components and knife
//! chains), per-stage motion ranges, cumulative stock checkpoints and the
//! knife pivot/tip detail. Machining meaning still comes only from the core
//! plan; this module decides colors, bounds and what the timeline may show.
use crate::compute::{Package, SimPackage, package, vertex};
use crate::sim::{Motion, Stock, ToolSpec};
use cam_core::project::{
    ToolGeometry,
    v5::{self, CamJobV5, OperationSettingsV5},
};
use cam_core::sequence::{OperationPlanV5, StageRole};
use serde_json::{Value, json};

/// One stage span of the executed prefix, in display order.
#[derive(Clone, Debug)]
pub struct StageSpan {
    pub operation_id: String,
    pub operation_name: String,
    pub stage_id: String,
    pub role: StageRole,
    pub tool_index: usize,
    pub start: usize,
    pub end: usize,
}

impl StageSpan {
    fn role_word(&self) -> &'static str {
        match self.role {
            StageRole::Face => "Face",
            StageRole::VcarveRough => "Endmill",
            StageRole::VcarveFinish => "V-bit",
            StageRole::ProfileRough => "Profile rough",
            StageRole::ProfileFinish => "Profile finish",
            StageRole::Knife => "Knife",
        }
    }
}

/// A timeline control: one stage's visible path range and stock checkpoint.
pub struct Group {
    pub label: String,
    pub jump: String,
    pub start: usize,
    pub end: usize,
    pub tool: usize,
    pub operation: String,
    pub role: &'static str,
}

/// Path labels stay exactly as GUI2–GUI6 published them for a single
/// operation; a sequence qualifies a duplicated role word with the operation
/// name instead of inventing a second vocabulary.
pub fn groups_for(plan: &OperationPlanV5) -> Result<Vec<Group>, String> {
    let spans = stage_spans(plan)?;
    // The established single-operation carving keeps exactly the controls
    // GUI2–GUI6 published, including the V-bit entry in endmill-only mode
    // (where it seeks the same finished prefix as the endmill button).
    let single_carving = plan.operation_results.len() == 1
        && plan
            .job_snapshot
            .operations
            .iter()
            .find(|operation| {
                Some(&operation.id) == plan.operation_results.first().map(|r| &r.operation_id)
            })
            .is_some_and(|operation| {
                matches!(operation.settings, OperationSettingsV5::FlatVcarve(_))
            });
    if single_carving {
        let rough = plan
            .stages
            .iter()
            .find(|stage| stage.role == StageRole::VcarveRough)
            .map_or(0, |stage| stage.motion_range.1);
        let total = plan.motions.len();
        let finish_tool = spans
            .iter()
            .find(|span| span.role == StageRole::VcarveFinish)
            .map_or(0, |span| span.tool_index);
        return Ok(vec![
            Group {
                label: "Endmill paths".into(),
                jump: "After endmill".into(),
                start: 0,
                end: rough,
                tool: 0,
                operation: plan.operation_results[0].operation_id.clone(),
                role: "Endmill",
            },
            Group {
                label: "V-bit paths".into(),
                jump: "After V-bit".into(),
                start: rough,
                end: total,
                tool: finish_tool,
                operation: plan.operation_results[0].operation_id.clone(),
                role: "V-bit",
            },
        ]);
    }
    let mut used_labels: Vec<String> = vec![];
    let mut used_jumps: Vec<String> = vec![];
    let mut groups = vec![];
    for span in &spans {
        let mut label = format!("{} paths", span.role_word());
        if used_labels.contains(&label) {
            label = format!("{} · {}", span.operation_name, span.role_word());
        }
        used_labels.push(label.clone());
        let mut jump = format!("After {}", span.role_word().to_ascii_lowercase());
        if used_jumps.contains(&jump) {
            jump = format!(
                "After {} {}",
                span.operation_name,
                span.role_word().to_ascii_lowercase()
            );
        }
        used_jumps.push(jump.clone());
        groups.push(Group {
            label,
            jump,
            start: span.start,
            end: span.end,
            tool: span.tool_index,
            operation: span.operation_id.clone(),
            role: span.role_word(),
        });
    }
    Ok(groups)
}

/// Stage spans of a plan, with the simulation tool each stage owns.
fn stage_spans(plan: &OperationPlanV5) -> Result<Vec<StageSpan>, String> {
    let mut spans = vec![];
    let mut tools: Vec<String> = vec![];
    for stage in &plan.stages {
        let tool_index = tools
            .iter()
            .position(|id| id == &stage.tool_id)
            .unwrap_or_else(|| {
                tools.push(stage.tool_id.clone());
                tools.len() - 1
            });
        let operation_name = plan
            .job_snapshot
            .operations
            .iter()
            .find(|operation| operation.id == stage.operation_id)
            .map(|operation| operation.name.clone())
            .unwrap_or_else(|| stage.operation_id.clone());
        spans.push(StageSpan {
            operation_id: stage.operation_id.clone(),
            operation_name,
            stage_id: stage.stage_id.clone(),
            role: stage.role,
            tool_index,
            start: stage.motion_range.0,
            end: stage.motion_range.1,
        });
    }
    Ok(spans)
}

/// Selected-by-any-enabled-operation projection of the artwork the ordered
/// plan actually uses. Selection stays owned by the operation; this is a
/// display hint only.
fn artwork_inputs(_job: &CamJobV5) -> Result<v5::CombinedCatalogue, String> {
    v5::artwork::inspect_artwork(_job).map_err(|e| e.to_string())
}

fn selected_references(job: &CamJobV5) -> Vec<v5::GeometryRef> {
    let mut out = vec![];
    for operation in &job.operations {
        match &operation.settings {
            OperationSettingsV5::FlatVcarve(settings) => {
                out.extend(settings.components.iter().cloned());
            }
            OperationSettingsV5::Profile(settings) => {
                out.extend(settings.contours.iter().map(|c| c.geometry.clone()));
            }
            OperationSettingsV5::DragKnife(settings) => {
                out.extend(settings.chains.iter().cloned());
            }
            OperationSettingsV5::Face(_) => {}
        }
    }
    out
}

fn sim_tool(job: &CamJobV5, tool_id: &str) -> Result<ToolSpec, String> {
    let tool = job
        .tools
        .iter()
        .find(|tool| tool.id == tool_id)
        .ok_or_else(|| format!("Plan stage references missing job tool '{tool_id}'"))?;
    match &tool.geometry {
        Some(ToolGeometry::Endmill(geometry)) => Ok(ToolSpec::Endmill {
            diameter: geometry.diameter_mm,
        }),
        Some(ToolGeometry::Vbit(geometry)) => Ok(ToolSpec::Vbit {
            angle: geometry.included_angle_deg,
            tip: geometry.tip_diameter_mm,
            diameter: geometry.max_cutting_diameter_mm,
            height: geometry.cutting_height_mm,
        }),
        Some(ToolGeometry::DragKnife(knife)) => Ok(ToolSpec::Knife {
            offset: knife.blade_offset_mm,
        }),
        None => Err(format!(
            "Generate requires the geometry of every used tool; '{}' has none",
            tool.id
        )),
    }
}

fn role_color(role: StageRole) -> [f32; 4] {
    match role {
        StageRole::Face => [0.16, 0.68, 0.38, 1.],
        StageRole::VcarveRough => [0.19, 0.72, 0.81, 1.],
        StageRole::VcarveFinish => [1., 0.62, 0.2, 1.],
        StageRole::ProfileRough => [0.45, 0.42, 0.86, 1.],
        StageRole::ProfileFinish => [0.85, 0.36, 0.68, 1.],
        StageRole::Knife => [1., 0.62, 0.2, 1.],
    }
}

/// Build the complete scene for `job`; `plan` is the retained execution of the
/// chosen scope, or `None` for the artwork/stock preview before generation.
pub fn build(
    job: &CamJobV5,
    plan: Option<&OperationPlanV5>,
    mut report: Value,
) -> Result<(crate::compute::SceneMeta, Vec<u8>), String> {
    let catalogue = artwork_inputs(job)?;
    let selected = selected_references(job);
    let components = crate::authoring::catalogue_components(&catalogue);
    let chains = crate::knife::chains(job)?;
    let mut bounds = job
        .setup
        .stock
        .xy
        .map(|xy| {
            [
                xy.min_x_mm,
                xy.min_y_mm,
                xy.min_x_mm + xy.width_mm,
                xy.min_y_mm + xy.length_mm,
            ]
        })
        .unwrap_or([0., 0., 1., 1.]);
    for component in &components {
        bounds[0] = bounds[0].min(component.bounds[0]);
        bounds[1] = bounds[1].min(component.bounds[1]);
        bounds[2] = bounds[2].max(component.bounds[2]);
        bounds[3] = bounds[3].max(component.bounds[3]);
    }
    for chain in &chains {
        for point in &chain.vertices {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
        }
    }
    let mut vertices = Vec::new();
    let mut spans = Vec::new();
    for component in &components {
        let start = vertices.len();
        let color = if selected.contains(&component.reference) {
            [0.25, 0.8, 0.85, 1.]
        } else {
            [0.5, 0.55, 0.6, 1.]
        };
        for ring in &component.rings {
            for i in 0..ring.len() {
                for p in [ring[i], ring[(i + 1) % ring.len()]] {
                    vertices.push(vertex([p[0], p[1], 0.02], bounds, color));
                }
            }
        }
        spans.push(json!([
            component.reference.artwork_item_id,
            start,
            vertices.len()
        ]));
    }
    for chain in &chains {
        let start = vertices.len();
        let color = if selected.contains(&chain.reference) {
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
    report["components"] = json!(components);
    report["chains"] = json!(chains);
    report["artworkSpans"] = json!(spans);
    if let Some(xy) = job.setup.stock.xy {
        report["stockRect"] = json!([
            xy.min_x_mm,
            xy.min_y_mm,
            xy.width_mm,
            xy.length_mm,
            job.setup.stock.thickness_mm.unwrap_or(0.)
        ]);
    }
    if report["issues"].is_null() {
        report["issues"] = json!(
            v5::references::inspect_references(job)
                .map_err(|e| e.to_string())?
                .issues
        );
    }
    let contour_vertices = vertices.len();
    let Some(plan) = plan else {
        report["inferredStockXY"] = json!(job.setup.stock.xy.is_none());
        if !vertices.is_empty() {
            bounds = [
                bounds[0] - 2.,
                bounds[1] - 2.,
                bounds[2] + 2.,
                bounds[3] + 2.,
            ];
        }
        return package(Package {
            name: job.name.clone(),
            job: job.to_json().map_err(|e| e.to_string())?,
            report: json!({"gui2":report,"protocol":crate::session::PROTOCOL}),
            programs: vec![],
            bounds,
            contour_vertices,
            rough_vertices: 0,
            vertices,
            preview: None,
            sim: None,
        });
    };
    let spans = stage_spans(plan)?;
    let groups = groups_for(plan)?;
    // An incomplete plan has no executed stage: report the inspection and the
    // located reasons without inventing motion, stock playback or a tool.
    if spans.is_empty() {
        report["inspection"] =
            json!(v5::inspection::inspect_plan(plan).map_err(|e| e.to_string())?);
        report["groups"] = json!([]);
        report["executionFingerprint"] = json!(plan.execution_fingerprint);
        report["roughingMotions"] = json!(0);
        report["finishingMotions"] = json!(0);
        return package(Package {
            name: job.name.clone(),
            job: job.to_json().map_err(|e| e.to_string())?,
            report: json!({"gui2":report,"protocol":crate::session::PROTOCOL,
                "roughingMotions":0,"finishingMotions":0}),
            programs: vec![],
            bounds,
            contour_vertices,
            rough_vertices: 0,
            vertices,
            preview: None,
            sim: None,
        });
    }
    let knife_offset = spans
        .iter()
        .filter(|span| span.role == StageRole::Knife)
        .map(|span| span.start)
        .min();
    let mut tools = Vec::new();
    for span in &spans {
        if tools.len() <= span.tool_index {
            let stage = plan
                .stages
                .iter()
                .find(|stage| stage.stage_id == span.stage_id)
                .ok_or("Missing plan stage")?;
            tools.push(sim_tool(job, &stage.tool_id)?);
        }
    }
    let mut detail = f64::INFINITY;
    let mut radius: f64 = 0.;
    for tool in &tools {
        match *tool {
            ToolSpec::Knife { .. } => {}
            ToolSpec::Endmill { diameter } => {
                radius = radius.max(diameter / 2.);
                detail = detail.min(diameter);
            }
            ToolSpec::Vbit {
                tip,
                diameter,
                height,
                angle,
            } => {
                let slope = (angle / 2.).to_radians().tan();
                radius = radius.max((tip + 2. * height * slope).min(diameter) / 2.);
                detail = detail.min(tip.max(0.2));
                let _ = height;
            }
        }
    }
    if !detail.is_finite() {
        detail = 0.4;
    }
    let thickness = job
        .setup
        .stock
        .thickness_mm
        .ok_or("Generate requires the stock thickness")?;
    let margin = radius + 1.;
    let mut stock = Stock {
        x0: bounds[0] - margin,
        y0: bounds[1] - margin,
        x1: bounds[2] + margin,
        y1: bounds[3] + margin,
        thickness_mm: thickness,
    };
    if let Some(xy) = job.setup.stock.xy {
        stock.x0 = xy.min_x_mm;
        stock.y0 = xy.min_y_mm;
        stock.x1 = xy.min_x_mm + xy.width_mm;
        stock.y1 = xy.min_y_mm + xy.length_mm;
    }
    for motion in &plan.motions {
        for p in [motion.start, motion.end] {
            bounds[0] = bounds[0].min(p.x);
            bounds[1] = bounds[1].min(p.y);
            bounds[2] = bounds[2].max(p.x);
            bounds[3] = bounds[3].max(p.y);
        }
    }
    bounds = [
        bounds[0].min(stock.x0) - 1.,
        bounds[1].min(stock.y0) - 1.,
        bounds[2].max(stock.x1) + 1.,
        bounds[3].max(stock.y1) + 1.,
    ];
    let width = stock.x1 - stock.x0;
    let length = stock.y1 - stock.y0;
    let resolution = crate::sim::choose_resolution(width, length, detail, 8192., 64_000_000.)?;
    let stage_of = |index: usize| -> Option<&StageSpan> {
        spans
            .iter()
            .find(|span| index >= span.start && index < span.end)
    };
    let mut motions = Vec::with_capacity(plan.motions.len());
    for (index, motion) in plan.motions.iter().enumerate() {
        let stage = stage_of(index).ok_or("Plan motion outside every stage")?;
        let cutting = motion.effect == cam_core::toolpath::MotionEffect::MillingSweep;
        motions.push(Motion {
            kind: if cutting { "cut" } else { "rapid_xy" }.into(),
            tool: stage.tool_index,
            x0: motion.start.x,
            y0: motion.start.y,
            z0: motion.start.z,
            x1: motion.end.x,
            y1: motion.end.y,
            z1: motion.end.z,
        });
    }
    for (index, motion) in plan.motions.iter().enumerate() {
        let color = match stage_of(index) {
            Some(stage) => role_color(stage.role),
            None => [0.3, 0.36, 0.44, 0.45],
        };
        let color = if motion.effect == cam_core::toolpath::MotionEffect::MillingSweep
            || motion.effect == cam_core::toolpath::MotionEffect::KnifeTrace
        {
            color
        } else {
            [0.3, 0.36, 0.44, 0.45]
        };
        vertices.push(vertex(
            [motion.start.x, motion.start.y, motion.start.z],
            bounds,
            color,
        ));
        vertices.push(vertex(
            [motion.end.x, motion.end.y, motion.end.z],
            bounds,
            color,
        ));
    }
    let marks: Vec<usize> = spans
        .iter()
        .map(|span| span.end)
        .chain(std::iter::once(0))
        .collect();
    let preview = crate::stock_preview::build_with_marks(
        &crate::sim::Input {
            stock,
            tools: tools.clone(),
            resolution,
            motions: motions.clone(),
            prefixes: vec![],
        },
        &marks,
        crate::stock_preview::CHECKPOINTS,
        crate::stock_preview::MAX_PREVIEW_BYTES,
    )?;
    if let Some(offset) = knife_offset {
        let knife_tools: Vec<Option<&cam_core::project::DragKnifeSpec>> = job
            .tools
            .iter()
            .map(|tool| match &tool.geometry {
                Some(ToolGeometry::DragKnife(knife)) => Some(knife),
                _ => None,
            })
            .collect();
        let detail: Vec<Value> = plan
            .motions
            .iter()
            .enumerate()
            .filter(|(_, motion)| motion.effect == cam_core::toolpath::MotionEffect::KnifeTrace
                || motion.blade_heading_deg.is_some())
            .map(|(index, motion)| {
                let offset_mm = knife_tools
                    .iter()
                    .find_map(|candidate| candidate.as_ref().map(|k| k.blade_offset_mm))
                    .unwrap_or(0.);
                json!({
                    "index": index,
                    "heading": motion.blade_heading_deg,
                    "purpose": motion.purpose,
                    "pass": motion.pass_id,
                    "layer": motion.layer,
                    "start": [motion.start.x, motion.start.y, motion.start.z],
                    "end": [motion.end.x, motion.end.y, motion.end.z],
                    "toolOffsetMm": offset_mm,
                    "tipStart": motion.blade_heading_deg.map(|h| cam_core::toolpath::knife_tip(motion.start.xy(), h.0, offset_mm)),
                    "tipEnd": motion.blade_heading_deg.map(|h| cam_core::toolpath::knife_tip(motion.end.xy(), h.1, offset_mm)),
                    "contact": motion.effect == cam_core::toolpath::MotionEffect::KnifeTrace,
                })
            })
            .collect();
        report["knife"] = json!(true);
        report["knifeOffset"] = json!(offset);
        report["knifeMotions"] = json!(detail);
    }
    let rough = plan
        .stages
        .iter()
        .find(|stage| stage.role == StageRole::VcarveRough)
        .map_or(0, |stage| stage.motion_range.1);
    report["inspection"] = json!(v5::inspection::inspect_plan(plan).map_err(|e| e.to_string())?);
    report["groups"] = json!(
        groups
            .iter()
            .map(|group| json!({
                "label": group.label,
                "jump": group.jump,
                "start": group.start,
                "end": group.end,
                "tool": group.tool,
                "operation": group.operation,
                "role": group.role,
            }))
            .collect::<Vec<_>>()
    );
    report["executionFingerprint"] = json!(plan.execution_fingerprint);
    report["scopeOperation"] = json!(
        plan.operation_results
            .last()
            .map(|result| result.operation_id.clone())
    );
    report["stageRoles"] = json!(
        spans
            .iter()
            .map(|span| json!({
                "operation": span.operation_id,
                "stage": span.stage_id,
                "role": span.role,
                "start": span.start,
                "end": span.end,
                "tool": span.tool_index,
            }))
            .collect::<Vec<_>>()
    );
    let mut report = report;
    report["roughingMotions"] = json!(rough);
    report["finishingMotions"] = json!(plan.motions.len().saturating_sub(rough));
    package(Package {
        name: job.name.clone(),
        job: job.to_json().map_err(|e| e.to_string())?,
        report: json!({"gui2":report,"protocol":crate::session::PROTOCOL,
            "roughingMotions":rough,"finishingMotions":plan.motions.len().saturating_sub(rough)}),
        programs: vec![],
        bounds,
        contour_vertices,
        rough_vertices: rough * 2,
        vertices,
        preview: Some(preview),
        sim: Some(SimPackage {
            stock,
            tools,
            resolution,
            motions,
        }),
    })
}
