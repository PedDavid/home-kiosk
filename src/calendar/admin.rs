//! `/api/calendars`: the calendar page's API. Behind the dashboard key like the rest of `/api`.
//! Secrets go in (app passwords, ICS links) and never come back out.
//!
//! | Route | Does |
//! |---|---|
//! | `GET /api/calendars` | accounts, people, the login in progress |
//! | `POST /api/calendars/login` `{ server }` | starts Nextcloud Login Flow v2 |
//! | `GET /api/calendars/login/qr` | the login page as a QR code (SVG) |
//! | `POST /api/calendars/login/poll` | checks it: `waiting`, `connected`, `expired` |
//! | `DELETE /api/calendars/login` | cancels it |
//! | `POST /api/calendars/password` `{ server, username, password }` | adds a CalDAV account with an app password |
//! | `POST /api/calendars/ics` `{ url, name, people }` | adds an ICS link |
//! | `PATCH /api/calendars/{account}/{calendar}` `{ people?, show?, addEvents? }` | changes a calendar |
//! | `POST /api/calendars/{account}/refresh` | finds calendars added on the server |
//! | `DELETE /api/calendars/{account}` | forgets an account |

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde_json::{Value, json};

use super::accounts::{Manager, Patch, Problem};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(view))
        .route("/login", post(start_login).delete(cancel_login))
        .route("/login/qr", get(login_qr))
        .route("/login/poll", post(poll_login))
        .route("/password", post(add_password))
        .route("/ics", post(add_ics))
        .route("/{account}", axum::routing::delete(remove))
        .route("/{account}/refresh", post(refresh))
        .route("/{account}/{calendar}", patch(update))
}

fn ok(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

fn fail(status: u16, msg: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": msg })),
    )
        .into_response()
}

fn done(r: Result<Value, Problem>) -> Response {
    match r {
        Ok(v) => ok(v),
        Err(p) => fail(p.status, &p.message),
    }
}

fn manager(app: &AppState) -> Result<Arc<Manager>, Box<Response>> {
    app.calendar_accounts
        .clone()
        .ok_or_else(|| Box::new(fail(404, "calendar setup isn't available on this server")))
}

fn body(b: &Bytes) -> Value {
    serde_json::from_slice(b).unwrap_or_else(|_| json!({}))
}

fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn people(v: &Value) -> Vec<String> {
    match &v["people"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|p| p.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

async fn view(State(app): State<AppState>) -> Response {
    match manager(&app) {
        Ok(m) => ok(m.view().await),
        Err(r) => *r,
    }
}

async fn start_login(State(app): State<AppState>, b: Bytes) -> Response {
    match manager(&app) {
        Ok(m) => done(m.start_login(text(&body(&b), "server")).await),
        Err(r) => *r,
    }
}

async fn cancel_login(State(app): State<AppState>) -> Response {
    match manager(&app) {
        Ok(m) => {
            m.cancel_login().await;
            ok(json!({ "ok": true }))
        }
        Err(r) => *r,
    }
}

async fn login_qr(State(app): State<AppState>) -> Response {
    let m = match manager(&app) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    match m.login_qr().await {
        Some(svg) => (
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            svg,
        )
            .into_response(),
        None => fail(404, "no login in progress"),
    }
}

async fn poll_login(State(app): State<AppState>) -> Response {
    match manager(&app) {
        Ok(m) => done(m.poll_login().await),
        Err(r) => *r,
    }
}

async fn add_password(State(app): State<AppState>, b: Bytes) -> Response {
    let b = body(&b);
    match manager(&app) {
        Ok(m) => done(
            m.add_with_password(text(&b, "server"), text(&b, "username"), text(&b, "password"))
                .await,
        ),
        Err(r) => *r,
    }
}

async fn add_ics(State(app): State<AppState>, b: Bytes) -> Response {
    let b = body(&b);
    match manager(&app) {
        Ok(m) => done(m.add_ics(text(&b, "url"), text(&b, "name"), &people(&b)).await),
        Err(r) => *r,
    }
}

async fn update(State(app): State<AppState>, Path((account, calendar)): Path<(String, String)>, b: Bytes) -> Response {
    let b = body(&b);
    let patch = Patch {
        people: b.get("people").map(|_| people(&b)),
        show: b["show"].as_bool(),
        add_events: b["addEvents"].as_bool(),
    };
    match manager(&app) {
        Ok(m) => done(m.update(&account, &calendar, patch).await),
        Err(r) => *r,
    }
}

async fn refresh(State(app): State<AppState>, Path(account): Path<String>) -> Response {
    match manager(&app) {
        Ok(m) => done(m.refresh(&account).await),
        Err(r) => *r,
    }
}

async fn remove(State(app): State<AppState>, Path(account): Path<String>) -> Response {
    match manager(&app) {
        Ok(m) => done(m.remove(&account).await.map(|()| json!({ "ok": true }))),
        Err(r) => *r,
    }
}
