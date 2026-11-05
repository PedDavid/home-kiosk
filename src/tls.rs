//! Optional TLS listener, for serving the dashboard straight from its own LAN address (a
//! LoadBalancer IP) instead of through a shared reverse proxy, so a firewall rule can allow a
//! frame to reach this one service and nothing else.
//!
//! The certificate is reloaded in place when the files change, so a renewal never needs a
//! restart. Kubernetes updates a mounted Secret by swapping a symlink to a new directory, which
//! changes both files at once; the poll compares contents, since mtimes don't survive the swap.

use std::{io, path::PathBuf, time::Duration};

use anyhow::{Context, bail};
use axum_server::tls_rustls::RustlsConfig;

/// PEM certificate chain and private key, as last loaded.
pub type Pem = (Vec<u8>, Vec<u8>);

#[derive(Clone, Debug)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TlsFiles {
    /// `HQ_TLS_CERT` and `HQ_TLS_KEY`: both set enables TLS, neither leaves it off.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        match (var("HQ_TLS_CERT"), var("HQ_TLS_KEY")) {
            (Some(cert), Some(key)) => Ok(Some(TlsFiles { cert, key })),
            (None, None) => Ok(None),
            _ => bail!("set both HQ_TLS_CERT and HQ_TLS_KEY, or neither"),
        }
    }

    async fn read(&self) -> io::Result<Pem> {
        Ok((tokio::fs::read(&self.cert).await?, tokio::fs::read(&self.key).await?))
    }
}

/// Uses ring as rustls' crypto provider (no cmake or NASM needed to build). Call once, early.
pub fn install_crypto_provider() {
    // Err only means a provider is already installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub async fn load(files: &TlsFiles) -> anyhow::Result<(RustlsConfig, Pem)> {
    let pem = files
        .read()
        .await
        .with_context(|| format!("reading {} and {}", files.cert.display(), files.key.display()))?;
    let config = RustlsConfig::from_pem(pem.0.clone(), pem.1.clone())
        .await
        .context("loading the TLS certificate and key")?;
    Ok((config, pem))
}

/// Reloads `config` if the files differ from `current`. Returns whether it reloaded. A file that
/// can't be read or parsed is logged and the previous certificate stays in use.
pub async fn reload_if_changed(config: &RustlsConfig, files: &TlsFiles, current: &mut Pem) -> bool {
    let pem = match files.read().await {
        Ok(pem) if pem == *current => return false,
        Ok(pem) => pem,
        Err(error) => {
            tracing::warn!(%error, "reading the TLS files failed; keeping the current certificate");
            return false;
        }
    };
    match config.reload_from_pem(pem.0.clone(), pem.1.clone()).await {
        Ok(()) => {
            tracing::info!(cert = %files.cert.display(), "reloaded the TLS certificate");
            *current = pem;
            true
        }
        Err(error) => {
            tracing::warn!(%error, "the new TLS files don't load; keeping the current certificate");
            false
        }
    }
}

/// Polls the files every `every` and reloads on change. Runs until the task is dropped.
pub async fn watch(config: RustlsConfig, files: TlsFiles, mut current: Pem, every: Duration) {
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await; // the first tick is immediate
    loop {
        tick.tick().await;
        reload_if_changed(&config, &files, &mut current).await;
    }
}
