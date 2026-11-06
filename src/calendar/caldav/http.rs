//! The real [`Transport`]: reqwest over rustls, like the ICS fetch. Redirects are not followed,
//! so credentials are never sent on to another host and a moved calendar shows as an error.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Method, header};

use super::dav::{HttpRequest, HttpResponse, Transport};
use crate::calendar::fetch::MAX_BODY;

pub struct Http {
    client: reqwest::Client,
}

impl Http {
    /// Needs a rustls crypto provider installed first (`tls::install_crypto_provider`).
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("HomeHQ/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
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
impl Transport for Http {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let method = Method::from_bytes(req.method.as_bytes()).map_err(|_| "bad method".to_string())?;
        let mut b = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            b = b.header(*k, v);
        }
        if let Some(body) = req.body {
            b = b.body(body);
        }
        let res = b.send().await.map_err(describe)?;
        let status = res.status().as_u16();
        let etag = res
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err("response is larger than 20 MB".into());
        }
        let body = res.text().await.map_err(describe)?;
        if body.len() > MAX_BODY {
            return Err("response is larger than 20 MB".into());
        }
        Ok(HttpResponse { status, etag, body })
    }
}
