//! Travel time for the "leave by" countdown (issue #18).
//!
//! Three layers:
//!
//! - The [`TravelTime`] port: driving minutes from an origin [`Place`] to a free-text
//!   destination (an event's `LOCATION`), or "unknown". Geocoding is the adapter's business, so a
//!   hosted provider that takes addresses directly (Google Routes, Mapbox, TomTom, …) fits the
//!   same trait.
//! - [`Drives`], the shared wrapper every adapter sits behind. It owns upstream's rules: only
//!   events with a location starting in the next 4 hours, results (failures too) kept 10
//!   minutes, a daily request cap, and the longest drive winning when several people attend.
//! - [`osrm::Osrm`], the self-hosted adapter: OSRM for routing, Photon or Nominatim for
//!   geocoding.
//!
//! Origins are static, from the settings: `travel.home` (or the household `location`), and
//! optionally a person's own `origin`. Nothing here talks to Home Assistant.
//!
//! When nothing is configured [`Drives::configured`] is false, no event gets a `drive`, and the
//! frontend keeps using the event's `Travel:` line.

pub mod drives;
pub mod osrm;

use std::{collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    Clock,
    calendar::{self, fetch::Fetch},
    settings::Settings,
};

pub use drives::{Drives, Origins, Rules, Trip};

/// Where a drive starts: coordinates, or an address the adapter geocodes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Place {
    Coords { lat: f64, lon: f64 },
    Address(String),
}

impl Place {
    /// Identifies the place in caches: two spellings of the same coordinates share a key.
    pub fn key(&self) -> String {
        match self {
            Place::Coords { lat, lon } => format!("{lat:.5},{lon:.5}"),
            Place::Address(a) => format!("addr:{}", normalise(a)),
        }
    }
}

/// Lower-cased, with whitespace collapsed: the cache key for a free-text place.
pub fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A drive on the wire, in an `/api/calendar` event: `drive: { mins, from }`. `from` is `"home"`
/// or the name of the person whose own origin gave the longest drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Drive {
    pub mins: u32,
    pub from: String,
}

/// An adapter failure (network, HTTP status, unreadable answer). The wrapper caches it like a
/// result, so a broken provider isn't asked again on every poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TravelError(pub String);

impl fmt::Display for TravelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TravelError {}

/// The port: how long a drive takes.
///
/// - `Ok(Some(mins))`: the drive, in whole minutes rounded up.
/// - `Ok(None)`: unknown: a place that can't be found, or no route between them.
/// - `Err(_)`: the provider failed.
///
/// Adapters don't cache, budget or filter events; [`Drives`] does that for all of them.
#[async_trait]
pub trait TravelTime: Send + Sync {
    /// Whether a provider is set up. When not, no event gets a `drive`.
    fn configured(&self) -> bool;

    async fn minutes(&self, from: &Place, to: &str) -> Result<Option<u32>, TravelError>;
}

/// No provider: `configured()` is false and every drive is unknown.
pub struct NotConfigured;

#[async_trait]
impl TravelTime for NotConfigured {
    fn configured(&self) -> bool {
        false
    }

    async fn minutes(&self, _from: &Place, _to: &str) -> Result<Option<u32>, TravelError> {
        Ok(None)
    }
}

/// The `travel` section of the settings.
///
/// ```json
/// "travel": {
///   "osrm": "http://osrm.routing.svc:5000",
///   "photon": "http://photon.routing.svc:2322",
///   "home": { "lat": 38.7223, "lon": -9.1393 },
///   "dailyCap": 300
/// }
/// ```
///
/// `home` may also be an address; without it the household `location` is home. Use `nominatim`
/// instead of `photon` for a Nominatim server. A person can have their own `origin` (same forms
/// as `home`) in `people`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TravelConfig {
    /// Only `"osrm"` today; a hosted provider would add its own name here.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// The OSRM server's base URL.
    pub osrm: Option<String>,
    /// The OSRM profile the server was built with.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// A Photon server's base URL (geocoding).
    pub photon: Option<String>,
    /// A Nominatim server's base URL (geocoding), instead of Photon.
    pub nominatim: Option<String>,
    pub home: Option<Place>,
    /// Provider requests a day, counted per origin and destination asked.
    #[serde(default = "default_cap")]
    pub daily_cap: u32,
}

fn default_provider() -> String {
    "osrm".into()
}

fn default_profile() -> String {
    "driving".into()
}

fn default_cap() -> u32 {
    drives::DAILY_CAP
}

/// The wrapper the settings ask for, using `fetch` for HTTP. Not configured when there's no
/// `travel` section; an inconsistent section is an error, so a typo fails at start-up.
pub fn from_settings(settings: &Settings, fetch: Arc<dyn Fetch>, clock: Clock) -> anyhow::Result<Drives> {
    let tz = settings.tz()?;
    let Some(cfg) = &settings.travel else {
        return Ok(Drives::not_configured(tz, clock));
    };
    let home = cfg.home.clone().or_else(|| {
        settings
            .location
            .as_ref()
            .map(|l| Place::Coords { lat: l.lat, lon: l.lon })
    });
    let people: BTreeMap<String, (String, Place)> = settings
        .people
        .iter()
        .filter_map(|p| Some((p.id.clone(), (p.name.clone(), p.origin.clone()?))))
        .collect();
    if home.is_none() && people.is_empty() {
        anyhow::bail!("travel: no origin; set travel.home, location, or a person's origin");
    }
    let near = match &home {
        Some(Place::Coords { lat, lon }) => Some((*lat, *lon)),
        _ => settings.location.as_ref().map(|l| (l.lat, l.lon)),
    };
    let port: Arc<dyn TravelTime> = match cfg.provider.as_str() {
        "osrm" => {
            let base = cfg
                .osrm
                .clone()
                .ok_or_else(|| anyhow::anyhow!("travel: set travel.osrm, the OSRM server's URL"))?;
            let geocoder = match (&cfg.photon, &cfg.nominatim) {
                (Some(url), None) => osrm::Geocoder::Photon(url.clone()),
                (None, Some(url)) => osrm::Geocoder::Nominatim(url.clone()),
                (None, None) => anyhow::bail!("travel: set travel.photon or travel.nominatim, a geocoder's URL"),
                (Some(_), Some(_)) => anyhow::bail!("travel: set only one of travel.photon and travel.nominatim"),
            };
            Arc::new(osrm::Osrm::new(fetch, base, cfg.profile.clone(), geocoder, near))
        }
        other => anyhow::bail!("travel: unknown provider {other:?} (known: \"osrm\")"),
    };
    let rules = Rules {
        daily_cap: cfg.daily_cap,
        ..Rules::default()
    };
    Ok(Drives::new(port, Origins { home, people }, rules, tz, clock))
}

/// What the wrapper needs from a calendar event.
pub fn trip(e: &calendar::Event) -> Trip<'_> {
    let (start, all_day) = match e.when {
        calendar::When::Timed { start, .. } => (start, false),
        calendar::When::AllDay { start, .. } => (calendar::engine::midnight(start, chrono_tz::UTC), true),
    };
    Trip {
        id: &e.id,
        location: e.location.as_deref(),
        start,
        all_day,
        people: &e.people,
    }
}

/// Adds `drive` to the events in an `/api/calendar` body (upstream's shape) that have one.
pub fn attach(body: &mut serde_json::Value, drives: &std::collections::HashMap<String, Drive>) {
    if drives.is_empty() {
        return;
    }
    for e in body["events"].as_array_mut().into_iter().flatten() {
        if let Some(d) = e["id"].as_str().and_then(|id| drives.get(id)) {
            e["drive"] = serde_json::json!(d);
        }
    }
}
