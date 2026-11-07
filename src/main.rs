use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Context;
use home_hq::{AppState, calendar, geocoding, router, settings::Settings, share::ImmichPublicUrl, tls, weather};
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

    let immich_public_url = ImmichPublicUrl::parse(&std::env::var("HQ_IMMICH_PUBLIC_URL").unwrap_or_default())?;
    match &immich_public_url {
        Some(u) => tracing::info!(immich_public_url = u.as_str(), "photo sharing on"),
        None => tracing::info!("HQ_IMMICH_PUBLIC_URL is not set: the Share button is hidden"),
    }

    // reqwest (calendar, Immich, weather) and the TLS listener share rustls's ring provider.
    tls::install_crypto_provider();
    let clock: home_hq::Clock = Arc::new(chrono::Utc::now);
    let calendar = calendars(&settings, key.as_deref(), clock.clone())?;
    let weather = weather_from_env(&settings, clock.clone())?;
    let geocoding = if weather.is_some() {
        settings
            .location
            .as_ref()
            .map(|location| {
                let endpoint = std::env::var("HQ_GEOCODING_URL").unwrap_or_else(|_| geocoding::NOMINATIM_URL.into());
                geocoding::Geocoding::new(location, &endpoint, geocoding::http_fetch()?).map(Arc::new)
            })
            .transpose()?
    } else {
        None
    };
    let mut state = AppState::new(settings, key, clock)
        .with_calendar_accounts(calendar)
        .with_immich_public_url(immich_public_url);
    if let Some(w) = weather {
        state = state.with_weather(w);
    }
    if let Some(source) = geocoding {
        source.start();
        state.geocoding = Some(source);
    }
    state.device_auth = home_hq::device_auth::DeviceAuth::from_env()?.map(Arc::new);
    if state.device_auth.is_none() {
        state.immich = home_hq::immich::Immich::from_env()?.map(Arc::new);
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

/// Live weather from Open-Meteo by default (`HQ_OPEN_METEO_URL` overrides the endpoint);
/// `HQ_WEATHER=demo` serves the demo forecast instead and makes no outbound calls.
fn weather_from_env(settings: &Settings, clock: home_hq::Clock) -> anyhow::Result<Option<weather::Weather>> {
    match std::env::var("HQ_WEATHER").unwrap_or_default().as_str() {
        "demo" => Ok(None),
        "" | "open-meteo" => {
            let url = std::env::var("HQ_OPEN_METEO_URL")
                .ok()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| weather::OPEN_METEO_URL.into());
            let w = weather::Weather::new(settings, &url, weather::http_fetch()?, clock)?;
            match w.request_url() {
                Some(_) => tracing::info!(endpoint = %url, "weather from Open-Meteo"),
                None => tracing::warn!("settings have no location: weather is off"),
            }
            Ok(Some(w))
        }
        other => anyhow::bail!("HQ_WEATHER must be open-meteo or demo, not {other:?}"),
    }
}

/// Calendars from `HQ_CALENDARS` and from the calendar page (`/calendars`), kept encrypted in
/// `HQ_STATE_DIR` when `HQ_KEY` is set. With neither, the demo calendar.
fn calendars(
    settings: &Settings,
    key: Option<&str>,
    clock: home_hq::Clock,
) -> anyhow::Result<Arc<calendar::accounts::Manager>> {
    settings.tz()?;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let manager = calendar::accounts::Manager::new(calendar::accounts::Config {
        settings: settings.clone(),
        env: env("HQ_CALENDARS"),
        fetch: Arc::new(calendar::fetch::Http::new()?),
        transport: Arc::new(calendar::caldav::http::Http::new()?),
        clock,
        state_dir: env("HQ_STATE_DIR").map(Into::into),
        hq_key: key.map(String::from),
        allowed_servers: env("HQ_CALENDAR_SERVERS"),
    });
    let source = manager.source();
    if source.configured() {
        tracing::info!(writable = source.can_write(), "calendar: configured");
    } else {
        tracing::info!("calendar: nothing configured yet, serving the demo calendar (add calendars at /calendars)");
    }
    Ok(Arc::new(manager))
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
