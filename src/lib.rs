//! Home HQ: a self-hosted backend for the Home HQ family dashboard frontend.

pub mod api;
pub mod assets;
pub mod auth;
pub mod demo;
pub mod settings;

use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone)]
pub struct AppState {
    /// The dashboard key. `None` leaves the dashboard open (development and demo only).
    pub key: Option<Arc<str>>,
    pub demo: Arc<Mutex<demo::Demo>>,
    pub clock: Clock,
}

impl AppState {
    pub fn new(settings: settings::Settings, key: Option<String>, clock: Clock) -> Self {
        let demo = demo::Demo::new(settings, clock());
        AppState {
            key: key.filter(|k| !k.is_empty()).map(Arc::from),
            demo: Arc::new(Mutex::new(demo)),
            clock,
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api::router())
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(state.clone(), auth::middleware))
        .with_state(state)
}
