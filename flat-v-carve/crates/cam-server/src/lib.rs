//! Static hosting for the browser build.
//!
//! The workspace plans in the process that shows the work: the browser build
//! runs the engine as WebAssembly in a worker, and the native GUI spawns its
//! own compute worker. The service therefore serves files and nothing else.
//! The schema-3 planning/verification endpoints and the server-side tool
//! library are gone with the schema diet (docs/flat-v-carve/job-model.md).
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::Path,
    sync::Arc,
};

pub type Assets = HashMap<String, (&'static str, Bytes)>;

pub mod serve;

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map" | "manifest") => "application/json",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Read only build assets at startup. HTTP paths never become filesystem paths.
pub fn load_assets(directory: &Path) -> io::Result<Assets> {
    let mut assets = HashMap::new();
    let mut total = 0;
    collect_assets(directory, "", &mut assets, &mut total)?;
    Ok(assets)
}

/// Walks one prebuilt UI directory. Symlinks are refused so an asset directory
/// cannot reach outside itself, and the whole tree stays within the service's
/// fixed asset budget.
fn collect_assets(
    root: &Path,
    relative: &str,
    assets: &mut Assets,
    total: &mut usize,
) -> io::Result<()> {
    let directory = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    for entry in fs::read_dir(&directory)? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| io::Error::other("UI asset names must be UTF-8"))?;
        let child = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let path = root.join(&child);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(io::Error::other(format!(
                "UI assets must not be symlinks: {child}"
            )));
        }
        if metadata.is_dir() {
            collect_assets(root, &child, assets, total)?;
        } else if metadata.is_file() {
            let mut bytes = Vec::new();
            fs::File::open(&path)?
                .take((32_000_000 - *total + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 32_000_000 - *total {
                return Err(io::Error::other(
                    "UI assets must be regular files totaling at most 32 MB",
                ));
            }
            *total += bytes.len();
            let key = if child == "index.html" {
                "/".to_owned()
            } else {
                format!("/{child}")
            };
            assets.insert(key, (content_type(&child), bytes.into()));
        } else {
            return Err(io::Error::other("UI assets must be regular files"));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct AppState {
    authority: String,
    origin: String,
    assets: Arc<Assets>,
}

/// A loopback-only file server. Host and Origin must name the exact address the
/// service printed, so a page on another origin cannot drive it.
pub fn router(port: u16, assets: Assets) -> io::Result<Router> {
    let authority = format!("127.0.0.1:{port}");
    let state = AppState {
        origin: format!("http://{authority}"),
        authority,
        assets: Arc::new(assets),
    };
    Ok(Router::new()
        .fallback(asset)
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state))
}

async fn boundary(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let header_is = |name: &str, expected: &str| {
        headers.get_all(name).iter().count() == 1
            && headers.get(name).and_then(|v| v.to_str().ok()) == Some(expected)
    };
    if !header_is("host", &state.authority)
        || (headers.contains_key("origin") && !header_is("origin", &state.origin))
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site" || v == "same-site")
    {
        return (
            StatusCode::FORBIDDEN,
            "Use the exact loopback URL printed by the local service.",
        )
            .into_response();
    }
    next.run(request).await
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
