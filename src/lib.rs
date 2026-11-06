//! Home HQ: a self-hosted backend for the Home HQ family dashboard frontend.

pub mod api;
pub mod assets;
pub mod auth;
pub mod demo;
pub mod device_auth;
pub mod grant_store;
pub mod immich;
pub mod settings;
pub mod tls;

use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone)]
pub struct AppState {
    /// The dashboard key. `None` leaves the dashboard open (development and demo only).
    pub key: Option<Arc<str>>,
    pub device_auth: Option<Arc<device_auth::DeviceAuth>>,
    pub immich: Option<Arc<immich::Immich>>,
    pub demo: Arc<Mutex<demo::Demo>>,
    pub clock: Clock,
}

impl AppState {
    pub fn new(settings: settings::Settings, key: Option<String>, clock: Clock) -> Self {
        let demo = demo::Demo::new(settings, clock());
        AppState {
            immich: None,
            device_auth: None,
            key: key.filter(|k| !k.is_empty()).map(Arc::from),
            demo: Arc::new(Mutex::new(demo)),
            clock,
        }
    }

    pub async fn photo_source(&self) -> Option<Arc<immich::Immich>> {
        if let Some(auth) = &self.device_auth {
            auth.source().await
        } else {
            self.immich.clone()
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
