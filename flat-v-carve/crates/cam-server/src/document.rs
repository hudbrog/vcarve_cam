//! HTTP DTOs project engine results; no machining rules live in the service.
use cam_core::{
    geometry::{Diagnostic, Point},
    job::Job,
    svg::{ImportOptions, NormalizedGeometry, import_svg},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const API_VERSION: &str = "ui-7";
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const JOB_BYTES: usize = 64_000_000;
pub const REQUEST_BYTES: usize = 128_100_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentRequest {
    pub api_version: String,
    pub request_id: String,
    pub revision: u64,
    pub command: Command,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    Import {
        filename: String,
        svg: String,
        options: ImportOptions,
    },
    Open {
        json: String,
    },
    Display {
        svg: String,
        options: ImportOptions,
    },
    Validate {
        job: Value,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDiagnostic {
    pub code: String,
    pub severity: cam_core::geometry::Severity,
    pub stage: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}
impl From<Diagnostic> for UiDiagnostic {
    fn from(d: Diagnostic) -> Self {
        Self {
            code: d.code,
            severity: d.severity,
            stage: d.stage.into(),
            message: d.message,
            source_id: d.source_id,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkDisplay {
    coordinate_space: &'static str,
    width_mm: f64,
    height_mm: f64,
    engine_version: &'static str,
    geometry_tolerance_mm: f64,
    description: &'static str,
    components: Vec<DisplayComponent>,
}
#[derive(Debug, Serialize)]
struct DisplayComponent {
    id: String,
    label: String,
    rings: Vec<DisplayRing>,
}
#[derive(Debug, Serialize)]
struct DisplayRing {
    hole: bool,
    points: Vec<Point>,
}
fn display(geometry: &NormalizedGeometry, tolerance: f64) -> ArtworkDisplay {
    ArtworkDisplay {
        coordinate_space: "source-page-mm-y-up",
        width_mm: geometry.page_width_mm,
        height_mm: geometry.page_height_mm,
        engine_version: ENGINE_VERSION,
        geometry_tolerance_mm: tolerance,
        description: "Live Rust normalization · source geometry only",
        components: geometry
            .sources
            .iter()
            .map(|source| DisplayComponent {
                id: source.id.clone(),
                label: source
                    .label
                    .clone()
                    .unwrap_or_else(|| source.source_id.clone()),
                rings: source
                    .geometry
                    .rings()
                    .iter()
                    .zip(source.geometry.rings_mm())
                    .map(|(ring, points)| DisplayRing {
                        hole: ring.is_hole(),
                        points,
                    })
                    .collect(),
            })
            .collect(),
    }
}
fn diagnostics(geometry: &NormalizedGeometry) -> Vec<UiDiagnostic> {
    geometry
        .diagnostics
        .iter()
        .cloned()
        .map(Into::into)
        .collect()
}
// A document receipt, deliberately distinct from planner/verification fingerprints.
pub(crate) fn fingerprint(job: &Job) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ui-document-v1\0");
    hash.update(ENGINE_VERSION.as_bytes());
    hash.update(serde_json::to_vec(job).expect("validated finite job serializes"));
    format!("{:x}", hash.finalize())
}
fn opened(job: Job) -> Result<Value, Diagnostic> {
    let inspection = job.inspect()?;
    Ok(json!({
        "display": display(&inspection.geometry, job.import.geometry_tolerance_mm),
        "diagnostics": diagnostics(&inspection.geometry),
        "missingMachiningFields": inspection.missing_machining_fields,
        "documentFingerprint": fingerprint(&job), "job": job,
    }))
}
pub fn execute(command: Command) -> Result<Value, UiDiagnostic> {
    match command {
        Command::Import {
            filename,
            svg,
            options,
        } => opened(Job::from_svg(filename, svg, options)?).map_err(Into::into),
        Command::Open { json } => opened(Job::from_json(&json)?).map_err(Into::into),
        Command::Display { svg, options } => {
            // Placement affects source flattening/grid precision. Use all import options,
            // but no machining settings or inclusion so unfinished setup stays inspectable.
            let geometry = import_svg(&svg, &options, None)?;
            Ok(json!(display(&geometry, options.geometry_tolerance_mm)))
        }
        Command::Validate { job } => {
            let checked = Job::from_json(&job.to_string())
                .and_then(|job| job.inspect().map(|inspection| (job, inspection)));
            Ok(match checked {
                Ok((job, inspection)) => json!({
                    "valid": true, "scope": "editable-job-and-svg", "authoritative": true,
                    "documentFingerprint": fingerprint(&job),
                    "diagnostics": diagnostics(&inspection.geometry),
                    "missingMachiningFields": inspection.missing_machining_fields,
                }),
                Err(error) => json!({
                    "valid": false, "scope": "editable-job-and-svg", "authoritative": true,
                    "documentFingerprint": null, "diagnostics": [UiDiagnostic::from(error)],
                    "missingMachiningFields": [],
                }),
            })
        }
    }
}
