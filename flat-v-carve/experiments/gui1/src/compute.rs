//! Experimental adapter only. All SVG import, planning and checks are Rust core calls.
use cam_core::{job::Job, vcarve::plan_combined_with_receipt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const PROTOCOL: &str = "gui1-spike-1";
pub const SMALL: &str = include_str!("../../../fixtures/m4/contact-line.json");
pub const FLOWER: &str = include_str!("../../../../real_data/flower_box-svg.job-real.json");
pub const PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");
pub const MAX_SEGMENTS: usize = 1_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Reference { flower: bool, export: bool },
    Open { json: String },
    Synthetic { segments: usize },
    Busy,
    Crash,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub contour_vertices: usize,
    pub rough_vertices: usize,
    pub bounds: [f64; 4],
    pub name: String,
    pub report: Value,
    pub job: String,
    pub programs: Vec<cam_core::post::Program>,
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn run(request: Request) -> Result<Scene, String> {
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
fn real(input: &str, export: bool) -> Result<Scene, String> {
    if input.len() > 8_000_000 {
        return Err("GUI1 input exceeds 8 MB experiment limit".into());
    }
    let job = Job::from_json(input).map_err(|e| e.to_string())?;
    if job.vbit_planning.is_none() {
        return Err("GUI1 reference probe requires combined endmill/V-bit settings".into());
    }
    let inspection = job.inspect().map_err(|e| e.to_string())?;
    let bounds = inspection.geometry.bounds.ok_or("Artwork has no bounds")?;
    let b = [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y];
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
        "simulation": "Not yet ported: these are actual motion lines, not a heightfield simulation"});
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
    Ok(Scene {
        vertices,
        contour_vertices,
        rough_vertices: plan.endmill.motions.len() * 2,
        bounds: b,
        name: job.name.clone(),
        report,
        job: input.into(),
        programs,
    })
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
pub fn synthetic(segments: usize) -> Result<Scene, String> {
    if ![20_000, 200_000, MAX_SEGMENTS].contains(&segments) {
        return Err("Unknown synthetic workload".into());
    }
    let mut seed = 0x5eed_u64;
    let mut vertices = Vec::with_capacity(segments * 2);
    for i in 0..segments {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = ((seed >> 32) as u32 as f64 / u32::MAX as f64) as f32 * 1.6 - 0.8;
        let y = (i % 1000) as f32 / 625. - 0.8;
        for px in [x, (x + 0.015).min(0.8)] {
            vertices.push(Vertex {
                position: [px, y, -0.02],
                color: [0.15, 0.7, 0.78, 0.65],
            });
        }
    }
    let (sources, operations, field) = match segments {
        20_000 => (2, 10, 512),
        200_000 => (20, 100, 2048),
        _ => (100, 1000, 0),
    };
    let report = json!({"seed": "0x5eed", "segments": segments, "paths": segments, "vertices": vertices.len(),
        "vertexBytes": vertices.len()*std::mem::size_of::<Vertex>(), "sources": sources,
        "operations": operations, "tools": 2, "plannedFieldSide": field, "actualFieldCells": 0,
        "limitations": "Synthetic lines only; field allocation, paging and tiled updates are not implemented"});
    Ok(Scene {
        vertices,
        contour_vertices: 0,
        rough_vertices: segments * 2,
        bounds: [0., 0., 100., 100.],
        name: format!("Experimental synthetic / {segments} segments"),
        report,
        job: String::new(),
        programs: Vec::new(),
    })
}
