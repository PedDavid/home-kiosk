//! Writes the API fixtures the screenshot tests serve (`frontend/tests/screens/`).
//!
//! The responses come from the demo data in `src/demo.rs`, rendered through the real router at
//! the fixed moment the tests pin the browser clock to, so the pages see the same shapes and
//! times the backend would send. Run it again after changing the demo data:
//!
//! ```text
//! cargo run --example screen_fixtures
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use axum::{Router, body::Body, http::Request};
use chrono::{DateTime, Utc};
use home_hq::{
    AppState, router,
    settings::{EXAMPLE, Settings},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Keep in step with `NOW` in `frontend/tests/screens/kiosk.mjs`.
const NOW: &str = "2026-11-05T17:30:00Z";

async fn get(app: &Router, uri: &str) -> Vec<u8> {
    let res = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(res.status().is_success(), "{uri}: {}", res.status());
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

fn pretty(bytes: &[u8]) -> Vec<u8> {
    let v: Value = serde_json::from_slice(bytes).unwrap();
    let mut out = serde_json::to_vec_pretty(&v).unwrap();
    out.push(b'\n');
    out
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let now: DateTime<Utc> = NOW.parse().unwrap();
    let app = router(AppState::new(
        Settings::parse(EXAMPLE).unwrap(),
        None,
        Arc::new(move || now),
    ));
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "frontend", "tests", "screens", "fixtures"]
        .iter()
        .collect();
    std::fs::create_dir_all(dir.join("photos")).unwrap();

    for (name, uri) in [
        ("state", "/api/state"),
        ("calendar", "/api/calendar"),
        ("weather", "/api/weather"),
        ("home", "/api/home"),
        ("energy", "/api/home/energy"),
        ("frames", "/api/frames"),
        ("alerts", "/api/alerts"),
    ] {
        let body = pretty(&get(&app, uri).await);
        std::fs::write(dir.join(format!("{name}.json")), body).unwrap();
    }

    let state: Value = serde_json::from_slice(&get(&app, "/api/state").await).unwrap();
    for p in state["photos"].as_array().unwrap() {
        let id = p["id"].as_str().unwrap();
        let full = get(&app, &format!("/api/photos/{id}")).await;
        std::fs::write(dir.join("photos").join(format!("{id}.svg")), full).unwrap();
        let small = get(&app, &format!("/api/photos/{id}?s=1")).await;
        std::fs::write(dir.join("photos").join(format!("{id}-s.svg")), small).unwrap();
    }
    for id in ["camera.front_door", "camera.garden"] {
        let svg = get(&app, &format!("/api/ha/camera?id={id}")).await;
        std::fs::write(dir.join("photos").join(format!("{id}.svg")), svg).unwrap();
    }
    println!("wrote {}", dir.display());
}
