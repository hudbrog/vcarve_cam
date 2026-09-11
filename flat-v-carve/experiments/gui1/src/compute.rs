//! Experimental adapter only. All SVG import, planning and checks are Rust core calls.
//!
//! A scene leaves the disposable compute process as a small metadata document
//! plus one binary payload (see `pages`). Motion geometry and stock cells are
//! never serialized into the metadata document.
use crate::pages::{
    Builder, Payload, SECTION_CONTOUR, SECTION_MOTIONS, SECTION_SIM, SECTION_STOCK,
};
use crate::sim::{Input, Motion, Resolution, Stock, ToolSpec};
use crate::stock_preview::{Preview, PreviewMeta};
use cam_core::{job::Job, vcarve::plan_combined_with_receipt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const PROTOCOL: &str = "gui1-spike-4";
pub const SMALL: &str = include_str!("../../../fixtures/m4/contact-line.json");
pub const FLOWER: &str = include_str!("../../../../real_data/flower_box-svg.job-real.json");
pub const PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");
pub const MAX_SEGMENTS: usize = 1_000_000;
/// Above this the display process does not receive a replayable motion stream;
/// checkpoint selection still works and the UI says so.
pub const SIM_TRANSPORT_LIMIT: usize = 300_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Reference { flower: bool, export: bool },
    Open { json: String },
    Synthetic { segments: usize },
    Busy,
    Crash,
}
#[repr(C)]
#[derive(
    Clone, Copy, Debug, PartialEq, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable,
)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

/// Small, JSON-safe part of a scene result.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneMeta {
    pub protocol: String,
    pub name: String,
    pub report: Value,
    pub job: String,
    pub programs: Vec<cam_core::post::Program>,
    pub bounds: [f64; 4],
    pub contour_vertices: usize,
    pub rough_vertices: usize,
    pub motions: usize,
    pub page_motions: usize,
    pub motion_offset: usize,
    pub motion_len: usize,
    pub payload_bytes: usize,
    pub payload_sha256: String,
    pub sections: Vec<crate::pages::Section>,
    pub stock: Option<PreviewMeta>,
    pub sim: Option<SimMeta>,
    pub transport: TransportMeta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimMeta {
    pub stock: Stock,
    pub tools: Vec<ToolSpec>,
    pub resolution: Resolution,
    pub motions: usize,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportMeta {
    pub metadata_bytes: usize,
    pub payload_bytes: usize,
    pub contour_bytes: usize,
    pub vertex_bytes: usize,
    pub stock_bytes: usize,
    pub sim_bytes: usize,
    pub motion_pages: usize,
    pub stock_checkpoints: usize,
}

/// Scene as the display process holds it: metadata plus the shared payload.
#[derive(Clone, Debug)]
pub struct Scene {
    pub meta: SceneMeta,
    pub payload: Arc<Vec<u8>>,
}

impl Scene {
    pub fn motion_count(&self) -> usize {
        self.meta.motions
    }
    /// Identity of the payload contents, derived from its hash. The renderer
    /// uses it to decide whether resident pages can still be trusted.
    pub fn identity(&self) -> u64 {
        u64::from_str_radix(self.meta.payload_sha256.get(..16).unwrap_or("0"), 16).unwrap_or(0)
    }
    pub fn vertex_count(&self) -> usize {
        self.meta.contour_vertices + self.meta.motions * 2
    }
    pub fn contour_bytes(&self) -> &[u8] {
        &self.payload[..self.meta.motion_offset]
    }
    pub fn motion_bytes(&self) -> &[u8] {
        &self.payload[self.meta.motion_offset..self.meta.motion_offset + self.meta.motion_len]
    }
    pub fn stock_cells(&self, checkpoint: usize) -> Option<&[u8]> {
        let section = self
            .meta
            .sections
            .iter()
            .filter(|s| s.kind == SECTION_STOCK)
            .nth(checkpoint)?;
        Some(&self.payload[section.offset..section.offset + section.len])
    }
    /// Rebuild the replayable display input when it was transported.
    pub fn sim_input(&self) -> Result<Option<Input>, String> {
        let Some(sim) = &self.meta.sim else {
            return Ok(None);
        };
        let motions = crate::sim::decode_motions(
            &self.payload[sim.offset..sim.offset + sim.len],
            sim.tools.len(),
        )?;
        if motions.len() != sim.motions {
            return Err("Transported motion stream length mismatch".into());
        }
        Ok(Some(Input {
            stock: sim.stock,
            tools: sim.tools.clone(),
            resolution: sim.resolution,
            motions,
            prefixes: Vec::new(),
        }))
    }
    pub fn stock_sections(&self) -> impl Iterator<Item = &crate::pages::Section> {
        self.meta
            .sections
            .iter()
            .filter(|s| s.kind == SECTION_STOCK)
    }
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn run(request: Request) -> Result<(SceneMeta, Vec<u8>), String> {
    match request {
        Request::Reference { flower, export } => real(if flower { FLOWER } else { SMALL }, export),
        Request::Open { json } => real(&json, false),
        Request::Synthetic { segments } => synthetic(segments),
        Request::Crash => Err("Injected compute failure; previous result retained".into()),
        Request::Busy => {
            // Deliberately non-yielding CPU work. Only the supervising process/Worker can stop it.
            let mut seed = 0x5eed_u64;
            loop {
                seed = std::hint::black_box(seed.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        }
    }
}

struct Package {
    name: String,
    job: String,
    report: Value,
    programs: Vec<cam_core::post::Program>,
    bounds: [f64; 4],
    contour_vertices: usize,
    rough_vertices: usize,
    vertices: Vec<Vertex>,
    preview: Option<Preview>,
    sim: Option<SimPackage>,
}

struct SimPackage {
    stock: Stock,
    tools: Vec<ToolSpec>,
    resolution: Resolution,
    motions: Vec<Motion>,
}

/// Assemble metadata plus the sectioned payload. `vertices` holds the contour
/// lines followed by two vertices per motion.
fn package(package: Package) -> Result<(SceneMeta, Vec<u8>), String> {
    let motions = (package.vertices.len() - package.contour_vertices) / 2;
    let mut builder = Builder::new();
    builder.push(
        SECTION_CONTOUR,
        bytemuck::cast_slice(&package.vertices[..package.contour_vertices]).to_vec(),
    );
    builder.push(
        SECTION_MOTIONS,
        bytemuck::cast_slice(&package.vertices[package.contour_vertices..]).to_vec(),
    );
    if let Some(preview) = &package.preview {
        for cells in &preview.cells {
            builder.push(SECTION_STOCK, cells.clone());
        }
    }
    if let Some(sim) = &package.sim {
        builder.push(SECTION_SIM, crate::sim::encode_motions(&sim.motions));
    }
    let parsed = Payload::parse(builder.finish())?;
    let sections = parsed.sections().to_vec();
    let (motion_offset, motion_len) = section_of(&parsed, SECTION_MOTIONS, 0)?;
    let stock_bytes = (0..package.preview.as_ref().map_or(0, |p| p.cells.len()))
        .filter_map(|i| parsed.find(SECTION_STOCK, i))
        .map(|s| s.len)
        .sum();
    let sim_bytes = parsed.find(SECTION_SIM, 0).map_or(0, |s| s.len);
    let (sim_offset, sim_len) = if sim_bytes > 0 {
        section_of(&parsed, SECTION_SIM, 0)?
    } else {
        (0, 0)
    };
    let payload = parsed.into_bytes();
    let transport = TransportMeta {
        metadata_bytes: 0,
        payload_bytes: payload.len(),
        contour_bytes: package.contour_vertices * std::mem::size_of::<Vertex>(),
        vertex_bytes: motions * 2 * std::mem::size_of::<Vertex>(),
        stock_bytes,
        sim_bytes,
        motion_pages: crate::pages::PageTable::new(motion_offset, motion_len, motions)?
            .page_count(),
        stock_checkpoints: package.preview.as_ref().map_or(0, |p| p.cells.len()),
    };
    let mut report = package.report;
    report["transport"] = serde_json::to_value(&transport).map_err(|e| e.to_string())?;
    let mut meta = SceneMeta {
        protocol: PROTOCOL.into(),
        name: package.name,
        report,
        job: package.job,
        programs: package.programs,
        bounds: package.bounds,
        contour_vertices: package.contour_vertices,
        rough_vertices: package.rough_vertices,
        motions,
        page_motions: crate::pages::PAGE_MOTIONS,
        motion_offset,
        motion_len,
        payload_bytes: payload.len(),
        payload_sha256: hash(&payload),
        sections,
        stock: package.preview.map(|p| p.meta),
        sim: package.sim.map(|s| SimMeta {
            stock: s.stock,
            tools: s.tools,
            resolution: s.resolution,
            motions: s.motions.len(),
            offset: sim_offset,
            len: sim_len,
        }),
        transport,
    };
    // Two passes so the reported metadata size includes the report itself.
    for _ in 0..2 {
        meta.transport.metadata_bytes = serde_json::to_vec(&meta).map_err(|e| e.to_string())?.len();
    }
    Ok((meta, payload))
}

fn real(input: &str, export: bool) -> Result<(SceneMeta, Vec<u8>), String> {
    if input.len() > 8_000_000 {
        return Err("GUI1 input exceeds 8 MB experiment limit".into());
    }
    let job = Job::from_json(input).map_err(|e| e.to_string())?;
    if job.vbit_planning.is_none() {
        return Err("GUI1 reference probe requires combined endmill/V-bit settings".into());
    }
    let inspection = job.inspect().map_err(|e| e.to_string())?;
    let bounds = inspection.geometry.bounds.ok_or("Artwork has no bounds")?;
    let margin = job
        .tools
        .iter()
        .filter_map(|t| t.geometry.as_ref())
        .map(|g| match g {
            cam_core::job::ToolGeometry::Endmill(t) => t.diameter_mm / 2.,
            cam_core::job::ToolGeometry::Vbit(t) => {
                t.max_cutting_diameter_mm.max(t.tip_diameter_mm) / 2.
            }
        })
        .fold(0., f64::max)
        + 1.;
    let b = [
        bounds.min.x - margin,
        bounds.min.y - margin,
        bounds.max.x + margin,
        bounds.max.y + margin,
    ];
    let mut vertices = Vec::new();
    for ring in inspection.geometry.selected.rings_mm() {
        for i in 0..ring.len() {
            let a = ring[i];
            let z = ring[(i + 1) % ring.len()];
            vertices.push(vertex([a.x, a.y, 0.02], b, [0.85, 0.87, 0.76, 1.]));
            vertices.push(vertex([z.x, z.y, 0.02], b, [0.85, 0.87, 0.76, 1.]));
        }
    }
    let contour_vertices = vertices.len();
    let (plan, receipt) = plan_combined_with_receipt(&job).map_err(|e| e.to_string())?;
    let count = plan.endmill.motions.len() + plan.vbit_motions.len();
    if count > MAX_SEGMENTS {
        return Err("Motion limit exceeded; no partial success".into());
    }
    for (motions, color) in [
        (&plan.endmill.motions, [0.19, 0.72, 0.81, 1.]),
        (&plan.vbit_motions, [1., 0.62, 0.2, 1.]),
    ] {
        for m in motions {
            let color = if m.kind.cutting() {
                color
            } else {
                [0.3, 0.36, 0.44, 0.45]
            };
            vertices.push(vertex([m.start.x, m.start.y, m.start.z], b, color));
            vertices.push(vertex([m.end.x, m.end.y, m.end.z], b, color));
        }
    }
    let mut report = json!({"protocol": PROTOCOL, "inputSha256": hash(input.as_bytes()),
        "sourceSha256": hash(job.source.svg.as_bytes()), "profileSha256": hash(PROFILE.as_bytes()),
        "summary": cam_service::summary::combined(&plan), "roughingMotions": plan.endmill.motions.len(),
        "finishingMotions": plan.vbit_motions.len(), "paths": inspection.geometry.selected.rings_mm().len(),
        "vertices": vertices.len(), "vertexBytes": vertices.len() * std::mem::size_of::<Vertex>(),
        "tools": job.tools.len(), "operations": 1, "stockThicknessMm": job.stock.thickness_mm,
        "simulation": "Display-only Rust heightfield preview; explicit coarser grid, discrete checkpoints and bounded replay"});
    let slices = cam_service::inspection::Inspection::combined(&plan)
        .slices
        .into_iter()
        .map(|s| s.info)
        .collect::<Vec<_>>();
    let setup = crate::sim_setup::build(&job, &plan, &slices);
    let preview = setup
        .as_ref()
        .map_err(|e| e.clone())
        .and_then(|input| crate::stock_preview::build(input, plan.endmill.motions.len()));
    let stock_preview = match preview {
        Ok(preview) => {
            report["stockPreview"] = json!({"cols":preview.meta.cols,"rows":preview.meta.rows,
                "cellMm":preview.meta.cell_mm,"referenceCellMm":preview.meta.reference_cell_mm,
                "retainedCellBytes":preview.meta.retained_bytes,
                "prefixes":preview.meta.frames.iter().map(|f|f.prefix).collect::<Vec<_>>(),
                "tiles":preview.meta.tiles_x*preview.meta.tiles_y,
                "finalChecksum":preview.meta.frames.last().map(|f|f.checksum.as_str())});
            Some(preview)
        }
        Err(error) => {
            report["stockPreviewError"] = json!(error);
            None
        }
    };
    let mut programs = Vec::new();
    if export {
        let profile =
            cam_core::post::LinuxCncProfile::from_json(PROFILE).map_err(|e| e.to_string())?;
        let retained = plan.to_json().map_err(|e| e.to_string())?;
        let output = cam_core::vcarve::export_retained_plan(
            retained.as_bytes(),
            &receipt,
            &profile,
            cam_core::post::ProgramLayout::Combined,
            &cam_core::verification::VerificationOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        report["export"] = serde_json::to_value(output.report).map_err(|e| e.to_string())?;
        programs = output.programs;
    }
    // Record the current sequence migration separately; it never substitutes for this real plan.
    let migration = cam_service::sequence::execute(cam_service::sequence::SequenceCommand::Open {
        json: input.into(),
    });
    report["canonicalOpen"] = match migration {
        Ok(v) => v,
        Err(e) => json!({"error": e.to_string()}),
    };
    let sim = match setup {
        Ok(input) if input.motions.len() <= SIM_TRANSPORT_LIMIT => Some(SimPackage {
            stock: input.stock,
            tools: input.tools,
            resolution: input.resolution,
            motions: input.motions,
        }),
        Ok(input) => {
            report["simReplay"] = json!(format!(
                "{} motions exceed the {} replay transport limit; checkpoint selection only",
                input.motions.len(),
                SIM_TRANSPORT_LIMIT
            ));
            None
        }
        Err(_) => None,
    };
    package(Package {
        name: job.name.clone(),
        job: input.into(),
        report,
        programs,
        bounds: b,
        contour_vertices,
        rough_vertices: plan.endmill.motions.len() * 2,
        vertices,
        preview: stock_preview,
        sim,
    })
}

fn section_of(payload: &Payload, kind: u8, index: usize) -> Result<(usize, usize), String> {
    payload
        .find(kind, index)
        .map(|section| (section.offset, section.len))
        .ok_or_else(|| "Payload is missing a required section".into())
}

fn vertex(p: [f64; 3], b: [f64; 4], color: [f32; 4]) -> Vertex {
    let size = (b[2] - b[0]).max(b[3] - b[1]).max(0.001);
    Vertex {
        position: [
            ((p[0] - (b[0] + b[2]) / 2.) / size * 1.6) as f32,
            ((p[1] - (b[1] + b[3]) / 2.) / size * 1.6) as f32,
            (p[2] / size * 1.6) as f32,
        ],
        color,
    }
}

/// Deterministic synthetic stream used as the transport/rendering workload.
/// Bounds are real millimetres so the same motions can drive the display field.
pub fn synthetic(segments: usize) -> Result<(SceneMeta, Vec<u8>), String> {
    if ![20_000, 200_000, MAX_SEGMENTS].contains(&segments) {
        return Err("Unknown synthetic workload".into());
    }
    // `checkpoints` bounds the display replay window; `applied` bounds how much
    // of the L stream is integrated into the probe field.
    let (sources, operations, side, applied, checkpoints) = match segments {
        20_000 => (2, 10, 512, segments, 5),
        200_000 => (20, 100, 2048, segments, 9),
        _ => (100, 1000, 2048, 150_000, 5),
    };
    let mut seed = 0x5eed_u64;
    let mut motions = Vec::with_capacity(segments);
    for i in 0..segments {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = ((seed >> 32) as u32 as f64 / u32::MAX as f64) * 92. + 4.;
        let y = 4. + (i % 1000) as f64 * 0.092;
        let depth = -0.4 - 0.6 * ((i / 37) as f64 * 0.11).sin().abs();
        motions.push(Motion {
            kind: "cut".into(),
            tool: 0,
            x0: x,
            y0: y,
            z0: depth,
            x1: (x + 3.).min(96.),
            y1: y,
            z1: depth,
        });
    }
    let bounds = [-1., -1., 101., 101.];
    let stock = Stock {
        x0: 0.,
        y0: 0.,
        x1: 100.,
        y1: 100.,
        thickness_mm: 8.,
    };
    let tools = vec![
        ToolSpec::Endmill { diameter: 3. },
        ToolSpec::Vbit {
            angle: 90.,
            tip: 0.2,
            diameter: 12.,
            height: 6.,
        },
    ];
    let resolution = crate::sim::choose_resolution(100., 100., 0.2, 8192., 64_000_000.)?;
    let mut vertices = Vec::with_capacity(segments * 2);
    for (i, motion) in motions.iter().enumerate() {
        let color = if i % 5 == 0 {
            [1., 0.62, 0.2, 1.]
        } else {
            [0.15, 0.7, 0.78, 0.65]
        };
        vertices.push(vertex([motion.x0, motion.y0, motion.z0], bounds, color));
        vertices.push(vertex([motion.x1, motion.y1, motion.z1], bounds, color));
    }
    let display = Input {
        stock,
        tools: tools.clone(),
        resolution,
        motions: motions[..applied].to_vec(),
        prefixes: Vec::new(),
    };
    let preview = crate::stock_preview::build_with(
        &display,
        applied / 2,
        checkpoints,
        crate::stock_preview::PROBE_BUDGET_BYTES,
    )
    .map_err(|e| format!("Synthetic display field: {e}"))?;
    let replayable = segments <= SIM_TRANSPORT_LIMIT;
    let report = json!({"seed": "0x5eed", "segments": segments, "paths": segments,
        "vertices": vertices.len(), "vertexBytes": vertices.len()*std::mem::size_of::<Vertex>(),
        "sources": sources, "operations": operations, "tools": tools.len(),
        "plannedFieldSide": side, "displayFieldCells": preview.meta.cols * preview.meta.rows,
        "displayFieldCols": preview.meta.cols, "displayFieldRows": preview.meta.rows,
        "displayTiles": preview.meta.tiles_x * preview.meta.tiles_y,
        "appliedStockMotions": applied, "replayableMotions": replayable,
        "stockCheckpoints": preview.meta.frames.len(),
        "stockPartial": applied < segments,
        "limitations": "Synthetic cutting moves only; the display field is a paging and upload probe, not a machining simulation"});
    package(Package {
        name: format!("Experimental synthetic / {segments} segments"),
        job: String::new(),
        report,
        programs: Vec::new(),
        bounds,
        contour_vertices: 0,
        rough_vertices: segments * 2,
        vertices,
        preview: Some(preview),
        sim: replayable.then_some(SimPackage {
            stock,
            tools,
            resolution,
            motions,
        }),
    })
}
