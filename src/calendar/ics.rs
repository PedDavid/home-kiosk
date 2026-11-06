//! The read-only ICS adapter: private iCal links (Google's "Secret address in iCal format",
//! iCloud's public calendar, Nextcloud's export link), one per person or shared.
//!
//! Configured by `HQ_CALENDARS`, a JSON list kept in the Secret because the links are secrets:
//!
//! ```json
//! [ { "url": "https://calendar.google.com/calendar/ical/…/basic.ics", "people": ["alex"] },
//!   { "url": "webcal://p01-caldav.icloud.com/published/2/…", "people": "all" } ]
//! ```
//!
//! Terms: a **feed** is one entry, a calendar link; the **.ics file** is what the link returns,
//! the iCalendar text that [`engine::parse_feed`] reads.
//!
//! `people` is a list of person ids from the settings, one id, or `"all"`. Feeds are read
//! together and cached for [`TTL`]. A feed that fails keeps the events it had and reports
//! "Calendar N: …" (never its URL) in [`Listing::errors`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use tokio::sync::Mutex;

use super::fetch::Fetch;
use super::{CalendarSource, Event, Listing, Range, engine};
use crate::Clock;

/// How long feeds are kept before they're read again, as upstream.
pub const TTL: Duration = Duration::minutes(10);

/// One entry of `HQ_CALENDARS`.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    pub url: String,
    #[serde(default)]
    pub people: People,
}

/// `"all"`, one id, or a list of ids.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum People {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl People {
    fn ids(&self) -> Vec<String> {
        match self {
            People::None => Vec::new(),
            People::One(s) => vec![s.clone()],
            People::Many(v) => v.clone(),
        }
    }
}

/// Reads `HQ_CALENDARS`. The message never echoes the value (it holds the links).
pub fn parse_config(json: &str) -> Result<Vec<FeedConfig>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| "HQ_CALENDARS is not valid JSON".to_string())?;
    if !v.is_array() {
        return Err("HQ_CALENDARS must be a list [ … ]".into());
    }
    serde_json::from_value(v)
        .map_err(|_| r#"HQ_CALENDARS entries must look like { "url": "…", "people": ["id"] | "all" }"#.to_string())
}

struct Feed {
    url: String,
    people: Vec<String>,
    /// Config problems found up front (unknown people), reported with every refresh.
    notes: Vec<String>,
}

#[derive(Default)]
struct Cache {
    /// Last good events per feed, as read for `window`.
    feeds: Vec<Option<Vec<Event>>>,
    merged: Vec<Event>,
    errors: Vec<String>,
    checked: Option<DateTime<Utc>>,
    window: Option<Range>,
}

/// The ICS [`CalendarSource`].
pub struct IcsSource {
    feeds: Vec<Feed>,
    /// A broken `HQ_CALENDARS` (configured, but nothing can be read).
    config_error: Option<String>,
    fetch: Arc<dyn Fetch>,
    clock: Clock,
    tz: Tz,
    ttl: Duration,
    cache: Mutex<Cache>,
}

impl IcsSource {
    /// `known_people` are the settings' person ids: `"all"` expands to them, and feeds naming
    /// anyone else get a note on the Status panel.
    pub fn new(feeds: Vec<FeedConfig>, known_people: &[String], tz: Tz, fetch: Arc<dyn Fetch>, clock: Clock) -> Self {
        let feeds = feeds
            .into_iter()
            .enumerate()
            .map(|(i, f)| {
                let wanted = f.people.ids();
                let mut notes = Vec::new();
                let people: Vec<String> = if wanted.iter().any(|p| p == "all") || wanted.is_empty() {
                    known_people.to_vec()
                } else {
                    let unknown: Vec<&str> = wanted
                        .iter()
                        .filter(|p| !known_people.contains(p))
                        .map(String::as_str)
                        .collect();
                    if !unknown.is_empty() {
                        notes.push(format!(
                            "Calendar {}: unknown people {} (ids must match settings.json)",
                            i + 1,
                            unknown.join(", ")
                        ));
                    }
                    let known: Vec<String> = wanted.into_iter().filter(|p| known_people.contains(p)).collect();
                    if known.is_empty() { known_people.to_vec() } else { known }
                };
                Feed {
                    url: f.url.trim().replacen("webcal://", "https://", 1),
                    people,
                    notes,
                }
            })
            .collect();
        IcsSource {
            feeds,
            config_error: None,
            fetch,
            clock,
            tz,
            ttl: TTL,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// From the raw `HQ_CALENDARS` value: `None` when it's unset or empty (not configured). A
    /// value that doesn't parse still counts as configured, and reports why on the Status panel.
    pub fn from_env_value(
        value: Option<&str>,
        known_people: &[String],
        tz: Tz,
        fetch: Arc<dyn Fetch>,
        clock: Clock,
    ) -> Option<Self> {
        let value = value.map(str::trim).filter(|v| !v.is_empty())?;
        Some(match parse_config(value) {
            Ok(feeds) => Self::new(feeds, known_people, tz, fetch, clock),
            Err(e) => {
                tracing::error!(error = %e, "calendars");
                let mut s = Self::new(Vec::new(), known_people, tz, fetch, clock);
                s.config_error = Some(e);
                s
            }
        })
    }

    /// How many feeds are configured.
    pub fn len(&self) -> usize {
        self.feeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.feeds.is_empty()
    }

    /// Changes the cache lifetime (tests).
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    async fn read_feed(&self, i: usize, window: Range) -> Result<Vec<Event>, String> {
        let feed = &self.feeds[i];
        let res = self.fetch.get(&feed.url).await?;
        if !(200..300).contains(&res.status) {
            let hint = if res.status == 404 {
                " (link wrong, or reset in Google)"
            } else {
                ""
            };
            return Err(format!("HTTP {}{hint}", res.status));
        }
        let head: String = res.body.chars().take(2000).collect();
        if !head.to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
            return Err(r#"not an iCal feed (use "Secret address in iCal format")"#.into());
        }
        engine::parse_feed(&res.body, &feed.people, window, self.tz)
    }

    async fn refresh(&self, cache: &mut Cache, window: Range, now: DateTime<Utc>) {
        if let Some(e) = &self.config_error {
            cache.errors = vec![e.clone()];
            cache.checked = Some(now);
            cache.window = Some(window);
            return;
        }
        cache.feeds.resize(self.feeds.len(), None);
        let results = futures_util::future::join_all((0..self.feeds.len()).map(|i| self.read_feed(i, window))).await;
        let mut errors = Vec::new();
        for (i, result) in results.into_iter().enumerate() {
            errors.extend(self.feeds[i].notes.iter().cloned());
            match result {
                Ok(events) => {
                    tracing::info!(calendar = i + 1, events = events.len(), "calendar read");
                    cache.feeds[i] = Some(events);
                }
                Err(e) => {
                    tracing::warn!(calendar = i + 1, error = %e, "calendar failed; keeping its last events");
                    errors.push(format!("Calendar {}: {e}", i + 1));
                }
            }
        }
        cache.merged = engine::merge(cache.feeds.iter().flatten().cloned(), self.tz);
        cache.errors = errors;
        cache.checked = Some(now);
        cache.window = Some(window);
    }
}

#[async_trait]
impl CalendarSource for IcsSource {
    fn configured(&self) -> bool {
        true
    }

    async fn list(&self, range: Range) -> Listing {
        let now = (self.clock)();
        // Held across the refresh, so concurrent requests wait for one read instead of each
        // fetching the feeds.
        let mut cache = self.cache.lock().await;
        let fresh = cache.checked.is_some_and(|t| now - t < self.ttl && now >= t);
        let covers = cache.window.is_some_and(|w| w.contains(&range));
        if !fresh || !covers {
            let window = Range::around(now).union(&range);
            self.refresh(&mut cache, window, now).await;
        }
        Listing {
            events: cache
                .merged
                .iter()
                .filter(|e| e.when.overlaps(&range, self.tz))
                .cloned()
                .collect(),
            errors: cache.errors.clone(),
            checked: cache.checked.unwrap_or(now),
        }
    }
}
