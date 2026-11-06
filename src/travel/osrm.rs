//! The self-hosted [`TravelTime`] adapter: a geocoder (Photon or Nominatim) turns the event's
//! free-text location into coordinates, and OSRM's `route` service gives the driving time.
//!
//! No live traffic: OSRM's times come from road speeds in the OpenStreetMap extract. Everything
//! runs in the cluster, so no event location leaves the house.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Url;
use serde_json::Value;

use super::{Place, TravelError, TravelTime};
use crate::calendar::fetch::Fetch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Geocoder {
    /// A Photon server's base URL (`GET /api?q=…`).
    Photon(String),
    /// A Nominatim server's base URL (`GET /search?q=…&format=jsonv2`).
    Nominatim(String),
}

pub struct Osrm {
    fetch: Arc<dyn Fetch>,
    base: String,
    profile: String,
    geocoder: Geocoder,
    /// Home's coordinates: geocoding prefers results near here.
    near: Option<(f64, f64)>,
}

fn base(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn err(what: &str, detail: impl std::fmt::Display) -> TravelError {
    TravelError(format!("{what}: {detail}"))
}

impl Osrm {
    pub fn new(
        fetch: Arc<dyn Fetch>,
        osrm: String,
        profile: String,
        geocoder: Geocoder,
        near: Option<(f64, f64)>,
    ) -> Self {
        let geocoder = match geocoder {
            Geocoder::Photon(u) => Geocoder::Photon(base(&u)),
            Geocoder::Nominatim(u) => Geocoder::Nominatim(base(&u)),
        };
        Osrm {
            fetch,
            base: base(&osrm),
            profile,
            geocoder,
            near,
        }
    }

    async fn get_json(&self, what: &str, url: Url) -> Result<(u16, Value), TravelError> {
        let res = self.fetch.get(url.as_str()).await.map_err(|e| err(what, e))?;
        let json = serde_json::from_str(&res.body).map_err(|_| err(what, format!("HTTP {}, not JSON", res.status)))?;
        Ok((res.status, json))
    }

    /// Coordinates for `place`, `None` when the geocoder doesn't know it.
    async fn locate(&self, place: &Place) -> Result<Option<(f64, f64)>, TravelError> {
        match place {
            Place::Coords { lat, lon } => Ok(Some((*lat, *lon))),
            Place::Address(a) => self.geocode(a).await,
        }
    }

    async fn geocode(&self, text: &str) -> Result<Option<(f64, f64)>, TravelError> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        match &self.geocoder {
            Geocoder::Photon(b) => {
                let mut q = vec![("q", text.to_string()), ("limit", "1".into())];
                if let Some((lat, lon)) = self.near {
                    q.push(("lat", lat.to_string()));
                    q.push(("lon", lon.to_string()));
                }
                let url = Url::parse_with_params(&format!("{b}/api"), &q).map_err(|e| err("photon", e))?;
                let (status, json) = self.get_json("photon", url).await?;
                if !(200..300).contains(&status) {
                    return Err(err("photon", format!("HTTP {status}")));
                }
                let c = &json["features"][0]["geometry"]["coordinates"];
                Ok(c[1].as_f64().zip(c[0].as_f64()))
            }
            Geocoder::Nominatim(b) => {
                let mut q = vec![
                    ("q", text.to_string()),
                    ("format", "jsonv2".into()),
                    ("limit", "1".into()),
                ];
                if let Some((lat, lon)) = self.near {
                    // A preference, not a limit (no `bounded=1`): about 50 km around home.
                    q.push((
                        "viewbox",
                        format!("{},{},{},{}", lon - 0.5, lat + 0.5, lon + 0.5, lat - 0.5),
                    ));
                }
                let url = Url::parse_with_params(&format!("{b}/search"), &q).map_err(|e| err("nominatim", e))?;
                let (status, json) = self.get_json("nominatim", url).await?;
                if !(200..300).contains(&status) {
                    return Err(err("nominatim", format!("HTTP {status}")));
                }
                let hit = &json[0];
                let num = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64());
                Ok(num(&hit["lat"]).zip(num(&hit["lon"])))
            }
        }
    }

    async fn route(&self, (alat, alon): (f64, f64), (blat, blon): (f64, f64)) -> Result<Option<u32>, TravelError> {
        let url = format!(
            "{}/route/v1/{}/{alon:.6},{alat:.6};{blon:.6},{blat:.6}?overview=false&alternatives=false&steps=false",
            self.base, self.profile
        );
        let url = Url::parse(&url).map_err(|e| err("osrm", e))?;
        let (status, json) = self.get_json("osrm", url).await?;
        match json["code"].as_str() {
            Some("Ok") => {
                let secs = json["routes"][0]["duration"]
                    .as_f64()
                    .ok_or_else(|| err("osrm", "no duration in the answer"))?;
                Ok(Some((secs / 60.0).ceil().max(1.0) as u32))
            }
            // Somewhere off the road network, or no road between them: unknown, not a failure.
            Some("NoRoute" | "NoSegment") => Ok(None),
            Some(code) => Err(err("osrm", format!("HTTP {status}, {code}"))),
            None => Err(err("osrm", format!("HTTP {status}"))),
        }
    }
}

#[async_trait]
impl TravelTime for Osrm {
    fn configured(&self) -> bool {
        true
    }

    async fn minutes(&self, from: &Place, to: &str) -> Result<Option<u32>, TravelError> {
        let Some(a) = self.locate(from).await? else {
            return Ok(None);
        };
        let Some(b) = self.geocode(to).await? else {
            return Ok(None);
        };
        self.route(a, b).await
    }
}
