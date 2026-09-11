//! Retained compute transport shared by native and WebAssembly workers.
//!
//! A scene leaves the compute worker as a small metadata document
//! plus one binary payload (see `pages`). Motion geometry and stock cells are
//! never serialized into the metadata document.
use crate::pages::{
    Builder, Payload, SECTION_CONTOUR, SECTION_MOTIONS, SECTION_SIM, SECTION_STOCK,
};
use crate::sim::{Input, Motion, Resolution, Stock, ToolSpec};
use crate::stock_preview::{Preview, PreviewMeta};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const PROTOCOL: &str = "cam-gui-retained-1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Gui2(crate::session::Command),
    Busy,
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
        Request::Gui2(command) => crate::session::run(command),
        Request::Busy => {
            // Deliberately non-yielding CPU work. Only the supervising process/Worker can stop it.
            let mut seed = 0x5eed_u64;
            loop {
                seed = std::hint::black_box(seed.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
        }
    }
}

/// Run a request and keep the failure message next to the payload, so callers
/// can build the worker message without losing either half.
pub fn run_framed(request: Request) -> (Result<SceneMeta, String>, Vec<u8>) {
    match run(request) {
        Ok((meta, payload)) => (Ok(meta), payload),
        Err(error) => (Err(error), Vec::new()),
    }
}

/// Metadata document carried inside a worker message. The parent deserializes
/// exactly this shape (`{"Ok":…}` or `{"Err":…}`), so the two sides cannot drift
/// apart: both the native worker and the browser worker use this function.
pub fn metadata_document(result: &Result<SceneMeta, String>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(result).map_err(|e| e.to_string())
}

/// The complete worker message: `u32 metadata length | metadata | payload`.
pub fn worker_message(request: Request) -> Result<Vec<u8>, String> {
    let (result, payload) = run_framed(request);
    Ok(crate::pages::frame_message(
        &metadata_document(&result)?,
        &payload,
    ))
}

/// Parent-side decoder for a worker message. This is the single place a worker
/// result becomes a scene, used by the native supervisor.
pub fn parse_worker_message(
    bytes: Vec<u8>,
) -> Result<(Result<SceneMeta, String>, Vec<u8>), String> {
    let (metadata, payload) = crate::pages::parse_message(bytes)?;
    let result: Result<SceneMeta, String> =
        serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    Ok((result, payload))
}

pub(crate) struct Package {
    pub name: String,
    pub job: String,
    pub report: Value,
    pub programs: Vec<cam_core::post::Program>,
    pub bounds: [f64; 4],
    pub contour_vertices: usize,
    pub rough_vertices: usize,
    pub vertices: Vec<Vertex>,
    pub preview: Option<Preview>,
    pub sim: Option<SimPackage>,
}

pub(crate) struct SimPackage {
    pub stock: Stock,
    pub tools: Vec<ToolSpec>,
    pub resolution: Resolution,
    pub motions: Vec<Motion>,
}

/// Assemble metadata plus the sectioned payload. `vertices` holds the contour
/// lines followed by two vertices per motion.
pub(crate) fn package(package: Package) -> Result<(SceneMeta, Vec<u8>), String> {
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

fn section_of(payload: &Payload, kind: u8, index: usize) -> Result<(usize, usize), String> {
    payload
        .find(kind, index)
        .map(|section| (section.offset, section.len))
        .ok_or_else(|| "Payload is missing a required section".into())
}

pub(crate) fn vertex(p: [f64; 3], b: [f64; 4], color: [f32; 4]) -> Vertex {
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
