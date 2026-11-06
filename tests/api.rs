//! Route-level tests against the full router.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{TimeZone, Utc};
use home_hq::{
    AppState, router,
    settings::{EXAMPLE, Settings},
    share::ImmichPublicUrl,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const KEY: &str = "s3cret/key";

fn app(key: Option<&str>) -> Router {
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap();
    router(AppState::new(
        Settings::parse(EXAMPLE).unwrap(),
        key.map(String::from),
        Arc::new(move || now),
    ))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes().to_vec(),
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).header("x-hq-key", KEY).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::post(uri)
        .header("x-hq-key", KEY)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_of(app: &Router, req: Request<Body>) -> Value {
    let (status, _, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

// ------------------------------------------------------------------ auth

#[tokio::test]
async fn api_is_locked_without_a_key() {
    let app = app(Some(KEY));
    let (status, _, body) = send(&app, Request::get("/api/state").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({ "error": "locked" })
    );

    let wrong = Request::get("/api/state")
        .header("x-hq-key", "nope")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, wrong).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn key_in_page_url_becomes_a_cookie() {
    let app = app(Some(KEY));
    let (status, headers, _) = send(
        &app,
        Request::get("/?frame=kitchen&key=s3cret%2Fkey")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers[header::LOCATION], "/?frame=kitchen");
    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with("hq=s3cret%2Fkey;"), "{cookie}");
    assert!(cookie.contains("HttpOnly") && cookie.contains("Secure") && cookie.contains("Max-Age=315360000"));

    let with_cookie = Request::get("/api/state")
        .header(header::COOKIE, "other=1; hq=s3cret%2Fkey")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, with_cookie).await.0, StatusCode::OK);
}

#[tokio::test]
async fn wrong_key_in_page_url_is_refused() {
    let app = app(Some(KEY));
    let (status, _, _) = send(&app, Request::get("/m?key=wrong").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn no_key_configured_leaves_the_api_open() {
    let app = app(None);
    let (status, _, _) = send(&app, Request::get("/api/state").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn health_needs_no_key() {
    let (status, _, body) = send(&app(Some(KEY)), Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"ok");
}

// ------------------------------------------------------------------ shapes

#[tokio::test]
async fn state_has_the_upstream_shape() {
    let s = json_of(&app(Some(KEY)), get("/api/state")).await;
    for k in [
        "people",
        "stores",
        "catOrder",
        "catalog",
        "usuals",
        "groceries",
        "photos",
        "hasHome",
        "scenes",
        "cameras",
        "bins",
        "notes",
        "awayOffMinutes",
    ] {
        assert!(s.get(k).is_some(), "missing {k}");
    }
    let p = &s["people"][0];
    for k in ["id", "name", "color", "rgb", "ink", "tracked"] {
        assert!(p.get(k).is_some(), "person missing {k}");
    }
    assert_eq!(s["catOrder"]["all"][0], "Produce");
    assert_eq!(s["catalog"][0], json!(["Apples", "Produce"]));
    assert_eq!(s["bins"]["next"][0]["date"], "2026-11-06");
}

#[tokio::test]
async fn calendar_weather_and_home_have_the_upstream_shape() {
    let app = app(Some(KEY));
    let c = json_of(&app, get("/api/calendar")).await;
    assert!(c["events"].as_array().unwrap().len() > 10);
    assert!(c["errors"].as_array().unwrap().is_empty());
    let e = c["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e.get("allDay").is_none())
        .unwrap();
    for k in ["id", "title", "people", "start", "end"] {
        assert!(e.get(k).is_some(), "event missing {k}");
    }

    let w = json_of(&app, get("/api/weather")).await;
    assert_eq!(w["day"], "2026-11-05");
    for k in [
        "units", "t", "p", "c", "hi", "lo", "feels", "humidity", "wind", "now", "warnings",
    ] {
        assert!(w.get(k).is_some(), "weather missing {k}");
    }

    let h = json_of(&app, get("/api/home")).await;
    assert_eq!(h["online"], true);
    for k in [
        "rooms",
        "devices",
        "scenes",
        "cameras",
        "climate",
        "presence",
        "sensors",
        "energy",
        "players",
        "checks",
        "alertConfig",
    ] {
        assert!(h.get(k).is_some(), "home missing {k}");
    }
    let en = json_of(&app, get("/api/home/energy")).await;
    assert_eq!(en["hours"].as_array().unwrap().len(), 18); // hours 0..=17 local
}

#[tokio::test]
async fn unknown_api_route_is_a_json_404() {
    let (status, _, body) = send(&app(Some(KEY)), get("/api/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({ "error": "not found" })
    );
}

// ------------------------------------------------------------------ writes

#[tokio::test]
async fn shopping_writes_show_up_in_state() {
    let app = app(Some(KEY));
    json_of(
        &app,
        post(
            "/api/groceries",
            json!({ "id": "t1", "name": "Lemons", "cat": "Produce", "qty": 3, "store": "market" }),
        ),
    )
    .await;
    let s = json_of(&app, get("/api/state")).await;
    let g = s["groceries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == "t1")
        .unwrap()
        .clone();
    assert_eq!(
        g,
        json!({ "id": "t1", "name": "Lemons", "cat": "Produce", "qty": 3, "store": "market", "note": "", "done": false })
    );

    json_of(&app, post("/api/groceries/clear", json!({ "ids": ["t1"] }))).await;
    let s = json_of(&app, get("/api/state")).await;
    assert!(!s["groceries"].as_array().unwrap().iter().any(|g| g["id"] == "t1"));

    let (status, _, _) = send(&app, post("/api/groceries", json!({ "id": "t2" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bins_can_be_marked_out_and_back() {
    let app = app(Some(KEY));
    let b = json_of(&app, post("/api/bins/out", json!({ "date": "2026-11-06" }))).await;
    assert_eq!(b["out"]["date"], "2026-11-06");
    let b = json_of(
        &app,
        post("/api/bins/out", json!({ "date": "2026-11-06", "out": false })),
    )
    .await;
    assert!(b["out"].is_null());
    assert_eq!(
        send(&app, post("/api/bins/out", json!({}))).await.0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn notes_round_trip() {
    let app = app(Some(KEY));
    let r = json_of(
        &app,
        post(
            "/api/notes",
            json!({ "id": "note123", "text": " Hi ", "from": "alex", "to": "sam" }),
        ),
    )
    .await;
    assert!(
        r["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == "note123" && n["text"] == "Hi")
    );
    let del = Request::delete("/api/notes/note123")
        .header("x-hq-key", KEY)
        .body(Body::empty())
        .unwrap();
    let r = json_of(&app, del).await;
    assert!(!r["notes"].as_array().unwrap().iter().any(|n| n["id"] == "note123"));
    let r = json_of(&app, post("/api/notes/note123/restore", json!({}))).await;
    assert!(r["notes"].as_array().unwrap().iter().any(|n| n["id"] == "note123"));
}

#[tokio::test]
async fn frames_check_in_and_list() {
    let app = app(Some(KEY));
    let r = json_of(
        &app,
        post(
            "/api/frames",
            json!({ "id": "abc123def", "name": "Kitchen", "version": "1.23" }),
        ),
    )
    .await;
    assert_eq!(r, json!({ "ok": true, "replaced": 0 }));
    let list = json_of(&app, get("/api/frames")).await;
    assert_eq!(list[0]["name"], "Kitchen");
    assert_eq!(
        list[0]["seen"],
        Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap().timestamp_millis()
    );
}

#[tokio::test]
async fn controls_change_the_home_view() {
    let app = app(Some(KEY));
    json_of(&app, post("/api/home/device/light.porch", json!({ "on": true }))).await;
    let h = json_of(&app, get("/api/home")).await;
    assert!(
        h["devices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == "light.porch" && d["on"] == true)
    );
    assert_eq!(
        send(&app, post("/api/home/device/light.nope", json!({ "on": true })))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn photos_and_cameras_are_images() {
    let app = app(Some(KEY));
    let (status, headers, _) = send(&app, get("/api/photos/demo0?s=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    let (status, headers, _) = send(&app, get("/api/ha/camera?id=camera.garden")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        send(&app, get("/api/ha/camera?id=camera.nope")).await.0,
        StatusCode::NOT_FOUND
    );
}

// ------------------------------------------------------------------ alerts

// ------------------------------------------------------------------ photo sharing

const ASSET: &str = "9b2d8a4e-3f1c-4e7a-b0d5-6c8f1e2a3b4c";

/// An app whose demo0 photo is an Immich asset, with or without `IMMICH_PUBLIC_URL`.
async fn sharing_app(public_url: Option<&str>) -> Router {
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap();
    let state = AppState::new(
        Settings::parse(EXAMPLE).unwrap(),
        Some(KEY.into()),
        Arc::new(move || now),
    )
    .with_immich_public_url(public_url.map(|u| ImmichPublicUrl::parse(u).unwrap().unwrap()));
    state.demo.lock().await.set_immich_id("demo0", ASSET);
    router(state)
}

fn share_urls(state: &Value) -> Vec<(String, Option<String>)> {
    state["photos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().to_string(),
                p["shareUrl"].as_str().map(String::from),
            )
        })
        .collect()
}

#[tokio::test]
async fn immich_photos_get_a_share_url_when_the_public_url_is_set() {
    let app = sharing_app(Some("https://photos.example.com/")).await;
    let urls = share_urls(&json_of(&app, get("/api/state")).await);
    for (id, url) in urls {
        if id == "demo0" {
            assert_eq!(
                url.as_deref(),
                Some(&*format!("https://photos.example.com/photos/{ASSET}"))
            );
        } else {
            assert_eq!(url, None, "{id} has no Immich asset ID");
        }
    }
}

#[tokio::test]
async fn no_share_url_without_the_public_url() {
    let app = sharing_app(None).await;
    let urls = share_urls(&json_of(&app, get("/api/state")).await);
    assert!(!urls.is_empty());
    assert!(urls.iter().all(|(_, u)| u.is_none()), "{urls:?}");
}

#[tokio::test]
async fn demo_photos_have_no_share_url() {
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap();
    let state = AppState::new(
        Settings::parse(EXAMPLE).unwrap(),
        Some(KEY.into()),
        Arc::new(move || now),
    )
    .with_immich_public_url(ImmichPublicUrl::parse("https://photos.example.com").unwrap());
    let urls = share_urls(&json_of(&router(state), get("/api/state")).await);
    assert!(urls.iter().all(|(_, u)| u.is_none()), "{urls:?}");
}

#[tokio::test]
async fn the_state_holds_no_key_next_to_the_share_url() {
    let app = sharing_app(Some("https://photos.example.com")).await;
    let (_, _, body) = send(&app, get("/api/state")).await;
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("/photos/"));
    assert!(!body.contains(KEY));
}

#[tokio::test]
async fn alert_poll_returns_the_counter() {
    assert_eq!(json_of(&app(Some(KEY)), get("/api/alerts")).await, json!({ "seq": 0 }));
}

#[tokio::test]
async fn alert_socket_sends_the_counter_and_answers_pings() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app(Some(KEY))).into_future());

    let mut req = format!("ws://{addr}/api/alerts/ws").into_client_request().unwrap();
    req.headers_mut().insert("cookie", "hq=s3cret%2Fkey".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap(), Message::Text(r#"{"seq":0}"#.into()));
    ws.send(Message::Text("ping".into())).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap(), Message::Text("pong".into()));

    let unauth = format!("ws://{addr}/api/alerts/ws").into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(unauth).await.is_err());
}
