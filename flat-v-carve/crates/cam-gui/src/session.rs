//! GUI2's bounded adapter over the released ui-9 service. Lives only in the
//! persistent compute process/Worker; neither a serialized plan nor a client
//! receipt can enter the retained service.
use crate::compute::{Package, SceneMeta, SimPackage, package, vertex};
use crate::sim::{Input, Motion, Stock, ToolSpec};
use cam_core::project::v5::{self, CamJobV5, OperationSettingsV5};
use cam_service::collection::{CollectionCommand as C, CollectionScope};
use cam_service::retained::Retained;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;

pub const PROTOCOL: &str = "gui2-retained-1";
pub const FLOWER: &str = include_str!("../../../fixtures/gui2/flower.job.json");
pub const PROFILE: &str = include_str!("../../../fixtures/gui2/machine.json");
pub const MOTION_LIMIT: usize = 100_000;
thread_local! { static SERVICE: RefCell<Retained> = RefCell::new(Retained::new()); }
thread_local! { static DISPLAY: RefCell<Option<Display>> = const { RefCell::new(None) }; }
struct Display {
    fingerprint: String,
    playback: crate::sim::Playback,
    motions: Vec<Motion>,
    meta: crate::stock_preview::PreviewMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    Seek { handle: String, prefix: usize },
    Open { json: String },
    ApplyProfile { job: String, json: String },
    Generate { job: String },
    Prepare { job: String, handle: String },
}

pub fn open(text: &str) -> Result<CamJobV5, String> {
    if text.len() > 8_000_000 {
        return Err("GUI2 supports jobs up to 8 MB".into());
    }
    let job = CamJobV5::from_json(text).map_err(|e| e.to_string())?;
    if job.artwork.len() != 1
        || job.operations.len() != 1
        || !matches!(
            job.operations[0].settings,
            OperationSettingsV5::FlatVcarve(_)
        )
        || !matches!(job.artwork[0].content, v5::ArtworkContent::Svg(_))
    {
        return Err(
            "GUI2 supports one SVG and one Flat V-carve operation; current document retained"
                .into(),
        );
    }
    Ok(job)
}

pub fn settings(job: &CamJobV5) -> &v5::FlatVcarveSettingsV5 {
    let OperationSettingsV5::FlatVcarve(settings) = &job.operations[0].settings else {
        unreachable!()
    };
    settings
}

pub fn run(command: Command) -> Result<(SceneMeta, Vec<u8>), String> {
    SERVICE.with(|service| execute(&mut service.borrow_mut(), command))
}

fn terminal(reply: &Value) -> Result<(), String> {
    if reply["task"]["state"] == "succeeded" {
        return Ok(());
    }
    Err(format!("{}", reply["task"]["diagnostic"]))
}

pub fn execute(service: &mut Retained, command: Command) -> Result<(SceneMeta, Vec<u8>), String> {
    let (job, report) = match command {
        Command::Seek { handle, prefix } => {
            let plan = service.generated_plan(&handle).map_err(|e| e.to_string())?;
            return DISPLAY.with(|display| {
                let mut display = display.borrow_mut();
                let display = display.as_mut().filter(|d| d.fingerprint == plan.trusted.plan().execution_fingerprint).ok_or("Display execution expired; generate again")?;
                if prefix > display.motions.len() { return Err("Stock motion outside retained execution".into()); }
                display.playback.seek(&display.motions, prefix)?;
                let field = &display.playback.field;
                let cells = field.packed_tile_bytes();
                let mut meta = display.meta.clone();
                meta.frames = vec![crate::stock_preview::FrameMeta { prefix, stats:field.stats.clone(),checksum:field.checksum(),versions:field.versions.clone(),allocated:field.versions.iter().enumerate().filter(|(i,_)|field.tile_allocated(*i)).map(|(i,_)|i as u32).collect() }];
                meta.retained_bytes = cells.len();
                package(Package {name:String::new(),job:String::new(),report:json!({"protocol":PROTOCOL,"gui2":{"kind":"seek","prefix":prefix,"handle":handle}}),programs:vec![],bounds:[0.,0.,1.,1.],contour_vertices:0,rough_vertices:0,vertices:vec![],preview:Some(crate::stock_preview::Preview {meta,cells:vec![cells]}),sim:None})
            });
        }
        Command::Open { json } => (open(&json)?, json!({"kind":"opened"})),
        Command::ApplyProfile { job, json } => {
            let mut job = open(&job)?;
            let machine = cam_core::post::sequence::SequenceProfile::from_json(&json)
                .map_err(|e| e.to_string())?;
            let profile = v5::machine::apply_machine_configuration(&job, &machine, &machine.id)
                .map_err(|e| e.to_string())?;
            job = profile.job;
            (job, json!({"kind":"profile"}))
        }
        Command::Generate { job } => {
            let job = open(&job)?;
            let reply = service
                .execute_driven(C::Generate {
                    job: json!(job),
                    scope: CollectionScope::AllEnabled,
                })
                .map_err(|e| e.to_string())?;
            terminal(&reply)?;
            let handle = reply["task"]["planHandle"]
                .as_str()
                .ok_or("Missing retained plan handle")?;
            let retained = service.generated_plan(handle).map_err(|e| e.to_string())?;
            let plan = retained.trusted.plan();
            if plan.motions.len() > MOTION_LIMIT {
                return Err("GUI2 exceeds 100,000 motions; no truncated scene admitted".into());
            }
            let result = scene(
                &job,
                plan,
                json!({"kind":"generated", "handle":handle,
                "checks":retained.checks, "executionFingerprint":plan.execution_fingerprint, "retained":reply["retained"]}),
            )?;
            let scene = crate::compute::Scene {
                meta: result.0.clone(),
                payload: std::sync::Arc::new(result.1.clone()),
            };
            let input = scene.sim_input()?.ok_or("Missing display input")?;
            let meta = scene.meta.stock.as_ref().ok_or("Missing display stock")?;
            let mut seed = Vec::new();
            for (index, frame) in meta.frames.iter().enumerate() {
                let field = crate::sim::Field::from_packed(
                    meta.stock,
                    &input.tools,
                    meta.cell_mm,
                    scene.stock_cells(index).ok_or("Missing checkpoint")?,
                    &frame.versions,
                    &frame.allocated,
                    frame.stats.clone(),
                )?;
                seed.push((frame.prefix, field));
            }
            let playback = crate::sim::Playback::seed(
                seed.last().ok_or("No checkpoints")?.1.clone(),
                seed[0].1.clone(),
                seed,
                crate::stock_preview::MAX_PREVIEW_BYTES,
            );
            DISPLAY.with(|d| {
                *d.borrow_mut() = Some(Display {
                    fingerprint: plan.execution_fingerprint.clone(),
                    playback,
                    motions: input.motions,
                    meta: meta.clone(),
                })
            });
            return Ok(result);
        }
        Command::Prepare { job, handle } => {
            let job = open(&job)?;
            let reply = service
                .execute_driven(C::PrepareOutput {
                    job: json!(job),
                    plan_handle: handle,
                    layout: cam_core::post::sequence::OutputLayout::OneProgram,
                })
                .map_err(|e| e.to_string())?;
            terminal(&reply)?;
            let task = reply["task"]["taskId"]
                .as_str()
                .ok_or("Missing preparation task")?;
            let bundle = service
                .execute(C::PreparedOutput {
                    task_id: task.into(),
                })
                .map_err(|e| e.to_string())?;
            let handle = bundle["bundle"]["bundleHandle"]
                .as_str()
                .ok_or("Missing bundle")?;
            let file = bundle["bundle"]["files"][0]["filename"]
                .as_str()
                .ok_or("Missing checked file")?;
            let bytes = service
                .execute(C::ReadPreparedBytes {
                    bundle_handle: handle.into(),
                    filename: file.into(),
                })
                .map_err(|e| e.to_string())?;
            (
                job,
                json!({"kind":"prepared", "file":bytes["file"], "bundle":bundle["bundle"], "retained":bytes["retained"]}),
            )
        }
    };
    outline(&job, report)
}

fn outline(job: &CamJobV5, mut report: Value) -> Result<(SceneMeta, Vec<u8>), String> {
    let region = v5::artwork::inspect_artwork(job).and_then(|catalogue| {
        v5::resolve::resolve_vcarve_region(job, &job.operations[0].id, settings(job), &catalogue)
    });
    let mut vertices = Vec::new();
    let mut bounds = [0., 0., 1., 1.];
    match region {
        Ok(region) => {
            if let Some(b) = region.bounds {
                bounds = [b.min.x - 7., b.min.y - 7., b.max.x + 7., b.max.y + 7.];
                if let Some(xy) = job.setup.stock.xy {
                    bounds = [
                        xy.min_x_mm,
                        xy.min_y_mm,
                        xy.min_x_mm + xy.width_mm,
                        xy.min_y_mm + xy.length_mm,
                    ];
                }
                for ring in region.region.rings_mm() {
                    for i in 0..ring.len() {
                        for p in [ring[i], ring[(i + 1) % ring.len()]] {
                            vertices.push(vertex([p.x, p.y, 0.02], bounds, [0.85, 0.87, 0.76, 1.]));
                        }
                    }
                }
                if let Some(thickness) = job.setup.stock.thickness_mm {
                    let corners = [
                        [bounds[0], bounds[1]],
                        [bounds[2], bounds[1]],
                        [bounds[2], bounds[3]],
                        [bounds[0], bounds[3]],
                    ];
                    for i in 0..4 {
                        for z in [0., -thickness] {
                            for p in [corners[i], corners[(i + 1) % 4]] {
                                vertices.push(vertex(
                                    [p[0], p[1], z],
                                    bounds,
                                    [0.3, 0.45, 0.55, 1.],
                                ));
                            }
                        }
                        for z in [0., -thickness] {
                            vertices.push(vertex(
                                [corners[i][0], corners[i][1], z],
                                bounds,
                                [0.3, 0.45, 0.55, 1.],
                            ));
                        }
                    }
                }
            }
        }
        Err(error) => report["artworkIssue"] = json!(error.to_string()),
    }
    package(Package {
        name: job.name.clone(),
        job: job.to_json().map_err(|e| e.to_string())?,
        report: json!({"gui2":report, "protocol":PROTOCOL}),
        programs: vec![],
        bounds,
        contour_vertices: vertices.len(),
        rough_vertices: 0,
        vertices,
        preview: None,
        sim: None,
    })
}

pub fn scene(
    job: &CamJobV5,
    plan: &cam_core::sequence::OperationPlanV5,
    report: Value,
) -> Result<(SceneMeta, Vec<u8>), String> {
    let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    let region =
        v5::resolve::resolve_vcarve_region(job, &job.operations[0].id, settings(job), &catalogue)
            .map_err(|e| e.to_string())?;
    let bounds = region.bounds.ok_or("No selected artwork")?;
    let mut ids = Vec::new();
    let mut tools = Vec::new();
    for tool in &job.tools {
        if !plan.stages.iter().any(|stage| stage.tool_id == tool.id) {
            continue;
        }
        let geometry = tool.geometry.as_ref().ok_or("Missing tool geometry")?;
        tools.push(match geometry {
            cam_core::project::ToolGeometry::Endmill(t) => ToolSpec::Endmill {
                diameter: t.diameter_mm,
            },
            cam_core::project::ToolGeometry::Vbit(t) => ToolSpec::Vbit {
                angle: t.included_angle_deg,
                tip: t.tip_diameter_mm,
                diameter: t.max_cutting_diameter_mm,
                height: t.cutting_height_mm,
            },
            _ => return Err("Unsupported GUI2 tool geometry".into()),
        });
        ids.push(tool.id.clone());
    }
    let motions = plan
        .motions
        .iter()
        .map(|m| {
            Ok(Motion {
                kind: if m.is_cutting() { "cut" } else { "rapid_xy" }.into(),
                tool: ids
                    .iter()
                    .position(|id| id == &m.tool_id)
                    .ok_or("Unmapped simulation tool")?,
                x0: m.start.x,
                y0: m.start.y,
                z0: m.start.z,
                x1: m.end.x,
                y1: m.end.y,
                z1: m.end.z,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut radius = 0_f64;
    let mut detail = f64::INFINITY;
    for t in &tools {
        match *t {
            ToolSpec::Endmill { diameter } => {
                radius = radius.max(diameter / 2.);
                detail = detail.min(diameter);
            }
            ToolSpec::Vbit { tip, diameter, .. } => {
                radius = radius.max(tip.max(diameter) / 2.);
                detail = detail.min(tip);
            }
        }
    }
    let resolution = crate::sim::choose_resolution(
        bounds.max.x - bounds.min.x,
        bounds.max.y - bounds.min.y,
        detail,
        8192.,
        64_000_000.,
    )?;
    let cell = resolution.cell_mm;
    let cols = ((bounds.max.x - bounds.min.x + 2. * (radius + 1.)) / cell).ceil();
    let rows = ((bounds.max.y - bounds.min.y + 2. * (radius + 1.)) / cell).ceil();
    let x0 = (bounds.min.x + bounds.max.x - cols * cell) / 2.;
    let y0 = (bounds.min.y + bounds.max.y - rows * cell) / 2.;
    let mut stock = Stock {
        x0,
        y0,
        x1: x0 + cols * cell,
        y1: y0 + rows * cell,
        thickness_mm: job
            .setup
            .stock
            .thickness_mm
            .ok_or("Missing stock thickness")?,
    };
    if let Some(xy) = job.setup.stock.xy {
        stock.x0 = xy.min_x_mm;
        stock.y0 = xy.min_y_mm;
        stock.x1 = xy.min_x_mm + xy.width_mm;
        stock.y1 = xy.min_y_mm + xy.length_mm;
    }
    let b = [stock.x0, stock.y0, stock.x1, stock.y1];
    let mut vertices = Vec::new();
    for ring in region.region.rings_mm() {
        for i in 0..ring.len() {
            for p in [ring[i], ring[(i + 1) % ring.len()]] {
                vertices.push(vertex([p.x, p.y, 0.02], b, [0.85, 0.87, 0.76, 1.]));
            }
        }
    }
    let contour_vertices = vertices.len();
    let rough = plan
        .stages
        .iter()
        .find(|s| s.role == cam_core::sequence::StageRole::VcarveRough)
        .map_or(0, |s| s.motion_range.1);
    for (i, m) in plan.motions.iter().enumerate() {
        let color = if !m.is_cutting() {
            [0.3, 0.36, 0.44, 0.45]
        } else if i < rough {
            [0.19, 0.72, 0.81, 1.]
        } else {
            [1., 0.62, 0.2, 1.]
        };
        vertices.push(vertex([m.start.x, m.start.y, m.start.z], b, color));
        vertices.push(vertex([m.end.x, m.end.y, m.end.z], b, color));
    }
    let input = Input {
        stock,
        tools: tools.clone(),
        resolution,
        motions: motions.clone(),
        prefixes: vec![],
    };
    let preview = crate::stock_preview::build(&input, rough)?;
    package(Package {
        name: job.name.clone(),
        job: job.to_json().map_err(|e| e.to_string())?,
        report: json!({"gui2":report,"protocol":PROTOCOL,"roughingMotions":rough,"finishingMotions":motions.len()-rough,
            "inferredStockXY":job.setup.stock.xy.is_none()}),
        programs: vec![],
        bounds: b,
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
