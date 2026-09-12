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

pub const PROTOCOL: &str = "gui2-retained-4";
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
    ImportSvg { filename: String, svg: String },
    Artwork { job: String, action: ArtworkCommand },
    Preview { job: String },
    ValidatePlan { job: String, handle: String },
    Seek { handle: String, prefix: usize },
    Open { json: String },
    Migrate { json: String },
    ApplyProfile { job: String, json: String },
    Generate { job: String },
    Prepare { job: String, handle: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ArtworkCommand {
    AddMany {
        files: Vec<crate::platform::SvgFile>,
    },
    Duplicate {
        item: v5::ArtworkItemId,
    },
    Reorder {
        items: Vec<v5::ArtworkItemId>,
    },
    Add {
        filename: String,
        svg: String,
    },
    Replace {
        item: v5::ArtworkItemId,
        filename: String,
        svg: String,
    },
    Delete {
        item: v5::ArtworkItemId,
    },
    Repair {
        expected: v5::GeometryRef,
        replacement: v5::GeometryRef,
    },
}

fn artwork_command(job: &CamJobV5, action: ArtworkCommand) -> Result<(CamJobV5, Value), String> {
    use v5::commands;
    let mut rejected = Vec::new();
    let (outcome, selected) = match action {
        ArtworkCommand::AddMany { files } => {
            let inputs = files
                .into_iter()
                .filter_map(|file| match file.content {
                    Ok(svg) => Some(commands::ArtworkInput {
                        filename: file.filename,
                        svg,
                        interpretation: Default::default(),
                        placement: Default::default(),
                        name: None,
                    }),
                    Err(error) => {
                        rejected.push(format!("{}: {}", file.filename, error));
                        None
                    }
                })
                .collect();
            let result = commands::add_artwork(job, inputs).map_err(|e| e.to_string())?;
            rejected.extend(
                result
                    .rejected
                    .iter()
                    .map(|r| format!("{}: {}", r.filename, r.error)),
            );
            if result.outcome.affected.is_empty() {
                return Err(rejected.join("\n"));
            }
            let selected = result.outcome.job.artwork.last().map(|i| i.id.clone());
            (result.outcome, selected)
        }
        ArtworkCommand::Duplicate { item } => {
            let result =
                commands::duplicate_artwork(job, &item, None).map_err(|e| e.to_string())?;
            let selected = result
                .job
                .artwork
                .iter()
                .find(|i| !job.artwork.iter().any(|old| old.id == i.id))
                .map(|i| i.id.clone());
            (result, selected)
        }
        ArtworkCommand::Reorder { items } => (
            commands::reorder_artwork(job, &items).map_err(|e| e.to_string())?,
            None,
        ),
        ArtworkCommand::Add { filename, svg } => {
            let result = commands::add_artwork(
                job,
                vec![commands::ArtworkInput {
                    filename,
                    svg,
                    interpretation: Default::default(),
                    placement: Default::default(),
                    name: None,
                }],
            )
            .map_err(|e| e.to_string())?;
            if let Some(rejection) = result.rejected.first() {
                return Err(rejection.error.to_string());
            }
            let selected = result.outcome.job.artwork.last().map(|i| i.id.clone());
            (result.outcome, selected)
        }
        ArtworkCommand::Replace {
            item,
            filename,
            svg,
        } => (
            commands::replace_artwork(
                job,
                &item,
                cam_core::job::SourceSnapshot { filename, svg },
                None,
            )
            .map_err(|e| e.to_string())?,
            Some(item),
        ),
        ArtworkCommand::Delete { item } => (
            commands::remove_artwork(job, &item).map_err(|e| e.to_string())?,
            None,
        ),
        ArtworkCommand::Repair {
            expected,
            replacement,
        } => {
            // Reject a target picked from an obsolete displayed catalogue.
            let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
            if !crate::authoring::catalogue_components(&catalogue)
                .iter()
                .any(|c| c.reference == replacement)
            {
                return Err("Replacement source changed; pick the component again".into());
            }
            (
                commands::replace_component_reference(
                    job,
                    &job.operations[0].id,
                    &expected,
                    &v5::artwork::GeometryPick {
                        artwork_item_id: replacement.artwork_item_id.clone(),
                        kind: replacement.kind,
                        local_geometry_id: replacement.local_geometry_id,
                    },
                )
                .map_err(|e| e.to_string())?,
                None,
            )
        }
    };
    let report = json!({"kind":"artwork", "activeArtwork":selected, "issues":outcome.issues,"rejectedFiles":rejected});
    let job = open(&outcome.job.to_json().map_err(|e| e.to_string())?)?;
    Ok((job, report))
}

pub fn open(text: &str) -> Result<CamJobV5, String> {
    if text.len() > 8_000_000 {
        return Err("GUI2 supports jobs up to 8 MB".into());
    }
    let job = CamJobV5::from_json(text).map_err(|e| e.to_string())?;
    if job.operations.len() != 1
        || !matches!(
            job.operations[0].settings,
            OperationSettingsV5::FlatVcarve(_)
        )
        || job
            .artwork
            .iter()
            .any(|item| !matches!(item.content, v5::ArtworkContent::Svg(_)))
    {
        return Err(
            "This workspace supports SVG artwork and one Flat V-carve operation; current document retained"
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
        Command::ImportSvg { filename, svg } => (
            crate::authoring::import_svg(filename, svg)?,
            json!({"kind":"imported"}),
        ),
        Command::Artwork { job, action } => artwork_command(&open(&job)?, action)?,
        Command::Preview { job } => (open(&job)?, json!({"kind":"preview"})),
        Command::ValidatePlan { job, handle } => {
            let job = open(&job)?;
            let retained = service.generated_plan(&handle).ok();
            let identity = cam_core::sequence::OperationPlanV5::machining_identity(
                &job,
                &v5::references::ReadinessScope::AllEnabled,
            )
            .ok();
            if let Some(retained) =
                retained.filter(|p| Some(&p.machining_identity) == identity.as_ref())
            {
                let plan = retained.trusted.plan();
                return scene(
                    &job,
                    plan,
                    json!({"kind":"revalidated","handle":handle,"executionFingerprint":plan.execution_fingerprint,"checks":retained.checks,"generationIssues":plan.generation_diagnostics}),
                );
            }
            return outline(&job, json!({"kind":"stale"}));
        }
        Command::Open { json } => (open(&json)?, json!({"kind":"opened"})),
        Command::Migrate { json } => {
            if json.len() > 8_000_000 {
                return Err("Input exceeds 8 MB limit".into());
            }
            let job = v5::migrate::migrate_json(&json).map_err(|e| e.to_string())?;
            (
                open(&job.to_json().map_err(|e| e.to_string())?)?,
                json!({"kind":"opened","migrated":true}),
            )
        }
        Command::ApplyProfile { job, json } => {
            let mut job = open(&job)?;
            let machine = cam_core::post::sequence::SequenceProfile::from_json(&json)
                .map_err(|e| e.to_string())?;
            let profile = v5::machine::apply_machine_configuration(&job, &machine, &machine.id)
                .map_err(|e| e.to_string())?;
            job = profile.job;
            job.setup.clearance_above_stock_mm = Some(machine.clearance_z_mm);
            (job, json!({"kind":"profile"}))
        }
        Command::Generate { job } => {
            let job = open(&job)?;
            let mut issues =
                v5::inspection::inspect_flat_vcarve_fields(&job, &job.operations[0].id)
                    .map_err(|e| e.to_string())?;
            issues.extend(
                v5::references::planning_readiness(
                    &job,
                    &v5::references::ReadinessScope::AllEnabled,
                )
                .map_err(|e| e.to_string())?
                .blockers(),
            );
            if !issues.is_empty() {
                return outline(&job, json!({"kind":"issues", "issues":issues}));
            }
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
                "checks":retained.checks, "generationIssues":plan.generation_diagnostics, "executionFingerprint":plan.execution_fingerprint, "retained":reply["retained"]}),
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
    let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    let components = crate::authoring::catalogue_components(&catalogue);
    let mut points = Vec::new();
    let mut spans = Vec::new();
    let mut bounds = [0., 0., 1., 1.];
    for item in &catalogue.items {
        let start = points.len();
        if let Some(error) = &item.import_error {
            report["artworkIssue"] = json!(error);
        }
        if let Some(catalogue) = &item.catalogue {
            for contour in &catalogue.contours {
                let selected = item.entries.iter().any(|entry| {
                    entry.reference.local_geometry_id == contour.component_id
                        && entry.kind == v5::GeometryRefKind::FilledComponent
                        && settings(job).components.contains(&entry.reference)
                });
                let color = if selected {
                    [0.25, 0.8, 0.85, 1.]
                } else {
                    [0.5, 0.55, 0.6, 1.]
                };
                for i in 0..contour.vertices.len() {
                    for p in [
                        contour.vertices[i],
                        contour.vertices[(i + 1) % contour.vertices.len()],
                    ] {
                        points.push(([p.x, p.y, 0.02], color));
                    }
                }
            }
        }
        spans.push(json!([item.id, start, points.len()]));
    }
    if !points.is_empty() {
        bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (p, _) in &points {
            bounds[0] = bounds[0].min(p[0]);
            bounds[1] = bounds[1].min(p[1]);
            bounds[2] = bounds[2].max(p[0]);
            bounds[3] = bounds[3].max(p[1]);
        }
    }
    let stock_start = points.len();
    if let Some(xy) = job.setup.stock.xy {
        report["stockRect"] = json!([
            xy.min_x_mm,
            xy.min_y_mm,
            xy.width_mm,
            xy.length_mm,
            job.setup.stock.thickness_mm.unwrap_or(0.)
        ]);
        bounds = [
            bounds[0].min(xy.min_x_mm),
            bounds[1].min(xy.min_y_mm),
            bounds[2].max(xy.min_x_mm + xy.width_mm),
            bounds[3].max(xy.min_y_mm + xy.length_mm),
        ];
        let corners = [
            [xy.min_x_mm, xy.min_y_mm],
            [xy.min_x_mm + xy.width_mm, xy.min_y_mm],
            [xy.min_x_mm + xy.width_mm, xy.min_y_mm + xy.length_mm],
            [xy.min_x_mm, xy.min_y_mm + xy.length_mm],
        ];
        for i in 0..4 {
            for z in [0., -job.setup.stock.thickness_mm.unwrap_or(0.)] {
                for p in [corners[i], corners[(i + 1) % 4]] {
                    points.push(([p[0], p[1], z], [0.35, 0.5, 0.65, 1.]));
                }
            }
        }
    }
    spans.push(json!(["", stock_start, points.len()]));
    report["artworkSpans"] = json!(spans);
    bounds = [
        bounds[0] - 2.,
        bounds[1] - 2.,
        bounds[2] + 2.,
        bounds[3] + 2.,
    ];
    let vertices = points
        .into_iter()
        .map(|(p, c)| vertex(p, bounds, c))
        .collect::<Vec<_>>();
    report["components"] = json!(components);
    if report["issues"].is_null() {
        report["issues"] = json!(
            v5::references::inspect_references(job)
                .map_err(|e| e.to_string())?
                .issues
        );
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
    mut report: Value,
) -> Result<(SceneMeta, Vec<u8>), String> {
    let catalogue = v5::artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    report["components"] = json!(crate::authoring::catalogue_components(&catalogue));
    report["inspection"] = json!(v5::inspection::inspect_plan(plan).map_err(|e| e.to_string())?);
    report["detailResidual"] = json!(settings(job).max_detail_residual_mm);
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
    let components = crate::authoring::catalogue_components(&catalogue);
    let mut b = [stock.x0, stock.y0, stock.x1, stock.y1];
    for c in &components {
        b[0] = b[0].min(c.bounds[0]);
        b[1] = b[1].min(c.bounds[1]);
        b[2] = b[2].max(c.bounds[2]);
        b[3] = b[3].max(c.bounds[3]);
    }
    let mut vertices = Vec::new();
    let mut spans = Vec::new();
    for component in &components {
        let start = vertices.len();
        let color = if settings(job).components.contains(&component.reference) {
            [0.25, 0.8, 0.85, 1.]
        } else {
            [0.5, 0.55, 0.6, 1.]
        };
        for ring in &component.rings {
            for i in 0..ring.len() {
                for p in [ring[i], ring[(i + 1) % ring.len()]] {
                    vertices.push(vertex([p[0], p[1], 0.02], b, color));
                }
            }
        }
        spans.push(json!([
            component.reference.artwork_item_id,
            start,
            vertices.len()
        ]));
    }
    report["artworkSpans"] = json!(spans);
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
