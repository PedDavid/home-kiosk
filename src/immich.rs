//! Immich favourites, translated to the dashboard's photo contract.
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use reqwest::{Client, Url, header};
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub struct Immich {
    client: Client,
    base: Url,
    cache: Mutex<Option<(Instant, Vec<Value>)>>,
}

impl Immich {
    /// The URL includes `/api`. Redirects are disabled so API keys cannot leak to another host.
    pub fn new(url: &str, key: &str) -> anyhow::Result<Self> {
        let base = Url::parse(&format!("{}/", url.trim_end_matches('/')))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || key.is_empty()
        {
            bail!("Immich requires an HTTP(S) API URL without credentials, query or fragment, and an API key");
        }
        let mut headers = header::HeaderMap::new();
        let mut key = header::HeaderValue::from_str(key).context("invalid Immich API key")?;
        key.set_sensitive(true);
        headers.insert("x-api-key", key);
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            client,
            base,
            cache: Mutex::new(None),
        })
    }

    pub fn from_env() -> anyhow::Result<Option<Self>> {
        match (
            std::env::var("HQ_IMMICH_URL").ok(),
            std::env::var("HQ_IMMICH_API_KEY").ok(),
        ) {
            (None, None) => Ok(None),
            (Some(url), Some(key)) => Ok(Some(Self::new(&url, &key)?)),
            _ => bail!("set both HQ_IMMICH_URL and HQ_IMMICH_API_KEY"),
        }
    }

    pub async fn photos(&self) -> anyhow::Result<Vec<Value>> {
        let mut cache = self.cache.lock().await;
        if let Some((at, photos)) = cache.as_ref()
            && at.elapsed() < Duration::from_secs(60)
        {
            return Ok(photos.clone());
        }
        let mut photos = Vec::new();
        for page in 1..=100 {
            let result: Value = self
                .client
                .post(self.base.join("search/metadata")?)
                .json(&json!({"isFavorite":true,"type":"IMAGE","withExif":true,"page":page,"size":100}))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let assets = &result["assets"];
            let items = assets["items"].as_array().context("invalid Immich search response")?;
            for asset in items {
                if asset["type"] != "IMAGE" || asset["isFavorite"] != true {
                    continue;
                }
                let id = asset["id"].as_str().context("Immich asset has no id")?;
                if !valid_id(id) {
                    bail!("invalid Immich asset id");
                }
                let exif = &asset["exifInfo"];
                let date = asset["localDateTime"]
                    .as_str()
                    .or_else(|| asset["fileCreatedAt"].as_str())
                    .unwrap_or("");
                let date = date.get(..10).unwrap_or("");
                photos.push(json!({"id":id,"o":if exif["exifImageHeight"].as_u64().unwrap_or(0) > exif["exifImageWidth"].as_u64().unwrap_or(0) {"p"} else {"l"},
                    "date":date,"place":exif["city"].as_str().unwrap_or(""),"from":null,"fav":true,"tag":date}));
            }
            if assets["nextPage"].is_null() {
                *cache = Some((Instant::now(), photos.clone()));
                return Ok(photos);
            }
        }
        bail!("Immich favourites exceed the 10000-photo limit")
    }

    async fn contains(&self, id: &str) -> anyhow::Result<bool> {
        Ok(valid_id(id) && self.photos().await?.iter().any(|p| p["id"] == id))
    }

    pub async fn image(&self, id: &str, small: bool) -> anyhow::Result<Option<(String, Vec<u8>)>> {
        if !self.contains(id).await? {
            return Ok(None);
        }
        let response = self
            .client
            .get(self.base.join(&format!("assets/{id}/thumbnail"))?)
            .query(&[("size", if small { "thumbnail" } else { "preview" })])
            .send()
            .await?
            .error_for_status()?;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !matches!(
            content_type.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/avif"
        ) {
            bail!("unexpected Immich image content type");
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 20 * 1024 * 1024 {
                bail!("Immich image exceeds 20 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some((content_type, bytes)))
    }

    pub async fn favourite(&self, id: &str, fav: bool) -> anyhow::Result<bool> {
        if !self.contains(id).await? {
            return Ok(false);
        }
        self.client
            .put(self.base.join(&format!("assets/{id}"))?)
            .json(&json!({"isFavorite":fav}))
            .send()
            .await?
            .error_for_status()?;
        *self.cache.lock().await = None;
        Ok(true)
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
