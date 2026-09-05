use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use cam_core::{
    job::Job,
    svg::{ImportOptions, import_svg},
};
use cam_server::document::{API_VERSION, Command, ENGINE_VERSION, REQUEST_BYTES, execute};
use serde_json::{Value, json};
use tower::ServiceExt;

const SVG: &str = include_str!("../../../fixtures/m2/inkscape-export.svg");
fn app() -> Router {
    cam_server::router(4848, Default::default()).unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: String,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn token(app: &Router) -> String {
    call(
        app,
        "GET",
        "/api/v1/session",
        &[("host", "127.0.0.1:4848")],
        String::new(),
    )
    .await
    .1["sessionToken"]
        .as_str()
        .unwrap()
        .into()
}
async fn document(app: &Router, command: Value) -> (StatusCode, Value) {
    let token = token(app).await;
    call(app, "POST", "/api/v1/document", &[("host", "127.0.0.1:4848"), ("origin", "http://127.0.0.1:4848"), ("x-cam-session", &token), ("content-type", "application/json")],
        json!({ "apiVersion": API_VERSION, "requestId": "test-request", "revision": 42, "command": command }).to_string()).await
}

#[tokio::test]
async fn local_boundary_requires_exact_host_origin_and_session() {
    let app = app();
    for headers in [
        vec![],
        vec![("host", "evil.example:4848")],
        vec![("host", "127.0.0.1:4848"), ("origin", "null")],
        vec![
            ("host", "127.0.0.1:4848"),
            ("origin", "http://localhost:4848"),
        ],
        vec![("host", "127.0.0.1:4848"), ("sec-fetch-site", "cross-site")],
        vec![("host", "127.0.0.1:4848"), ("sec-fetch-site", "same-site")],
        vec![("host", "127.0.0.1:4848"), ("host", "evil.example")],
    ] {
        assert_eq!(
            call(&app, "GET", "/api/v1/session", &headers, String::new())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/v1/capabilities",
            &[("host", "127.0.0.1:4848")],
            String::new()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let old = token(&app).await;
    let restarted = self::app();
    assert_ne!(old, token(&restarted).await);
    assert_eq!(
        call(
            &restarted,
            "GET",
            "/api/v1/capabilities",
            &[("host", "127.0.0.1:4848"), ("x-cam-session", &old)],
            String::new()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn capability_envelope_has_limits_and_no_unimplemented_operations() {
    let app = app();
    let token = token(&app).await;
    let (status, caps) = call(
        &app,
        "GET",
        "/api/v1/capabilities",
        &[("host", "127.0.0.1:4848"), ("x-cam-session", &token)],
        String::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(caps["apiVersion"], API_VERSION);
    assert_eq!(caps["engineVersion"], ENGINE_VERSION);
    assert_eq!(caps["limits"]["svgBytes"], cam_core::svg::MAX_SVG_BYTES);
    assert_eq!(caps["limits"]["requestBytes"], REQUEST_BYTES);
    assert_eq!(caps["planningStages"], json!(["endmill", "combined"]));
    assert_eq!(caps["planning"]["concurrentPlans"], 1);
    assert_eq!(caps["planning"]["maxPending"], 4);
    for field in ["verificationScopes", "exportFormats"] {
        assert_eq!(
            caps[field],
            if field == "verificationScopes" {
                json!(["continuous-stock"])
            } else {
                json!([])
            }
        );
    }
    assert_eq!(
        caps["verification"]["defaultOptions"],
        json!(cam_core::verification::VerificationOptions::default())
    );
    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/api/v1/plan",
        "/crates/cam-core/src/lib.rs",
    ] {
        assert_eq!(
            call(
                &app,
                "GET",
                path,
                &[("host", "127.0.0.1:4848"), ("x-cam-session", &token)],
                String::new()
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn verification_submission_has_a_small_metadata_body_limit() {
    let app = app();
    let token = token(&app).await;
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/verifications",
        &[
            ("host", "127.0.0.1:4848"),
            ("x-cam-session", &token),
            ("content-type", "application/json"),
        ],
        json!({"excess":"x".repeat(17_000)}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn import_and_display_are_exact_engine_projections() {
    let app = app();
    let options = ImportOptions::default();
    let (status, envelope) = document(&app, json!({ "operation": "import", "filename": "inkscape.svg", "svg": SVG, "options": options })).await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    assert_eq!(envelope["revision"], 42);
    assert_eq!(envelope["requestId"], "test-request");
    let job = Job::from_svg("inkscape.svg".into(), SVG.into(), options.clone()).unwrap();
    assert_eq!(envelope["data"]["job"], json!(job));
    let geometry = import_svg(SVG, &options, None).unwrap();
    let components = envelope["data"]["display"]["components"]
        .as_array()
        .unwrap();
    assert_eq!(components.len(), geometry.sources.len());
    for (component, source) in components.iter().zip(&geometry.sources) {
        assert_eq!(component["id"], source.id);
        for ((ring, source_ring), points) in component["rings"]
            .as_array()
            .unwrap()
            .iter()
            .zip(source.geometry.rings())
            .zip(source.geometry.rings_mm())
        {
            assert_eq!(ring["hole"], source_ring.is_hole());
            assert_eq!(ring["points"], json!(points));
        }
    }
    let (_, display) = document(
        &app,
        json!({ "operation": "display", "svg": SVG, "options": options }),
    )
    .await;
    assert_eq!(display["data"], envelope["data"]["display"]);
    assert_eq!(
        envelope["data"]["missingMachiningFields"],
        json!(job.inspect().unwrap().missing_machining_fields)
    );
}

#[tokio::test]
async fn migration_invalid_drafts_and_document_freshness_preserve_engine_meaning() {
    let app = app();
    let job = Job::from_svg("example.svg".into(), SVG.into(), ImportOptions::default()).unwrap();
    let mut old = json!(job);
    old["schema_version"] = json!(1);
    old.as_object_mut().unwrap().remove("vbit_planning");
    old.as_object_mut().unwrap().remove("endmill_planning");
    let (status, opened) = document(
        &app,
        json!({ "operation": "open", "json": old.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["data"]["job"], json!(job));
    let (_, valid) = document(&app, json!({ "operation": "validate", "job": job })).await;
    assert_eq!(valid["data"]["valid"], true);
    assert_eq!(valid["data"]["scope"], "editable-job-and-svg");
    assert_eq!(
        valid["data"]["documentFingerprint"],
        opened["data"]["documentFingerprint"]
    );
    let mut changed = json!(job);
    changed["name"] = json!("renamed");
    let (_, renamed) = document(&app, json!({ "operation": "validate", "job": changed })).await;
    assert_ne!(
        renamed["data"]["documentFingerprint"],
        valid["data"]["documentFingerprint"]
    );
    changed["stock"]["thickness_mm"] = json!(-2);
    let (_, invalid) = document(&app, json!({ "operation": "validate", "job": changed })).await;
    assert_eq!(invalid["data"]["valid"], false);
    assert_eq!(invalid["data"]["documentFingerprint"], Value::Null);
    assert_eq!(invalid["data"]["diagnostics"][0]["code"], "JOB_PARAMETER");
    let (status, _) = document(
        &app,
        json!({ "operation": "open", "json": changed.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn rejects_unknown_schema_fields_unsupported_svg_and_oversized_input() {
    let app = app();
    let job = Job::from_svg("example.svg".into(), SVG.into(), ImportOptions::default()).unwrap();
    let mut invalid = json!(job);
    invalid["schema_version"] = json!(99);
    assert_eq!(
        document(
            &app,
            json!({ "operation": "open", "json": invalid.to_string() })
        )
        .await
        .1["diagnostic"]["code"],
        "JOB_SCHEMA_VERSION"
    );
    invalid = json!(job);
    invalid["unrecognized"] = json!(true);
    assert_eq!(
        document(
            &app,
            json!({ "operation": "open", "json": invalid.to_string() })
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(document(&app, json!({ "operation": "display", "svg": "<svg><script>bad()</script></svg>", "options": ImportOptions::default() })).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(document(&app, json!({ "operation": "import", "filename": "huge.svg", "svg": " ".repeat(cam_core::svg::MAX_SVG_BYTES + 1), "options": ImportOptions::default() })).await.1["diagnostic"]["code"], "SVG_RESOURCE_LIMIT");
    let token = token(&app).await;
    let headers = [
        ("host", "127.0.0.1:4848"),
        ("x-cam-session", &token),
        ("content-type", "application/json"),
    ];
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/document",
            &headers,
            " ".repeat(REQUEST_BYTES + 1)
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let mut request = json!({ "apiVersion": "future", "requestId": "test", "revision": 0, "command": { "operation": "open", "json": "{}" } });
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/document",
            &headers,
            request.to_string()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    request["apiVersion"] = json!(API_VERSION);
    request["revision"] = json!(9_007_199_254_740_992u64);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/v1/document",
            &headers,
            request.to_string()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn configured_m4_jobs_roundtrip_without_changed_parameters_or_new_planning_claims() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/m4");
    let mut checked = 0;
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let json = std::fs::read_to_string(path).unwrap();
        let Ok(job) = Job::from_json(&json) else {
            continue;
        };
        if job.inspect().is_err() {
            continue;
        }
        let value = execute(Command::Open { json }).unwrap();
        assert_eq!(value["job"], json!(job));
        assert!(value.get("planningAvailable").is_none());
        checked += 1;
    }
    assert!(checked >= 10, "checked {checked} fixtures");
}
