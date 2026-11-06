//! `/api` routes, matching upstream's Worker paths and response shapes.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State, WebSocketUpgrade, ws::Message},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, demo};

type ApiResult = Result<Response, ApiError>;

/// A handler's failure: rendered as upstream's `{ "error": "..." }` with a status.
struct ApiError(&'static str, StatusCode);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        bad(self.0, self.1)
    }
}

fn ok(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

fn bad(msg: &str, status: StatusCode) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": msg })),
    )
        .into_response()
}

/// Upstream reads bodies leniently: anything that isn't JSON is an empty object.
fn body(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|_| json!({}))
}

fn image(content_type: &'static str, data: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox",
            ),
        ],
        data,
    )
        .into_response()
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/state", get(state))
        .route("/calendar", get(calendar))
        .route("/weather", get(weather))
        .route("/events", post(add_event))
        .route("/events/{id}", delete(delete_event))
        .route("/bins/out", post(bins_out))
        .route("/frames", get(list_frames).post(frame_beat))
        .route("/frames/test-notify", post(test_notify))
        .route("/frames/{id}", delete(forget_frame))
        .route("/alerts", get(alerts))
        .route("/alerts/ws", get(alerts_ws))
        .route("/notes", post(put_note))
        .route("/notes/{id}", delete(clear_note))
        .route("/notes/{id}/restore", post(restore_note))
        .route("/groceries", post(put_grocery))
        .route("/groceries/clear", post(clear_groceries))
        .route("/photos/{id}", get(photo).patch(favourite))
        .route("/home", get(home))
        .route("/home/energy", get(energy))
        .route("/home/device/{id}", post(control_device))
        .route("/home/media/{id}", post(media))
        .route("/home/{what}", post(home_action))
        .route("/ha/camera", get(camera))
        .route("/ha/art", get(art))
        .fallback(|| async { bad("not found", StatusCode::NOT_FOUND) })
}

async fn state(State(app): State<AppState>) -> Response {
    ok(app.demo.lock().await.state(app.now()))
}

async fn calendar(State(app): State<AppState>) -> Response {
    ok(app.demo.lock().await.calendar(app.now()))
}

async fn weather(State(app): State<AppState>) -> Response {
    if let Some(w) = &app.weather {
        return ok(w.get().await);
    }
    ok(app.demo.lock().await.weather(app.now()))
}

async fn add_event(State(app): State<AppState>, b: Bytes) -> ApiResult {
    app.demo
        .lock()
        .await
        .add_event(&body(&b))
        .map_err(|e| ApiError(e, StatusCode::BAD_REQUEST))?;
    Ok(ok(json!({ "ok": true })))
}

async fn delete_event(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    app.demo.lock().await.delete_event(&id);
    ok(json!({ "ok": true }))
}

async fn bins_out(State(app): State<AppState>, b: Bytes) -> ApiResult {
    let b = body(&b);
    let date = b["date"]
        .as_str()
        .filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
    let date = date.ok_or(ApiError("date required", StatusCode::BAD_REQUEST))?;
    let out = b["out"].as_bool() != Some(false);
    Ok(ok(app.demo.lock().await.set_bins_out(date, out, app.now())))
}

async fn frame_beat(State(app): State<AppState>, b: Bytes) -> ApiResult {
    app.demo
        .lock()
        .await
        .frame_beat(&body(&b), app.now())
        .map_err(|e| ApiError(e, StatusCode::BAD_REQUEST))?;
    Ok(ok(json!({ "ok": true, "replaced": 0 })))
}

async fn list_frames(State(app): State<AppState>) -> Response {
    ok(json!(app.demo.lock().await.frames()))
}

async fn test_notify() -> Response {
    bad("Frame notifications are not set up yet", StatusCode::NOT_FOUND)
}

async fn forget_frame(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    app.demo.lock().await.forget_frame(&id);
    ok(json!({ "ok": true }))
}

/// The alert counter, for a frame whose WebSocket is down. Nothing raises alerts yet.
async fn alerts() -> Response {
    ok(json!({ "seq": 0 }))
}

/// The frame's alert socket: sends the current counter on connect and answers its "ping"s.
async fn alerts_ws(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut socket| async move {
        if socket.send(Message::Text(r#"{"seq":0}"#.into())).await.is_err() {
            return;
        }
        while let Some(Ok(msg)) = socket.recv().await {
            if let Message::Text(t) = &msg
                && t.as_str() == "ping"
                && socket.send(Message::Text("pong".into())).await.is_err()
            {
                return;
            }
            if matches!(msg, Message::Close(_)) {
                return;
            }
        }
    })
}

async fn put_note(State(app): State<AppState>, b: Bytes) -> ApiResult {
    let now = app.now();
    let notes = app
        .demo
        .lock()
        .await
        .put_note(&body(&b), now)
        .map_err(|e| ApiError(e, StatusCode::BAD_REQUEST))?;
    Ok(ok(json!({ "ok": true, "notes": notes })))
}

async fn clear_note(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let notes = app.demo.lock().await.clear_note(&id, false, app.now());
    ok(json!({ "ok": true, "notes": notes }))
}

async fn restore_note(State(app): State<AppState>, Path(id): Path<String>) -> Response {
    let notes = app.demo.lock().await.clear_note(&id, true, app.now());
    ok(json!({ "ok": true, "notes": notes }))
}

async fn put_grocery(State(app): State<AppState>, b: Bytes) -> ApiResult {
    app.demo
        .lock()
        .await
        .put_grocery(&body(&b))
        .map_err(|e| ApiError(e, StatusCode::BAD_REQUEST))?;
    Ok(ok(json!({ "ok": true })))
}

async fn clear_groceries(State(app): State<AppState>, b: Bytes) -> Response {
    let b = body(&b);
    let ids: Vec<String> = b["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .take(500)
        .collect();
    app.demo
        .lock()
        .await
        .clear_groceries(&ids, b["cleared"] == json!(false));
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct PhotoQuery {
    s: Option<String>,
}

async fn photo(State(app): State<AppState>, Path(id): Path<String>, Query(q): Query<PhotoQuery>) -> Response {
    match app.demo.lock().await.photo_svg(&id, q.s.is_some()) {
        Some(svg) => (
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            svg,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn favourite(State(app): State<AppState>, Path(id): Path<String>, b: Bytes) -> Response {
    let fav = body(&b)["fav"].as_bool().unwrap_or(false);
    app.demo.lock().await.set_favourite(&id, fav);
    ok(json!({ "ok": true }))
}

async fn home(State(app): State<AppState>) -> Response {
    ok(app.demo.lock().await.home(app.now()))
}

async fn energy(State(app): State<AppState>) -> Response {
    ok(app.demo.lock().await.energy(app.now()))
}

async fn control_device(State(app): State<AppState>, Path(id): Path<String>, b: Bytes) -> ApiResult {
    app.demo.lock().await.control_device(&id, &body(&b)).map_err(|e| {
        let status = if e == "unknown device" {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        ApiError(e, status)
    })?;
    Ok(ok(json!({ "ok": true })))
}

async fn media(Path(id): Path<String>) -> Response {
    if id == "media_player.living_room" {
        ok(json!({ "ok": true }))
    } else {
        bad("unknown player or action", StatusCode::NOT_FOUND)
    }
}

async fn home_action(State(app): State<AppState>, Path(what): Path<String>, b: Bytes) -> ApiResult {
    app.demo
        .lock()
        .await
        .home_action(&what, &body(&b))
        .map_err(|e| ApiError(e, StatusCode::NOT_FOUND))?;
    Ok(ok(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct IdQuery {
    id: Option<String>,
}

async fn camera(State(app): State<AppState>, Query(q): Query<IdQuery>) -> Response {
    match q.id.as_deref().and_then(|id| demo::camera_svg(id, app.now())) {
        Some(svg) => image("image/svg+xml", svg),
        None => bad("unknown camera", StatusCode::NOT_FOUND),
    }
}

async fn art() -> Response {
    StatusCode::NOT_FOUND.into_response()
}
