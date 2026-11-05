//! The TLS listener serves the router, and picks up a renewed certificate without a restart.

use std::{net::SocketAddr, path::Path, sync::Arc};

use chrono::{TimeZone, Utc};
use home_hq::{
    AppState, router,
    settings::{EXAMPLE, Settings},
    tls::{self, TlsFiles},
};
use rcgen::{CertifiedKey, KeyPair};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

fn self_signed() -> CertifiedKey<KeyPair> {
    rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap()
}

fn write(dir: &Path, cert: &CertifiedKey<KeyPair>) -> TlsFiles {
    // Same layout as a mounted kubernetes.io/tls Secret.
    let files = TlsFiles {
        cert: dir.join("tls.crt"),
        key: dir.join("tls.key"),
    };
    std::fs::write(&files.cert, cert.cert.pem()).unwrap();
    std::fs::write(&files.key, cert.signing_key.serialize_pem()).unwrap();
    files
}

/// GETs /healthz trusting only `trusted`. Returns the status line and the certificate served.
async fn healthz(addr: SocketAddr, trusted: &CertifiedKey<KeyPair>) -> (String, Vec<u8>) {
    let mut roots = RootCertStore::empty();
    roots.add(trusted.cert.der().clone()).unwrap();
    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut stream = TlsConnector::from(Arc::new(client))
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap();
    let served = stream.get_ref().1.peer_certificates().unwrap()[0].to_vec();
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response).into_owned();
    (response.lines().next().unwrap_or_default().to_string(), served)
}

#[tokio::test]
async fn serves_over_tls_and_reloads_a_renewed_certificate() {
    tls::install_crypto_provider();
    let dir = std::env::temp_dir().join(format!("home-hq-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let first = self_signed();
    let files = write(&dir, &first);
    let (config, mut current) = tls::load(&files).await.unwrap();

    let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap();
    let app = router(AppState::new(
        Settings::parse(EXAMPLE).unwrap(),
        None,
        Arc::new(move || now),
    ));
    let handle = axum_server::Handle::new();
    tokio::spawn(
        axum_server::bind_rustls("127.0.0.1:0".parse::<SocketAddr>().unwrap(), config.clone())
            .handle(handle.clone())
            .serve(app.into_make_service()),
    );
    let addr = handle.listening().await.unwrap();

    let (status, served) = healthz(addr, &first).await;
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(served, first.cert.der().to_vec());

    // Unchanged files: nothing to do.
    assert!(!tls::reload_if_changed(&config, &files, &mut current).await);

    // Renewal: new files on disk, picked up by the running listener.
    let renewed = self_signed();
    write(&dir, &renewed);
    assert!(tls::reload_if_changed(&config, &files, &mut current).await);
    let (status, served) = healthz(addr, &renewed).await;
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(served, renewed.cert.der().to_vec());

    // A broken write keeps the last good certificate.
    std::fs::write(&files.key, "not a key").unwrap();
    assert!(!tls::reload_if_changed(&config, &files, &mut current).await);
    let (_, served) = healthz(addr, &renewed).await;
    assert_eq!(served, renewed.cert.der().to_vec());

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
