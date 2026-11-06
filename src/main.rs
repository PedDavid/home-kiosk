use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use home_hq::{AppState, router, settings::Settings, shopping, tls};
use tokio::sync::watch;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()))
        .json()
        .init();

    let listen = std::env::var("HQ_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let tls_listen = std::env::var("HQ_TLS_LISTEN").unwrap_or_else(|_| "0.0.0.0:8443".into());
    let tls_files = tls::TlsFiles::from_env()?;
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

    let shopping = shopping::from_env(&settings)?;
    let mut state = AppState::new(settings.clone(), key, Arc::new(chrono::Utc::now));
    if let Some(port) = shopping {
        state = state.with_shopping(port, &settings);
    }
    let app = router(state).layer(TraceLayer::new_for_http());

    // One signal stops every listener.
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown().await;
        let _ = stop_tx.send(true);
    });

    // Plain HTTP stays on: probes, and a reverse proxy in front if there is one.
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    tracing::info!(%listen, "listening");
    let plain = axum::serve(listener, app.clone()).with_graceful_shutdown(stopped(stop_rx.clone()));

    let Some(files) = tls_files else {
        plain.await?;
        return Ok(());
    };

    tls::install_crypto_provider();
    let (config, pem) = tls::load(&files).await?;
    let addr: SocketAddr = tls_listen
        .parse()
        .with_context(|| format!("HQ_TLS_LISTEN is not an address: {tls_listen}"))?;
    let reloader = tokio::spawn(tls::watch(config.clone(), files, pem, Duration::from_secs(60)));
    let handle = axum_server::Handle::new();
    tokio::spawn({
        let handle = handle.clone();
        let stop = stopped(stop_rx);
        async move {
            stop.await;
            handle.graceful_shutdown(Some(Duration::from_secs(10)));
        }
    });
    tracing::info!(listen = %addr, "listening (TLS)");
    let secure = axum_server::bind_rustls(addr, config)
        .handle(handle)
        .serve(app.into_make_service());

    let result = tokio::try_join!(async { plain.await.context("HTTP listener") }, async {
        secure.await.with_context(|| format!("TLS listener on {addr}"))
    },);
    reloader.abort();
    result?;
    Ok(())
}

async fn stopped(mut stop: watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopped| *stopped).await;
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
