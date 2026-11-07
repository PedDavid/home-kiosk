//! Resolve the configured coordinates in the background; API reads never wait for the provider.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{settings::Location, weather::Fetch};

pub const NOMINATIM_URL: &str = "https://nominatim.openstreetmap.org/reverse";

pub struct Geocoding {
    url: String,
    fetch: Fetch,
    name: RwLock<Option<String>>,
}

impl Geocoding {
    pub fn new(location: &Location, endpoint: &str, fetch: Fetch) -> anyhow::Result<Self> {
        let url = reqwest::Url::parse_with_params(
            endpoint,
            [
                ("lat", location.lat.to_string()),
                ("lon", location.lon.to_string()),
                ("format", "jsonv2".into()),
                ("zoom", "10".into()),
                ("addressdetails", "1".into()),
                ("accept-language", "en".into()),
            ],
        )?;
        Ok(Self {
            url: url.into(),
            fetch,
            name: RwLock::new(None),
        })
    }

    pub async fn name(&self) -> Option<String> {
        self.name.read().await.clone()
    }

    /// Cache a successful lookup for this configuration's lifetime. Failures leave it intact.
    pub async fn resolve(&self) -> anyhow::Result<()> {
        if self.name().await.is_some() {
            return Ok(());
        }
        let body = (self.fetch)(self.url.clone()).await?;
        let value: Value = serde_json::from_str(&body)?;
        let address = &value["address"];
        let name = ["city", "town", "village", "municipality", "hamlet"]
            .into_iter()
            .filter_map(|key| address[key].as_str())
            .map(str::trim)
            .find(|name| !name.is_empty())
            .context("reverse geocoding returned no city or locality")?;
        *self.name.write().await = Some(name.to_owned());
        Ok(())
    }

    /// A single background worker retries failures every 15 minutes, then stops after success.
    pub fn start(self: &Arc<Self>) {
        let source = self.clone();
        tokio::spawn(async move {
            loop {
                if source.resolve().await.is_ok() {
                    break;
                }
                tracing::warn!("weather location lookup unavailable; showing coordinates and retrying in 15 minutes");
                tokio::time::sleep(Duration::from_secs(15 * 60)).await;
            }
        });
    }
}

/// Nominatim requires an identifying User-Agent. No browser or third-party QR request is used.
pub fn http_fetch() -> anyhow::Result<Fetch> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!(
            "home-hq/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/PedDavid/home-kiosk)"
        ))
        .build()?;
    Ok(Arc::new(move |url| {
        let client = client.clone();
        Box::pin(async move { Ok(client.get(url).send().await?.error_for_status()?.text().await?) })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn resolves_coordinates_and_caches_the_city() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let fetch: Fetch = Arc::new(move |url| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let url = reqwest::Url::parse(&url).unwrap();
                let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
                assert_eq!(params["lat"], "38.7223");
                assert_eq!(params["lon"], "-9.1393");
                Ok(r#"{"address":{"city":"Lisbon","country":"Portugal"}}"#.into())
            })
        });
        let settings = Settings::parse(crate::settings::EXAMPLE).unwrap();
        let source = Arc::new(Geocoding::new(settings.location.as_ref().unwrap(), NOMINATIM_URL, fetch).unwrap());
        assert_eq!(source.name().await, None);
        source.resolve().await.unwrap();
        source.resolve().await.unwrap();
        assert_eq!(source.name().await.as_deref(), Some("Lisbon"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let mut app = crate::AppState::new(settings, None, Arc::new(chrono::Utc::now));
        app.geocoding = Some(source);
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let res = crate::router(app)
            .oneshot(
                axum::http::Request::get("/api/state")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let state: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(state["weatherLocation"]["name"], "Lisbon");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "API reads use the cache");
    }

    #[tokio::test]
    async fn failure_can_retry_and_small_settlements_are_supported() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let fetch: Fetch = Arc::new(move |_| {
            let attempt = count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if attempt == 0 {
                    anyhow::bail!("offline");
                }
                Ok(r#"{"address":{"village":"Sintra"}}"#.into())
            })
        });
        let settings = Settings::parse(crate::settings::EXAMPLE).unwrap();
        let source = Geocoding::new(settings.location.as_ref().unwrap(), NOMINATIM_URL, fetch).unwrap();
        assert!(source.resolve().await.is_err());
        assert_eq!(source.name().await, None);
        source.resolve().await.unwrap();
        assert_eq!(source.name().await.as_deref(), Some("Sintra"));
    }

    #[tokio::test]
    async fn no_locality_does_not_invent_a_city() {
        let fetch: Fetch = Arc::new(|_| Box::pin(async { Ok(r#"{"address":{"country":"Portugal"}}"#.into()) }));
        let settings = Settings::parse(crate::settings::EXAMPLE).unwrap();
        let source = Geocoding::new(settings.location.as_ref().unwrap(), NOMINATIM_URL, fetch).unwrap();
        assert!(source.resolve().await.is_err());
        assert_eq!(source.name().await, None);
    }
}
