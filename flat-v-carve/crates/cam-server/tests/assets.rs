use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cam-assets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("web")).unwrap();
        fs::create_dir_all(path.join("pkg")).unwrap();
        for (name, bytes) in [
            ("index.html", b"<html>root</html>".as_slice()),
            ("web/index.html", b"<html>app</html>".as_slice()),
            ("web/worker.js", b"self.onmessage=()=>{};".as_slice()),
            ("pkg/cam_gui_bg.wasm", b"\0asm".as_slice()),
        ] {
            fs::write(path.join(name), bytes).unwrap();
        }
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn prebuilt_directory_assets_and_root_alias_use_the_local_boundary() {
    let fixture = Fixture::new();
    let assets = cam_server::load_assets(&fixture.0).unwrap();
    let app = cam_server::router(4848, assets).unwrap();
    for (path, mime, bytes) in [
        (
            "/",
            "text/html; charset=utf-8",
            b"<html>root</html>".as_slice(),
        ),
        (
            "/index.html",
            "text/html; charset=utf-8",
            b"<html>root</html>".as_slice(),
        ),
        (
            "/web/index.html",
            "text/html; charset=utf-8",
            b"<html>app</html>".as_slice(),
        ),
        (
            "/web/worker.js",
            "text/javascript; charset=utf-8",
            b"self.onmessage=()=>{};".as_slice(),
        ),
        (
            "/pkg/cam_gui_bg.wasm",
            "application/wasm",
            b"\0asm".as_slice(),
        ),
    ] {
        let request = Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:4848")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()["content-type"], mime, "{path}");
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            bytes,
            "{path}"
        );
    }
    let request = Request::builder()
        .uri("/")
        .header("host", "untrusted.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn a_directory_without_a_root_index_has_no_root_alias() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("index.html")).unwrap();
    let assets = cam_server::load_assets(&fixture.0).unwrap();
    assert!(!assets.contains_key("/"));
    assert!(assets.contains_key("/web/index.html"));
}
