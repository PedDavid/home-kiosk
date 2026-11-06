//! The injected HTTP GET that adapters download .ics files with, so tests never touch the network.

use std::time::Duration;

use async_trait::async_trait;

/// A response: the status and the body as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

/// Fetches a URL. Errors must not contain the URL (calendar links are secrets).
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn get(&self, url: &str) -> Result<Response, String>;
}

/// .ics files larger than this are refused, so a wrong link can't fill memory.
pub const MAX_BODY: usize = 20 * 1024 * 1024;

/// The real fetch: the same reqwest as Immich (rustls with ring, the system's trust store).
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("HomeHQ/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Http { client })
    }
}

/// An error's message and causes, without the URL.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    msg
}

#[async_trait]
impl Fetch for Http {
    async fn get(&self, url: &str) -> Result<Response, String> {
        let res = self.client.get(url).send().await.map_err(describe)?;
        let status = res.status().as_u16();
        if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err("feed is larger than 20 MB".into());
        }
        let body = res.text().await.map_err(describe)?;
        if body.len() > MAX_BODY {
            return Err("feed is larger than 20 MB".into());
        }
        Ok(Response { status, body })
    }
}
