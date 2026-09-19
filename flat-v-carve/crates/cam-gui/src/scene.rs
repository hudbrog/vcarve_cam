//! One scene projection for every supported operation, in document order.
//!
//! GUI6 had a milling scene and a separate knife scene because each workspace
//! held exactly one operation. GUI7 orders operations, so the same builder now
//! projects the whole executed prefix: artwork (filled components and knife
//! chains), per-stage motion ranges, cumulative stock checkpoints and the
//! knife pivot/tip detail. Machining meaning still comes only from the core
//! plan; this module decides colors, bounds and what the timeline may show.
use crate::compute::{Package, SimPackage, package, vertex};
use crate::sim::{Stock, ToolSpec};
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
            StageRole::Drill => "Drill",
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

/// Colour of a travel move, in the motion stream the viewport draws. Its alpha
/// is also the display's marker for "this vertex is a travel move": the path
/// filters in `scene.wgsl` split travel from cutting by it, so a cutting colour
/// must stay opaque.
pub const TRAVEL_COLOR: [f32; 4] = [0.3, 0.36, 0.44, 0.45];

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
    // A role that repeats (a profile cuts every contour's rough and finishing
    // work in turn) is numbered rather than renamed after its operation, so the
    // timeline reads in execution order and each checkpoint stays distinct.
    let mut totals: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for span in &spans {
        *totals.entry(span.role_word()).or_default() += 1;
    }
    let mut seen: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut groups = vec![];
    for span in &spans {
        let word = span.role_word();
        let ordinal = {
            let entry = seen.entry(word).or_default();
            *entry += 1;
            *entry
        };
        let total = totals[word];
        let (label, jump) = if total > 1 {
            (
                format!("{word} paths ({ordinal} of {total})"),
                format!("After {} ({ordinal} of {total})", word.to_ascii_lowercase()),
            )
        } else {
            (
                format!("{word} paths"),
                format!("After {}", word.to_ascii_lowercase()),
            )
        };
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
            OperationSettingsV5::Drill(settings) => {
                out.extend(settings.points.iter().cloned());
            }
            OperationSettingsV5::Face(_) => {}
        }
    }
    out
}

/// How a job tool is held: the shaft above the cutter and its stickout. A tool
/// that states neither contributes a default (empty) assembly, which is what
/// every job saved before these fields existed says too.
pub(crate) fn sim_assembly(job: &CamJobV5, tool_id: &str) -> cam_core::project::ToolAssembly {
    job.tools
        .iter()
        .find(|tool| tool.id == tool_id)
        .map_or_else(Default::default, |tool| tool.assembly)
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
            cutting_length: geometry.cutting_length_mm,
        }),
        Some(ToolGeometry::Vbit(geometry)) => Ok(ToolSpec::Vbit {
            angle: geometry.included_angle_deg,
            tip: geometry.tip_diameter_mm,
            diameter: geometry.max_cutting_diameter_mm,
            height: geometry.cutting_height_mm,
        }),
        Some(ToolGeometry::DragKnife(knife)) => Ok(ToolSpec::Knife {
            offset: knife.blade_offset_mm,
            max_cut_depth: knife.max_cut_depth_mm,
        }),
        // The simulation draws the drill as its swept cylinder (the conical
        // point is not modelled yet — same v1 approximation as the stock
        // history's drill cutter).
        Some(ToolGeometry::Drill(geometry)) => Ok(ToolSpec::Endmill {
            diameter: geometry.diameter_mm,
            cutting_length: geometry.cutting_length_mm,
        }),
        None => Err(format!(
            "Generate requires the geometry of every used tool; '{}' has none",
            tool.id
        )),
    }
}

/// The display frame and the simulation inputs an executed plan contributes:
/// the tools its stages use, the simulated stock rectangle and the bounds that
/// include every motion. It is built before any vertex is normalized so the
/// artwork, the stock and the toolpath share one rectangle (plan section
/// 15.1). A toolpath that reaches outside the stock therefore widens the
/// frame instead of rescaling the artwork drawn inside it — the 2026-09-13
/// field report's "the imported SVG renders larger than the stock" once a
/// facing operation was generated.
struct ExecutedFrame {
    bounds: [f64; 4],
    stock: Stock,
    tools: Vec<ToolSpec>,
    /// How each tool is held, in the same order as `tools`.
    assemblies: Vec<cam_core::project::ToolAssembly>,
    detail: f64,
}

fn executed_frame(
    job: &CamJobV5,
    plan: &OperationPlanV5,
    spans: &[StageSpan],
    mut bounds: [f64; 4],
) -> Result<ExecutedFrame, String> {
    let mut tools = Vec::new();
    let mut assemblies = Vec::new();
    for span in spans {
        if tools.len() <= span.tool_index {
            let stage = plan
                .stages
                .iter()
                .find(|stage| stage.stage_id == span.stage_id)
                .ok_or("Missing plan stage")?;
            tools.push(sim_tool(job, &stage.tool_id)?);
            assemblies.push(sim_assembly(job, &stage.tool_id));
        }
    }
    let mut detail = f64::INFINITY;
    let mut radius: f64 = 0.;
    for tool in &tools {
        match *tool {
            ToolSpec::Knife { .. } => {}
            ToolSpec::Endmill {
                diameter,
                cutting_length,
            } => {
                radius = radius.max(diameter / 2.);
                detail = detail.min(diameter);
                let _ = cutting_length;
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
    // The executed toolpath joins the frame, and the physical stock rectangle
    // keeps a one-millimetre display margin so the block is never flush with
    // the edge of the view.
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
    Ok(ExecutedFrame {
        bounds,
        stock,
        tools,
        assemblies,
        detail,
    })
}

/// One planned motion as the simulator's display record.
///
/// The material-effect `kind` stays the caller's choice — a knife never removes
/// stock, so its stream is not a cutting stream — but the **machine execution**
/// always comes from the plan. The animation clock times a move from its
/// interpolation and its feed, so those two values have to travel with the
/// motion rather than being re-derived (or guessed) in the display.
/// The display motion one planned motion becomes.
///
/// **One planned motion stays one display motion**, including a programmed arc:
/// the arc travels with the motion and the field walks it when it sweeps. That
/// is what keeps one index space across the display — the stages, the timeline
/// spans, the path vertices, the picker and the knife headings all address
/// motions by the plan's index, so expanding an arc here would shift every one
/// of them by the chords it added
/// (`the_display_stream_is_one_motion_per_plan_motion`).
pub(crate) fn sim_motion(
    motion: &cam_core::toolpath::PlannedMotion,
    tool: usize,
    stage: u16,
    kind: &str,
) -> crate::sim::Motion {
    let arc = match motion.interpolation {
        cam_core::toolpath::Interpolation::ArcFeed(arc) => Some(arc),
        _ => None,
    };
    crate::sim::Motion {
        kind: kind.into(),
        tool,
        stage,
        interpolation: match motion.interpolation {
            cam_core::toolpath::Interpolation::Rapid => crate::sim::Interpolation::Rapid,
            cam_core::toolpath::Interpolation::LinearFeed
            | cam_core::toolpath::Interpolation::ArcFeed(_)
            | cam_core::toolpath::Interpolation::Dwell { .. } => crate::sim::Interpolation::Feed,
        },
        feed_mm_min: motion.feed_mm_min,
        arc,
        x0: motion.start.x,
        y0: motion.start.y,
        z0: motion.start.z,
        x1: motion.end.x,
        y1: motion.end.y,
        z1: motion.end.z,
    }
}

pub(crate) fn role_color(role: StageRole) -> [f32; 4] {
    match role {
        StageRole::Face => [0.16, 0.68, 0.38, 1.],
        StageRole::VcarveRough => [0.19, 0.72, 0.81, 1.],
        StageRole::VcarveFinish => [1., 0.62, 0.2, 1.],
        StageRole::ProfileRough => [0.45, 0.42, 0.86, 1.],
        StageRole::ProfileFinish => [0.85, 0.36, 0.68, 1.],
        StageRole::Knife => [1., 0.62, 0.2, 1.],
        StageRole::Drill => [0.3, 0.5, 0.75, 1.],
    }
}

/// How one piece of artwork is drawn: the operation's selection colour when it
/// is selected, and otherwise the colour the drawing itself used. Two shapes
/// that differ only in colour cut identically, so this is identity, never
/// geometry — it is what lets a carving and the outline of the part be told
/// apart while they are being picked.
fn artwork_color(selected: bool, paint: Option<cam_core::svg::SourcePaint>) -> [f32; 4] {
    if selected {
        return [0.25, 0.8, 0.85, 1.];
    }
    // The line is an overlay on the stock, so it is drawn opaque whatever the
    // source alpha was; a faint fill would otherwise be invisible.
    paint.map_or([0.5, 0.55, 0.6, 1.], |paint| {
        let [red, green, blue, _] = paint.rgba_unit();
        [red, green, blue, 1.]
    })
}

/// Build the complete scene for `job`; `plan` is the retained execution of the
/// chosen scope, or `None` for the artwork/stock preview before generation.
pub fn build(
    job: &CamJobV5,
    plan: Option<&OperationPlanV5>,
    report: Value,
) -> Result<(crate::compute::SceneMeta, Vec<u8>), String> {
    build_with_preset(
        job,
        plan,
        report,
        crate::stock_preview::DisplayPreset::Standard,
    )
}

/// The display resolution the caller asked for. It changes only the raster
/// preset; the plan, its motions and every machining value are untouched.
pub fn build_with_preset(
    job: &CamJobV5,
    plan: Option<&OperationPlanV5>,
    mut report: Value,
    preset: crate::stock_preview::DisplayPreset,
) -> Result<(crate::compute::SceneMeta, Vec<u8>), String> {
    let catalogue = artwork_inputs(job)?;
    let selected = selected_references(job);
    let components = crate::authoring::catalogue_components(&catalogue);
    let chains = crate::knife::chains(job)?;
    // The closed contours a profile operation selects, with their advisory
    // sides. This is the same catalogue the planner resolves against, projected
    // once per scene so the editor never imports the SVG on the frame thread.
    let profile_contours = crate::profile::contours(job)?;
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
    // Artwork and knife-chain points stay in setup coordinates until the
    // display frame is final below: the frame also carries the generated
    // toolpath, so normalizing them here would scale them against a different
    // rectangle than the stock and the motions.
    let mut contour_points: Vec<([f64; 3], [f32; 4])> = Vec::new();
    let mut spans = Vec::new();
    for component in &components {
        let start = contour_points.len();
        let color = artwork_color(selected.contains(&component.reference), component.paint);
        for ring in &component.rings {
            for i in 0..ring.len() {
                for p in [ring[i], ring[(i + 1) % ring.len()]] {
                    contour_points.push(([p[0], p[1], 0.02], color));
                }
            }
        }
        spans.push(json!([
            component.reference.artwork_item_id,
            start,
            contour_points.len()
        ]));
    }
    for chain in &chains {
        let start = contour_points.len();
        let color = artwork_color(selected.contains(&chain.reference), chain.paint);
        let segments = chain
            .vertices
            .len()
            .saturating_sub(usize::from(!chain.closed));
        for i in 0..segments {
            for p in [
                chain.vertices[i],
                chain.vertices[(i + 1) % chain.vertices.len()],
            ] {
                contour_points.push(([p[0], p[1], 0.02], color));
            }
        }
        spans.push(json!([
            chain.reference.artwork_item_id,
            start,
            contour_points.len()
        ]));
    }
    report["components"] = json!(components);
    report["chains"] = json!(chains);
    report["profileContours"] = json!(profile_contours);
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
    // Facing overlays, published before a plan exists: the requested area, the
    // coverage every pass must sweep, the allowed travel envelope and the
    // positions the cutter descends at. Display-only, and the same resolution
    // the planner performs, so the viewport and the plan cannot disagree.
    let mut face_plans = vec![];
    for operation in &job.operations {
        if !matches!(operation.settings, v5::OperationSettingsV5::Face(_)) {
            continue;
        }
        let Ok(Some(preview)) = v5::inspection::face_entry_preview(job, &operation.id) else {
            continue;
        };
        face_plans.push(json!({
            "operationId": operation.id,
            "axis": preview.axis.to_string(),
            "area": preview.area.map(|area| [
                area.min_x_mm,
                area.min_y_mm,
                area.width_mm,
                area.length_mm,
            ]),
            "coverage": [
                preview.coverage.min_x_mm,
                preview.coverage.min_y_mm,
                preview.coverage.width_mm,
                preview.coverage.length_mm,
            ],
            "envelope": [
                preview.envelope.min_x_mm,
                preview.envelope.min_y_mm,
                preview.envelope.width_mm,
                preview.envelope.length_mm,
            ],
            "entries": preview.entries,
            "clearances": preview.clearances,
            "passLow": preview.pass_low_mm,
            "passHigh": preview.pass_high_mm,
            "entryTravel": preview.entry_travel_mm,
            "exitTravel": preview.exit_travel_mm,
        }));
    }
    report["facePlans"] = json!(face_plans);
    if report["issues"].is_null() {
        report["issues"] = json!(
            v5::references::inspect_references(job)
                .map_err(|e| e.to_string())?
                .issues
        );
    }
    // Settle the one display frame every vertex is normalized against before
    // the first vertex is written. The frame carries the stock rectangle, the
    // artwork and — once a plan exists — the executed toolpath, so a facing
    // pass that travels past the stock widens the frame rather than changing
    // the scale of the artwork drawn inside it.
    let planned_spans = match plan {
        Some(plan) => stage_spans(plan)?,
        None => vec![],
    };
    let executed = if planned_spans.is_empty() {
        None
    } else {
        Some(executed_frame(
            job,
            plan.expect("a stage span implies a plan"),
            &planned_spans,
            bounds,
        )?)
    };
    match &executed {
        Some(frame) => bounds = frame.bounds,
        None if plan.is_none() && !contour_points.is_empty() => {
            bounds = [
                bounds[0] - 2.,
                bounds[1] - 2.,
                bounds[2] + 2.,
                bounds[3] + 2.,
            ];
        }
        None => {}
    }
    let mut vertices: Vec<crate::compute::Vertex> = contour_points
        .iter()
        .map(|(point, color)| vertex(*point, bounds, *color))
        .collect();
    let contour_vertices = vertices.len();
    let Some(plan) = plan else {
        report["inferredStockXY"] = json!(job.setup.stock.xy.is_none());
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
    let spans = planned_spans;
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
    let frame = executed.expect("a non-empty span list builds the executed frame");
    let ExecutedFrame {
        stock,
        tools,
        assemblies,
        detail,
        ..
    } = frame;
    let width = stock.x1 - stock.x0;
    let length = stock.y1 - stock.y0;
    let resolution = crate::sim::choose_resolution(width, length, detail, 8192., 64_000_000.)?;
    let stage_of = |index: usize| -> Option<&StageSpan> {
        spans
            .iter()
            .find(|span| index >= span.start && index < span.end)
    };
    // Stage index per motion. The simulated stock records this index per cell,
    // so it is the identity the palette and every colour mode resolve against;
    // `report["stages"]` publishes the same index space with its operation and
    // tool ids.
    let mut stage_of_motion = vec![0_usize; plan.motions.len()];
    for (index, span) in spans.iter().enumerate() {
        let end = span.end.min(plan.motions.len());
        for slot in &mut stage_of_motion[span.start.min(end)..end] {
            *slot = index;
        }
    }
    let mut motions = Vec::with_capacity(plan.motions.len());
    for (index, motion) in plan.motions.iter().enumerate() {
        let stage = stage_of(index).ok_or("Plan motion outside every stage")?;
        let cutting = motion.effect == cam_core::toolpath::MotionEffect::MillingSweep;
        motions.push(sim_motion(
            motion,
            stage.tool_index,
            stage_of_motion.get(index).copied().unwrap_or(0) as u16,
            if cutting { "cut" } else { "rapid_xy" },
        ));
    }
    for (index, motion) in plan.motions.iter().enumerate() {
        let color = match stage_of(index) {
            Some(stage) => role_color(stage.role),
            None => TRAVEL_COLOR,
        };
        let color = if motion.effect == cam_core::toolpath::MotionEffect::MillingSweep
            || motion.effect == cam_core::toolpath::MotionEffect::KnifeTrace
        {
            color
        } else {
            TRAVEL_COLOR
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
    // The machine checks run here, in the worker, over their own coarse raster:
    // one pass at generation time, nothing per displayed frame. They are display
    // estimates and the report says what raster they rest on.
    let holder_body = job
        .machine_configuration
        .as_ref()
        .and_then(|configuration| configuration.holder.as_ref())
        .and_then(|holder| holder.body())
        .unwrap_or_default();
    let warnings = crate::sim_checks::run(crate::sim_checks::CheckInput {
        motions: &motions,
        tools: &tools,
        assemblies: &assemblies,
        holder: &holder_body,
        stock,
    });
    let warning_cell_mm = crate::sim_checks::check_cell_mm(stock);
    let preview = crate::stock_preview::build_with_marks(
        &crate::sim::Input {
            stock,
            tools: tools.clone(),
            resolution,
            motions: motions.clone(),
            prefixes: vec![],
        },
        &marks,
        preset,
        // The execution fingerprint is the display's identity for this plan: a
        // revalidated or regenerated execution must not inherit these tiles.
        &plan.execution_fingerprint,
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
    // Stage identity table: the index the simulated stock stores per cell, with
    // the operation and tool ids a colour mode keys on. Kept separate from the
    // timeline's group labels, which have their own historical spelling.
    report["stages"] = json!(
        spans
            .iter()
            .enumerate()
            .map(|(index, span)| {
                let tool_id = plan
                    .stages
                    .iter()
                    .find(|stage| stage.stage_id == span.stage_id)
                    .map(|stage| stage.tool_id.clone())
                    .unwrap_or_default();
                json!({
                    "index": index,
                    "operation": span.operation_id,
                    "tool": span.tool_index,
                    "toolId": tool_id,
                    "role": span.role_word(),
                })
            })
            .collect::<Vec<_>>()
    );
    report["executionFingerprint"] = json!(plan.execution_fingerprint);
    report["warnings"] = json!(warnings);
    report["warningCellMm"] = json!(warning_cell_mm);
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
            assemblies,
            resolution,
            motions,
            // The machine's own rapid rate, when the job has applied a machine
            // configuration that states one. The display times G0 moves with
            // it; it is never invented here.
            rapid_rate_mm_min: job
                .machine_configuration
                .as_ref()
                .and_then(|configuration| configuration.rapid_rate_mm_min),
            // What the machine holds the tool with, when it says. The display
            // resolves the catalogue name; the segment numbers stay in core.
            holder: job
                .machine_configuration
                .as_ref()
                .and_then(|configuration| configuration.holder.clone()),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `scene.wgsl` tells travel from cutting moves by alpha: a travel vertex is
    /// translucent, a cutting one opaque. The path filters depend on that
    /// convention, so it is pinned here rather than left implicit in a colour.
    #[test]
    fn travel_moves_are_the_only_translucent_path_vertices() {
        let travel_alpha = TRAVEL_COLOR[3];
        assert!(travel_alpha < 1., "a travel vertex is translucent");
        for role in [
            StageRole::Face,
            StageRole::VcarveRough,
            StageRole::VcarveFinish,
            StageRole::ProfileRough,
            StageRole::ProfileFinish,
            StageRole::Knife,
        ] {
            assert_eq!(
                role_color(role)[3],
                1.,
                "{role:?} paths are opaque cutting moves"
            );
        }
    }

    #[test]
    fn artwork_is_drawn_in_its_own_colour_until_it_is_selected() {
        let source = cam_core::svg::SourcePaint {
            red: 0xc0,
            green: 0x39,
            blue: 0x2b,
            alpha: 255,
        };
        let drawn = artwork_color(false, Some(source));
        assert!((drawn[0] - 192. / 255.).abs() < 1e-6);
        assert!((drawn[1] - 57. / 255.).abs() < 1e-6);
        assert!((drawn[2] - 43. / 255.).abs() < 1e-6);
        assert_eq!(drawn[3], 1., "an overlay line is drawn opaque");
        // A source with no single colour keeps the neutral artwork colour.
        assert_eq!(artwork_color(false, None), [0.5, 0.55, 0.6, 1.]);
        // Selection is the operation's own signal and outranks the drawing.
        assert_eq!(artwork_color(true, Some(source)), artwork_color(true, None));
    }
}
