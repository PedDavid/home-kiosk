//! Home HQ: a self-hosted backend for the Home HQ family dashboard frontend.

pub mod api;
pub mod assets;
pub mod auth;
pub mod demo;
pub mod settings;
pub mod shopping;
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
    pub demo: Arc<Mutex<demo::Demo>>,
    pub clock: Clock,
    /// The shopping list port. `None` keeps the demo list.
    pub groceries: Option<Arc<shopping::Groceries>>,
}

impl AppState {
    pub fn new(settings: settings::Settings, key: Option<String>, clock: Clock) -> Self {
        let demo = demo::Demo::new(settings, clock());
        AppState {
            key: key.filter(|k| !k.is_empty()).map(Arc::from),
            demo: Arc::new(Mutex::new(demo)),
            clock,
            groceries: None,
        }
    }

    /// Serve the shopping list from `port` instead of the demo list.
    pub fn with_shopping(mut self, port: Arc<dyn shopping::ShoppingList>, settings: &settings::Settings) -> Self {
        self.groceries = Some(Arc::new(shopping::Groceries::new(port, settings.clone())));
        self
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
