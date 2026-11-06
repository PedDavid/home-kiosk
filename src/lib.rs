//! Home HQ: a self-hosted backend for the Home HQ family dashboard frontend.

pub mod api;
pub mod assets;
pub mod auth;
pub mod calendar;
pub mod demo;
pub mod device_auth;
pub mod grant_store;
pub mod immich;
pub mod settings;
pub mod tls;
pub mod travel;

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
    /// Calendar events. Not configured: `/api/calendar` serves the demo calendar.
    pub calendar: Arc<dyn calendar::CalendarSource>,
    /// Drive times added to calendar events. Not configured: events keep `Travel:` only.
    pub drives: Arc<travel::Drives>,
}

impl AppState {
    pub fn new(settings: settings::Settings, key: Option<String>, clock: Clock) -> Self {
        let demo = demo::Demo::new(settings, clock());
        let drives = Arc::new(travel::Drives::not_configured(chrono_tz::UTC, clock.clone()));
        AppState {
            immich: None,
            device_auth: None,
            key: key.filter(|k| !k.is_empty()).map(Arc::from),
            demo: Arc::new(Mutex::new(demo)),
            clock,
            calendar: Arc::new(calendar::NotConfigured),
            drives,
        }
    }

    /// Uses `source` for the calendar (when it reports itself configured).
    pub fn with_calendar(mut self, source: Arc<dyn calendar::CalendarSource>) -> Self {
        self.calendar = source;
        self
    }

    /// Uses `drives` for the calendar's `drive` times (when it reports itself configured).
    pub fn with_drives(mut self, drives: Arc<travel::Drives>) -> Self {
        self.drives = drives;
        self
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
