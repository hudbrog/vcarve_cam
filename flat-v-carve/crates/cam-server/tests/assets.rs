use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[tokio::test]
async fn embedded_nested_assets_and_root_alias_use_the_local_boundary() {
    let app = cam_server::router(
        4848,
        cam_server::embedded_assets(&[
            ("index.html", b"<html>app</html>"),
            ("assets/nested/icon.svg", b"<svg/>"),
            ("assets/font.woff2", b"font"),
        ]),
    )
    .unwrap();
    for (path, mime, bytes) in [
        (
            "/",
            "text/html; charset=utf-8",
            b"<html>app</html>".as_slice(),
        ),
        (
            "/index.html",
            "text/html; charset=utf-8",
            b"<html>app</html>".as_slice(),
        ),
        (
            "/assets/nested/icon.svg",
            "image/svg+xml",
            b"<svg/>".as_slice(),
        ),
        ("/assets/font.woff2", "font/woff2", b"font".as_slice()),
    ] {
        let request = Request::builder()
            .uri(path)
            .header("host", "127.0.0.1:4848")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], mime);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            bytes
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
