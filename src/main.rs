use std::sync::Arc;

use anyhow::Context;
use home_hq::{AppState, router, settings::Settings};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()))
        .json()
        .init();

    let listen = std::env::var("HQ_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let settings_path = std::env::var("HQ_SETTINGS").ok();
    let settings = Settings::load(settings_path.as_deref())?;
    let key = std::env::var("HQ_KEY").ok().filter(|k| !k.is_empty());
    if key.is_none() {
        tracing::warn!("HQ_KEY is not set: the dashboard is open to anyone who can reach it");
    }
    tracing::info!(
        settings = settings_path.as_deref().unwrap_or("(bundled example)"),
        "loaded settings"
    );

    let state = AppState::new(settings, key, Arc::new(chrono::Utc::now));
    let app = router(state).layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    tracing::info!(%listen, "listening");
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
