pub mod artifact;
pub mod document;
pub mod exporting;
pub mod inspection;
pub mod library;
pub mod planning;
pub mod planning_worker;
pub mod serve;
pub mod verification;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path as RoutePath, Request, State, rejection::JsonRejection},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use document::{API_VERSION, DocumentRequest, ENGINE_VERSION, JOB_BYTES, REQUEST_BYTES};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;

const WORKERS: usize = 2;
pub type Assets = HashMap<String, (&'static str, Bytes)>;

/// Assets compiled into the host executable; bytes are shared without copying.
pub fn embedded_assets(files: &[(&'static str, &'static [u8])]) -> Assets {
    files
        .iter()
        .map(|&(path, bytes)| {
            let key = if path == "index.html" {
                "/".into()
            } else {
                format!("/{path}")
            };
            (key, (content_type(path), Bytes::from_static(bytes)))
        })
        .collect()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Read only build assets at startup. HTTP paths never become filesystem paths.
pub fn load_assets(directory: &Path) -> io::Result<Assets> {
    let mut assets = HashMap::new();
    let mut total = 0;
    fn insert(
        assets: &mut Assets,
        total: &mut usize,
        path: &Path,
        key: String,
        mime: &'static str,
    ) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.len() > 32_000_000 - *total as u64 {
            return Err(io::Error::other(
                "UI assets must be regular files totaling at most 32 MB",
            ));
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take((32_000_000 - *total + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 32_000_000 - *total {
            return Err(io::Error::other(
                "UI assets changed beyond the 32 MB size limit",
            ));
        }
        *total += bytes.len();
        assets.insert(key, (mime, bytes.into()));
        Ok(())
    }
    insert(
        &mut assets,
        &mut total,
        &directory.join("index.html"),
        "/".into(),
        "text/html; charset=utf-8",
    )?;
    let asset_dir = directory.join("assets");
    if fs::symlink_metadata(&asset_dir)?.file_type().is_symlink() {
        return Err(io::Error::other("UI assets directory cannot be a symlink"));
    }
    for entry in fs::read_dir(asset_dir)? {
        let entry = entry?;
        let path = entry.path();
        let mime = match path.extension().and_then(|e| e.to_str()) {
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("png") => "image/png",
            Some("woff2") => "font/woff2",
            _ => continue,
        };
        insert(
            &mut assets,
            &mut total,
            &path,
            format!("/assets/{}", entry.file_name().to_string_lossy()),
            mime,
        )?;
    }
    Ok(assets)
}

#[derive(Clone)]
struct AppState {
    authority: String,
    origin: String,
    token: String,
    assets: Arc<Assets>,
    workers: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    planning: Arc<planning::Planning>,
    library: Option<library::Library>,
}
pub fn router(port: u16, assets: Assets) -> io::Result<Router> {
    router_with_planning(port, assets, planning::Planning::new()?)
}
pub fn router_with_planning(
    port: u16,
    assets: Assets,
    planning: Arc<planning::Planning>,
) -> io::Result<Router> {
    router_with_library(port, assets, planning, None)
}
pub fn router_with_library(
    port: u16,
    assets: Assets,
    planning: Arc<planning::Planning>,
    directory: Option<std::path::PathBuf>,
) -> io::Result<Router> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(io::Error::other)?;
    let authority = format!("127.0.0.1:{port}");
    let state = AppState {
        origin: format!("http://{authority}"),
        authority,
        token: secret.iter().map(|b| format!("{b:02x}")).collect(),
        assets: Arc::new(assets),
        workers: Arc::new(Semaphore::new(WORKERS)),
        requests: Arc::new(Semaphore::new(8)),
        planning,
        library: directory.map(library::Library::new),
    };
    Ok(Router::new()
        .route("/api/v1/session", get(session))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/document", post(document_request))
        .route(
            "/api/v1/library",
            post(library_metadata).layer(DefaultBodyLimit::max(library::METADATA_BYTES)),
        )
        .route("/api/v1/library/job", post(library_job))
        .route("/api/v1/tasks", post(start_plan))
        .route(
            "/api/v1/verifications",
            post(start_verification).layer(DefaultBodyLimit::max(16_384)),
        )
        .route(
            "/api/v1/exports",
            post(start_export).layer(DefaultBodyLimit::max(96_000)),
        )
        .route("/api/v1/tasks/{id}/export", get(export_result))
        .route("/api/v1/tasks/{id}/verification", get(verification_result))
        .route("/api/v1/tasks/{id}", get(task_snapshot))
        .route("/api/v1/tasks/{id}/cancel", post(cancel_plan))
        .route("/api/v1/tasks/{id}/result", get(plan_result))
        .route("/api/v1/tasks/{id}/slices/{slice}", get(stock_slice))
        .route("/api/v1/tasks/{id}/artifact", get(plan_artifact))
        .fallback(asset)
        .layer(DefaultBodyLimit::max(REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state))
}
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(
            json!({ "apiVersion": API_VERSION, "engineVersion": ENGINE_VERSION,
        "error": { "code": code, "message": message } }),
        ),
    )
        .into_response()
}
async fn boundary(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let header_is = |name: &str, expected: &str| {
        headers.get_all(name).iter().count() == 1
            && headers.get(name).and_then(|v| v.to_str().ok()) == Some(expected)
    };
    let mut response = if !header_is("host", &state.authority)
        || (headers.contains_key("origin") && !header_is("origin", &state.origin))
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site" || v == "same-site")
    {
        error(
            StatusCode::FORBIDDEN,
            "LOCAL_ORIGIN",
            "Use the exact loopback URL printed by the local service.",
        )
    } else if request.uri().path().starts_with("/api/")
        && request.uri().path() != "/api/v1/session"
        && !header_is("x-cam-session", &state.token)
    {
        error(
            StatusCode::UNAUTHORIZED,
            "LOCAL_SESSION",
            "Local service restarted or session is missing. Reconnect and retry.",
        )
    } else {
        let permit = state.requests.try_acquire();
        match permit {
            Err(_) => error(
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_BUSY",
                "Too many requests. Retry shortly.",
            ),
            Ok(_permit) => {
                match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await {
                    Ok(response) => response,
                    Err(_) => error(
                        StatusCode::REQUEST_TIMEOUT,
                        "REQUEST_TIMEOUT",
                        "Request timed out. Engine work may still be finishing.",
                    ),
                }
            }
        }
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
async fn session(State(state): State<AppState>) -> Json<Value> {
    Json(
        json!({ "apiVersion": API_VERSION, "engineVersion": ENGINE_VERSION, "sessionToken": state.token }),
    )
}
async fn capabilities(State(state): State<AppState>) -> Json<Value> {
    Json(
        json!({ "apiVersion": API_VERSION, "mode": "live", "engineVersion": ENGINE_VERSION,
        "importArtwork": true, "openJob": true, "validateDraft": true,
        "toolLibrary": state.library.as_ref().map(|l|library::limits(l.location())),
        "planningStages": ["endmill", "combined"], "verificationScopes": ["continuous-stock"], "exportFormats": ["linuxcnc"],
        "export": {"profileBytes":exporting::PROFILE_BYTES,"programBytes":exporting::PROGRAM_BYTES,"layouts":["combined","per_tool"]},
        "verification": { "defaultOptions": cam_core::verification::VerificationOptions::default() },
        "planning": { "instanceId": state.planning.instance_id, "concurrentPlans": 1,
            "maxPending": planning::MAX_PENDING, "maxTasks": planning::MAX_TASKS,
            "retainedResults": planning::RETAINED_RESULTS, "timeoutSeconds": planning::TIMEOUT_SECONDS,
            "previewMotions": planning_worker::PREVIEW_MOTIONS, "artifactBytes": null,
            "stockSlices": true, "sliceVertices": inspection::SLICE_VERTICES, "inspectionVertices": inspection::TOTAL_VERTICES },
        "limits": { "svgBytes": cam_core::svg::MAX_SVG_BYTES, "jobBytes": JOB_BYTES,
            "requestBytes": REQUEST_BYTES, "concurrentInspections": WORKERS } }),
    )
}
fn task_error(failure: planning::Failure) -> Response {
    error(
        StatusCode::from_u16(failure.0).unwrap(),
        failure.1,
        &failure.2,
    )
}
async fn start_plan(
    State(state): State<AppState>,
    body: Result<Json<planning::Start>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(value)) => value,
        Err(rejection) => return error(rejection.status(), "REQUEST_JSON", &rejection.body_text()),
    };
    let Ok(permit) = state.workers.clone().try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_BUSY",
            "The engine is inspecting other requests. Retry shortly.",
        );
    };
    // Parsing/canonicalizing large jobs must not occupy an async executor thread.
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.planning.start(request)
    })
    .await
    {
        Ok(Ok(snapshot)) => (StatusCode::ACCEPTED, Json(snapshot)).into_response(),
        Ok(Err(failure)) => task_error(failure),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PLAN_START_FAILURE",
            "Could not accept the plan. Retry with the same request ID.",
        ),
    }
}
async fn task_snapshot(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Response {
    match state.planning.snapshot(&id) {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn start_verification(
    State(state): State<AppState>,
    body: Result<Json<verification::Start>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(value)) => value,
        Err(rejection) => return error(rejection.status(), "REQUEST_JSON", &rejection.body_text()),
    };
    match verification::start(&state.planning, request) {
        Ok(snapshot) => (StatusCode::ACCEPTED, Json(snapshot)).into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn verification_result(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Response {
    match state.planning.result(&id) {
        Ok((snapshot, result)) if snapshot.verification.is_some() => {
            match serde_json::from_str::<Value>(&result.artifact) {
                Ok(report) => Json(
                    json!({"task":snapshot,"coordinateSpace":"workpiece-mm-z-up","report":report}),
                )
                .into_response(),
                Err(_) => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "VERIFICATION_REPORT",
                    "Stored report could not be read.",
                ),
            }
        }
        Ok(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "TASK_KIND",
            "This task is not a verification.",
        ),
        Err(failure) => task_error(failure),
    }
}
async fn start_export(
    State(state): State<AppState>,
    body: Result<Json<exporting::Start>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(value)) => value,
        Err(r) => return error(r.status(), "REQUEST_JSON", &r.body_text()),
    };
    match exporting::start(&state.planning, request) {
        Ok(snapshot) => (StatusCode::ACCEPTED, Json(snapshot)).into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn export_result(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Response {
    match state.planning.result(&id) {
        Ok((snapshot, result)) if snapshot.export.is_some() => {
            // The report and exact programs are one immutable response. No filename
            // supplied by a client is ever mapped to a path or arbitrary file.
            match serde_json::from_str::<Value>(&result.artifact) {
                Ok(report) => Json(json!({"task":snapshot,"report":report,"reportJson":result.artifact,
                    "programs": if result.summary["status"] == "passed" {result.programs.clone()} else {vec![]}})).into_response(),
                Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR,"EXPORT_REPORT","Stored export report could not be read."),
            }
        }
        Ok(_) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "TASK_KIND",
            "This task is not a LinuxCNC export.",
        ),
        Err(failure) => task_error(failure),
    }
}
async fn cancel_plan(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Response {
    match state.planning.cancel(&id) {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn plan_result(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Response {
    match state.planning.result(&id) {
        Ok((snapshot, _)) if snapshot.verification.is_some() || snapshot.export.is_some() => error(StatusCode::UNPROCESSABLE_ENTITY, "TASK_KIND", "This task contains report evidence, not a plan preview."),
        Ok((snapshot, result)) => Json(
            json!({ "task": snapshot, "coordinateSpace": "workpiece-mm-z-up",
            "motions": result.motions, "stockSlices": result.inspection.slices.iter().map(|s| &s.info).collect::<Vec<_>>() }),
        )
        .into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn stock_slice(
    State(state): State<AppState>,
    RoutePath((id, slice)): RoutePath<(String, String)>,
) -> Response {
    match state.planning.result(&id) {
        Ok((snapshot, result)) => match result.inspection.slices.iter().find(|s| s.info.id == slice)
        {
            Some(value) => Json(
                json!({ "task": snapshot, "coordinateSpace": "workpiece-mm-z-up", "slice": value }),
            )
            .into_response(),
            None => error(
                StatusCode::NOT_FOUND,
                "SLICE_NOT_FOUND",
                "This plan has no slice with that ID.",
            ),
        },
        Err(failure) => task_error(failure),
    }
}
async fn plan_artifact(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
) -> Response {
    match state.planning.result(&id) {
        Ok((_, result)) if result.plan_artifact.is_some() => {
            let artifact = result.plan_artifact.as_ref().unwrap().clone();
            let length = match artifact.byte_len() {
                Ok(length) => length,
                Err(_) => {
                    return error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "PLAN_ARTIFACT_IO",
                        "The retained plan file could not be read.",
                    );
                }
            };
            match artifact.body().await {
                Ok(body) => (
                    [
                        (header::CONTENT_TYPE, "application/json".to_owned()),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=plan.json".to_owned(),
                        ),
                        (header::CONTENT_LENGTH, length.to_string()),
                    ],
                    body,
                )
                    .into_response(),
                Err(_) => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "PLAN_ARTIFACT_IO",
                    "The retained plan file could not be read.",
                ),
            }
        }
        Ok((snapshot, result)) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    header::CONTENT_DISPOSITION,
                    if snapshot.export.is_some() {
                        "attachment; filename=export-report.json"
                    } else if snapshot.verification.is_some() {
                        "attachment; filename=verification.json"
                    } else {
                        "attachment; filename=plan.json"
                    },
                ),
            ],
            result.artifact.clone(),
        )
            .into_response(),
        Err(failure) => task_error(failure),
    }
}
async fn library_metadata(
    State(state): State<AppState>,
    body: Result<Json<library::Request>, JsonRejection>,
) -> Response {
    library_request(state, body, false).await
}
async fn library_job(
    State(state): State<AppState>,
    body: Result<Json<library::Request>, JsonRejection>,
) -> Response {
    library_request(state, body, true).await
}
async fn library_request(
    state: AppState,
    body: Result<Json<library::Request>, JsonRejection>,
    has_job: bool,
) -> Response {
    let request = match body {
        Ok(Json(r)) => r,
        Err(e) => return error(e.status(), "REQUEST_JSON", &e.body_text()),
    };
    if let Err(e) = state.planning.validate_identity(
        &request.api_version,
        &request.instance_id,
        &request.request_id,
        0,
    ) {
        return task_error(e);
    }
    if request.command.has_job() != has_job {
        return error(
            StatusCode::BAD_REQUEST,
            "LIBRARY_ROUTE",
            "This library operation belongs to another route.",
        );
    }
    let Some(library) = state.library else {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "LIBRARY_UNAVAILABLE",
            "No local tool library is configured.",
        );
    };
    let Ok(permit) = state.workers.clone().try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "SERVICE_BUSY",
            "The local service is handling other document or library requests. Retry shortly.",
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        library.execute(request.command)
    })
    .await;
    match result {
        Ok(Ok(data))=>Json(json!({"apiVersion":API_VERSION,"engineVersion":ENGINE_VERSION,"instanceId":request.instance_id,"requestId":request.request_id,"data":data})).into_response(),
        Ok(Err(e))=>error(StatusCode::from_u16(e.0).unwrap(),&e.1,&e.2),
        Err(_)=>error(StatusCode::INTERNAL_SERVER_ERROR,"LIBRARY_FAILURE","The library request could not finish. Reload before retrying a write; it may have completed."),
    }
}
async fn document_request(
    State(state): State<AppState>,
    body: Result<Json<DocumentRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(value)) => value,
        Err(rejection) => return error(rejection.status(), "REQUEST_JSON", &rejection.body_text()),
    };
    if request.api_version != API_VERSION {
        return error(
            StatusCode::CONFLICT,
            "API_VERSION",
            "UI and service API versions differ. Rebuild the UI and restart.",
        );
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || !request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || request.revision > 9_007_199_254_740_991
    {
        return error(
            StatusCode::BAD_REQUEST,
            "REQUEST_IDENTITY",
            "A short request ID and a safe integer revision are required.",
        );
    }
    let permit = match state.workers.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "SERVICE_BUSY",
                "The engine is inspecting other requests. Retry shortly.",
            );
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        // Keep the permit even if the HTTP future is dropped; abort is not worker cancellation.
        let _permit = permit;
        document::execute(request.command)
    })
    .await;
    let mut envelope = json!({ "apiVersion": API_VERSION, "engineVersion": ENGINE_VERSION,
        "requestId": request.request_id, "revision": request.revision });
    match result {
        Ok(Ok(data)) => {
            envelope["data"] = data;
            Json(envelope).into_response()
        }
        Ok(Err(diagnostic)) => {
            envelope["diagnostic"] = json!(diagnostic);
            (StatusCode::UNPROCESSABLE_ENTITY, Json(envelope)).into_response()
        }
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ENGINE_FAILURE",
            "The engine could not finish this request. Your draft is unchanged.",
        ),
    }
}
async fn asset(State(state): State<AppState>, request: Request) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = if request.uri().path() == "/index.html" {
        "/"
    } else {
        request.uri().path()
    };
    match state.assets.get(path) {
        Some((mime, bytes)) => (
            [(header::CONTENT_TYPE, *mime)],
            if request.method() == Method::HEAD {
                Body::empty()
            } else {
                Body::from(bytes.clone())
            },
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
