//! Weather from Open-Meteo (no key), mapped to upstream's `/api/weather` shape.
//!
//! The forecast for `settings.location` is cached in memory for [`TTL`]. A failed refresh serves
//! the last good value; with none, the response is upstream's `{ "error": ... }`. Fetching and the
//! clock are injected, so tests run on recorded responses.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{Clock, settings::Settings};

/// Open-Meteo's forecast endpoint.
pub const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";

/// How long a forecast is served before it is fetched again (upstream: 15 minutes).
pub const TTL: Duration = Duration::from_secs(15 * 60);

/// GETs a URL and returns the body, or an error for a transport failure or a non-2xx status.
pub type Fetch = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>> + Send + Sync>;

/// A [`Fetch`] over a real HTTP client, with a timeout so a hung upstream can't hold the cache.
pub fn http_fetch() -> anyhow::Result<Fetch> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("home-hq/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building the HTTP client")?;
    Ok(Arc::new(move |url: String| {
        let client = client.clone();
        Box::pin(async move {
            let res = client.get(&url).send().await.context("weather")?;
            let status = res.status();
            if !status.is_success() {
                bail!("weather: HTTP {}", status.as_u16());
            }
            res.text().await.context("weather")
        })
    }))
}

struct Cached {
    value: Value,
    expires: DateTime<Utc>,
}

pub struct Weather {
    /// `None` when settings have no location: the route answers `{ "none": true }`.
    url: Option<String>,
    fahrenheit: bool,
    fetch: Fetch,
    clock: Clock,
    cache: Mutex<Option<Cached>>,
}

impl Weather {
    /// `base_url` is Open-Meteo's forecast endpoint ([`OPEN_METEO_URL`], or a self-hosted one).
    pub fn new(settings: &Settings, base_url: &str, fetch: Fetch, clock: Clock) -> anyhow::Result<Self> {
        let fahrenheit = settings.location.as_ref().and_then(|l| l.units.as_deref()) == Some("fahrenheit");
        let url = match &settings.location {
            None => None,
            Some(loc) => Some(
                reqwest::Url::parse_with_params(
                    base_url,
                    [
                        ("latitude", loc.lat.to_string()),
                        ("longitude", loc.lon.to_string()),
                        ("timezone", settings.timezone.clone()),
                        ("forecast_days", "2".into()),
                        (
                            "temperature_unit",
                            if fahrenheit { "fahrenheit" } else { "celsius" }.into(),
                        ),
                        ("wind_speed_unit", if fahrenheit { "mph" } else { "kmh" }.into()),
                        (
                            "hourly",
                            "temperature_2m,precipitation_probability,weather_code,is_day".into(),
                        ),
                        ("daily", "temperature_2m_max,temperature_2m_min".into()),
                        (
                            "current",
                            "temperature_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m".into(),
                        ),
                    ],
                )
                .with_context(|| format!("Open-Meteo URL is not valid: {base_url}"))?
                .into(),
            ),
        };
        Ok(Weather {
            url,
            fahrenheit,
            fetch,
            clock,
            cache: Mutex::new(None),
        })
    }

    /// The forecast request, or `None` with no location configured.
    pub fn request_url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// The `/api/weather` body. Met Office warnings are dropped (UK-only), so `warnings` is `[]`.
    pub async fn get(&self) -> Value {
        let mut w = self.forecast().await;
        w["warnings"] = json!([]);
        w
    }

    async fn forecast(&self) -> Value {
        let Some(url) = &self.url else {
            return json!({ "none": true });
        };
        // Held across the fetch, so concurrent requests on an expired cache fetch once.
        let mut cache = self.cache.lock().await;
        let now = (self.clock)();
        if let Some(c) = cache.as_ref().filter(|c| c.expires > now) {
            return c.value.clone();
        }
        let fetched = (self.fetch)(url.clone()).await.and_then(|body| {
            let raw: Value = serde_json::from_str(&body).context("weather: response is not JSON")?;
            map(&raw, self.fahrenheit)
        });
        match fetched {
            Ok(value) => {
                *cache = Some(Cached {
                    value: value.clone(),
                    expires: now + TTL,
                });
                value
            }
            Err(err) => {
                tracing::warn!(
                    error = format!("{err:#}"),
                    stale = cache.is_some(),
                    "weather refresh failed"
                );
                match cache.as_ref() {
                    Some(c) => c.value.clone(), // serve stale rather than nothing
                    None => json!({ "error": format!("{err:#}") }),
                }
            }
        }
    }
}

/// Upstream's `wxCond`: a WMO weather code to the frontend's icon names.
pub fn cond(code: i64, is_day: bool) -> &'static str {
    match code {
        ..=1 if is_day => "sun",
        ..=1 => "moon",
        2 => "partly",
        3 | 45 | 48 => "cloud",
        71..=77 | 85 | 86 => "snow",
        _ => "rain",
    }
}

/// JavaScript's `Math.round` (halves round up, so -2.5 is -2), with `null` as 0 like upstream.
fn js_round(v: &Value) -> i64 {
    (v.as_f64().unwrap_or(0.0) + 0.5).floor() as i64
}

/// An Open-Meteo forecast response to upstream's weather shape.
pub fn map(w: &Value, fahrenheit: bool) -> anyhow::Result<Value> {
    let h = &w["hourly"];
    let arr = |v: &Value, name: &str| -> anyhow::Result<Vec<Value>> {
        v.as_array()
            .cloned()
            .with_context(|| format!("weather: response has no {name}"))
    };
    let day = h["time"][0]
        .as_str()
        .and_then(|t| t.get(..10))
        .context("weather: response has no hourly times")?; // index 0 = local midnight today
    let is_day = arr(&h["is_day"], "hourly.is_day")?;
    let c: Vec<&str> = arr(&h["weather_code"], "hourly.weather_code")?
        .iter()
        .enumerate()
        .map(|(i, code)| {
            let code = code.as_f64().map_or(0, |c| c as i64);
            cond(code, is_day.get(i).and_then(Value::as_f64).unwrap_or(0.0) != 0.0)
        })
        .collect();
    let t: Vec<i64> = arr(&h["temperature_2m"], "hourly.temperature_2m")?
        .iter()
        .map(js_round)
        .collect();
    // Upstream's `x || 0`: a missing probability is 0, a present one passes through.
    let p: Vec<Value> = arr(&h["precipitation_probability"], "hourly.precipitation_probability")?
        .into_iter()
        .map(|x| if x.as_f64().unwrap_or(0.0) == 0.0 { json!(0) } else { x })
        .collect();
    let (daily, current) = (&w["daily"], &w["current"]);
    if !daily.is_object() || !current.is_object() {
        bail!("weather: response has no daily or current values");
    }
    Ok(json!({
        "day": day,
        "units": if fahrenheit { "fahrenheit" } else { "celsius" },
        "t": t, "p": p, "c": c,
        "hi": js_round(&daily["temperature_2m_max"][0]),
        "lo": js_round(&daily["temperature_2m_min"][0]),
        "feels": js_round(&current["apparent_temperature"]),
        "humidity": current["relative_humidity_2m"],
        "wind": js_round(&current["wind_speed_10m"]),
        "now": js_round(&current["temperature_2m"]),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_codes_match_upstream() {
        let cases = [
            (0, true, "sun"),
            (1, true, "sun"),
            (0, false, "moon"),
            (1, false, "moon"),
            (2, true, "partly"),
            (2, false, "partly"),
            (3, true, "cloud"),
            (45, false, "cloud"),
            (48, true, "cloud"),
            (51, true, "rain"),
            (61, true, "rain"),
            (80, true, "rain"),
            (95, true, "rain"),
            (99, true, "rain"),
            (71, true, "snow"),
            (75, true, "snow"),
            (77, true, "snow"),
            (85, true, "snow"),
            (86, false, "snow"),
            (70, true, "rain"),
            (78, true, "rain"),
        ];
        for (code, day, want) in cases {
            assert_eq!(cond(code, day), want, "code {code} day {day}");
        }
    }

    #[test]
    fn rounds_like_javascript() {
        assert_eq!(js_round(&json!(2.5)), 3);
        assert_eq!(js_round(&json!(-2.5)), -2);
        assert_eq!(js_round(&json!(-2.6)), -3);
        assert_eq!(js_round(&Value::Null), 0);
    }
}
