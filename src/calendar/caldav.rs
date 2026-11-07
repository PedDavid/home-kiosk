//! The CalDAV adapter: read-and-write calendars (Radicale, Nextcloud, Baïkal, iCloud, Fastmail…).
//!
//! - **Read:** a `calendar-query` `REPORT` with a time range per calendar. Each object's
//!   iCalendar text goes through the same [`engine`] as ICS feeds, for recurrence, timezones,
//!   attribution and merging.
//! - **Add:** `PUT` of a new `VEVENT` (`If-None-Match: *`) to the calendar for the event's
//!   people ([`config::calendar_for`]), at `<calendar>/<frontend id>.ics`.
//! - **Remove:** `DELETE` with `If-Match` on the object's ETag. Only events added on the frame
//!   (their `UID` ends in [`vevent::UID_SUFFIX`]) are offered for removal.
//!
//! Configuration is in `HQ_CALENDARS` next to the ICS feeds (see [`config`]). Reads are cached
//! for [`TTL`] like ICS; writes update the cache straight away, and a failed write leaves it as it
//! was, so the frame keeps showing everything else while it says "Couldn't save".
//!
//! The WebDAV requests are written by hand ([`dav`]) over an injected [`dav::Transport`]
//! ([`http::Http`] in production), as the PRD expected.

pub mod config;
pub mod dav;
pub mod http;
pub mod login_flow;
pub mod vevent;

use std::sync::{Arc, Mutex as SyncMutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use tokio::sync::Mutex;

use self::config::{CalDavConfig, Target, calendar_for, ids};
use self::dav::{CalendarInfo, DavClient, DavError, Transport};
use super::{CalendarError, CalendarSource, Event, Listing, NewEvent, Range, engine};
use crate::{Clock, settings::WriteRule};

/// How long calendars are kept before they're read again, as for ICS feeds.
pub const TTL: Duration = Duration::minutes(10);

/// Longest title accepted from the frame, as upstream.
const MAX_TITLE: usize = 120;

struct Calendar {
    /// "Calendar N" on the Status panel: the entry's position in `HQ_CALENDARS`.
    number: usize,
    /// How the Status panel names it: the label, else "Calendar N".
    title: String,
    id: Option<String>,
    url: String,
    /// `people` as configured (`["all"]` kept), for choosing where to write.
    wanted: Vec<String>,
    /// The person ids its events are attributed to.
    people: Vec<String>,
    read_only: bool,
    client: DavClient,
    /// Config problems found up front (unknown people), reported with every refresh.
    notes: Vec<String>,
    /// What discovery found, once it has succeeded.
    info: SyncMutex<Option<CalendarInfo>>,
}

impl Calendar {
    fn info(&self) -> Option<CalendarInfo> {
        self.info.lock().unwrap().clone()
    }

    /// Writable as far as we know: not `readOnly`, and the server didn't say otherwise.
    fn writable(&self) -> bool {
        !self.read_only && self.info().is_none_or(|i| i.writable)
    }

    async fn discover(&self) -> Result<CalendarInfo, DavError> {
        if let Some(i) = self.info() {
            return Ok(i);
        }
        let found = self.client.discover(&self.url).await?;
        if !found.writable && !self.read_only {
            tracing::warn!(
                calendar = self.number,
                "the server doesn't let these credentials add events"
            );
        }
        *self.info.lock().unwrap() = Some(found.clone());
        Ok(found)
    }
}

/// One calendar object and the events it expanded to.
#[derive(Debug, Clone)]
struct Object {
    url: String,
    etag: Option<String>,
    events: Vec<Event>,
}

#[derive(Default)]
struct Cache {
    /// Last good objects per calendar, as read for `window`.
    calendars: Vec<Option<Vec<Object>>>,
    merged: Vec<Event>,
    errors: Vec<String>,
    checked: Option<DateTime<Utc>>,
    window: Option<Range>,
}

/// The CalDAV [`CalendarSource`].
pub struct CalDavSource {
    calendars: Vec<Calendar>,
    /// Entries of `HQ_CALENDARS` that couldn't be read, reported with every refresh.
    config_errors: Vec<String>,
    household: Vec<String>,
    rules: Vec<WriteRule>,
    clock: Clock,
    tz: Tz,
    ttl: Duration,
    cache: Mutex<Cache>,
}

impl CalDavSource {
    /// `household` is every person id in the settings; `rules` is the settings'
    /// `calendar.writeTo`.
    pub fn new(
        configs: Vec<CalDavConfig>,
        household: &[String],
        rules: Vec<WriteRule>,
        tz: Tz,
        transport: Arc<dyn Transport>,
        clock: Clock,
    ) -> Self {
        let calendars = configs
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let c = c.normalise();
                let number = if c.number == 0 { i + 1 } else { c.number };
                let wanted = ids(&c.people);
                let mut notes = Vec::new();
                let all = wanted.is_empty() || wanted.iter().any(|p| p == "all");
                let people: Vec<String> = if all {
                    household.to_vec()
                } else {
                    let unknown: Vec<&str> = wanted
                        .iter()
                        .filter(|p| !household.contains(p))
                        .map(String::as_str)
                        .collect();
                    if !unknown.is_empty() {
                        notes.push(format!(
                            "{}: unknown people {} (ids must match settings.json)",
                            c.label.clone().unwrap_or_else(|| format!("Calendar {number}")),
                            unknown.join(", ")
                        ));
                    }
                    let known: Vec<String> = wanted.iter().filter(|p| household.contains(p)).cloned().collect();
                    if known.is_empty() { household.to_vec() } else { known }
                };
                Calendar {
                    number,
                    title: c.label.clone().unwrap_or_else(|| format!("Calendar {number}")),
                    id: c.id.clone(),
                    url: c.url.clone(),
                    wanted: if all { vec!["all".into()] } else { wanted },
                    people,
                    read_only: c.read_only,
                    client: DavClient::new(transport.clone(), c.username.as_deref(), c.password.as_deref()),
                    notes,
                    info: SyncMutex::new(None),
                }
            })
            .collect();
        CalDavSource {
            calendars,
            config_errors: Vec::new(),
            household: household.to_vec(),
            rules,
            clock,
            tz,
            ttl: TTL,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// Problems with `HQ_CALENDARS` entries that were left out, for the Status panel.
    pub fn with_config_errors(mut self, errors: Vec<String>) -> Self {
        self.config_errors = errors;
        self
    }

    /// Changes the cache lifetime (tests).
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Expands one object with the engine; events added on the frame get their frontend id,
    /// `local`, and the people they were added for.
    fn expand(&self, cal: &Calendar, url: String, etag: Option<String>, ics: &str, window: Range) -> Object {
        let mut events = engine::parse_feed(ics, &cal.people, window, self.tz).unwrap_or_default();
        if let Some(id) = vevent::uid_of(ics).as_deref().and_then(vevent::frame_id) {
            let people = vevent::people_of(ics)
                .map(|p| p.into_iter().filter(|p| self.household.contains(p)).collect::<Vec<_>>())
                .filter(|p| !p.is_empty());
            for e in &mut events {
                e.id = id.to_string();
                e.local = true;
                // The frame says "Added on this display" itself; phones still see the description.
                if e.notes.as_deref() == Some(vevent::DESCRIPTION) {
                    e.notes = None;
                }
                if let Some(p) = &people {
                    e.people = p.clone();
                }
            }
        }
        Object { url, etag, events }
    }

    async fn read_calendar(&self, i: usize, window: Range) -> Result<Vec<Object>, DavError> {
        let cal = &self.calendars[i];
        let info = cal.discover().await?;
        let objects = cal.client.query(&info.url, window.from, window.to).await?;
        Ok(objects
            .into_iter()
            .map(|o| self.expand(cal, o.url, o.etag, &o.ics, window))
            .collect())
    }

    fn remerge(&self, cache: &mut Cache) {
        cache.merged = engine::merge(
            cache
                .calendars
                .iter()
                .flatten()
                .map(|objs| objs.iter().flat_map(|o| o.events.iter().cloned()).collect()),
            self.tz,
        );
    }

    async fn refresh(&self, cache: &mut Cache, window: Range, now: DateTime<Utc>) {
        cache.calendars.resize(self.calendars.len(), None);
        let results =
            futures_util::future::join_all((0..self.calendars.len()).map(|i| self.read_calendar(i, window))).await;
        let mut errors = self.config_errors.clone();
        for (i, result) in results.into_iter().enumerate() {
            let cal = &self.calendars[i];
            errors.extend(cal.notes.iter().cloned());
            match result {
                Ok(objects) => {
                    tracing::info!(calendar = cal.number, objects = objects.len(), "CalDAV calendar read");
                    cache.calendars[i] = Some(objects);
                }
                Err(e) => {
                    tracing::warn!(calendar = cal.number, error = ?e, "CalDAV calendar failed; keeping its last events");
                    errors.push(format!("{}: {e}", cal.title));
                }
            }
        }
        self.remerge(cache);
        cache.errors = errors;
        cache.checked = Some(now);
        cache.window = Some(window);
    }

    fn validate(&self, new: &NewEvent) -> Result<(), CalendarError> {
        let id_ok = !new.id.is_empty()
            && new.id.len() <= 64
            && new
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !id_ok {
            return Err(CalendarError::Invalid("id must be 1–64 letters, digits, - or _".into()));
        }
        if new.title.trim().is_empty() || new.title.chars().count() > MAX_TITLE {
            return Err(CalendarError::Invalid("title required (at most 120 characters)".into()));
        }
        if new.end <= new.start {
            return Err(CalendarError::Invalid("start < end required".into()));
        }
        if new.people.is_empty() || new.people.iter().any(|p| !self.household.contains(p)) {
            return Err(CalendarError::Invalid("people must be ids from the settings".into()));
        }
        Ok(())
    }

    fn backend(&self, cal: &Calendar, e: DavError) -> CalendarError {
        tracing::warn!(calendar = cal.number, error = ?e, "CalDAV write failed");
        CalendarError::Backend(e.to_string())
    }
}

fn object_url(calendar: &str, id: &str) -> String {
    format!(
        "{}{id}.ics",
        if calendar.ends_with('/') {
            calendar.to_string()
        } else {
            format!("{calendar}/")
        }
    )
}

#[async_trait]
impl CalendarSource for CalDavSource {
    fn configured(&self) -> bool {
        !self.calendars.is_empty() || !self.config_errors.is_empty()
    }

    /// True while some calendar may take new events: not `readOnly`, and the server hasn't said
    /// these credentials can't write.
    fn can_write(&self) -> bool {
        self.calendars.iter().any(Calendar::writable)
    }

    async fn list(&self, range: Range) -> Listing {
        let now = (self.clock)();
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

    async fn add(&self, new: NewEvent) -> Result<Event, CalendarError> {
        if !self.can_write() {
            return Err(CalendarError::ReadOnly);
        }
        self.validate(&new)?;
        // Writability is only known after discovery, so find out before choosing.
        for cal in self.calendars.iter().filter(|c| !c.read_only && c.info().is_none()) {
            if let Err(e) = cal.discover().await {
                tracing::warn!(calendar = cal.number, error = ?e, "CalDAV discovery failed");
            }
        }
        let targets: Vec<Target> = self
            .calendars
            .iter()
            .map(|c| Target {
                id: c.id.as_deref(),
                people: c.wanted.clone(),
                writable: c.writable() && c.info().is_some(),
            })
            .collect();
        let Some(i) = calendar_for(&new.people, &self.household, &targets, &self.rules) else {
            return Err(CalendarError::Invalid(
                "no writable calendar for these people (calendar.writeTo in the settings, or a CalDAV calendar for them)"
                    .into(),
            ));
        };
        let cal = &self.calendars[i];
        let info = cal.discover().await.map_err(|e| self.backend(cal, e))?;
        let url = object_url(&info.url, &new.id);
        let now = (self.clock)();
        let ics = vevent::serialize(&new, now);

        let mut cache = self.cache.lock().await;
        let etag = cal
            .client
            .create(&url, ics.clone())
            .await
            .map_err(|e| self.backend(cal, e))?;
        tracing::info!(calendar = cal.number, "event added on the frame saved to CalDAV");
        let window = cache
            .window
            .unwrap_or_else(|| Range::around(now))
            .union(&Range::new(new.start, new.end));
        let object = self.expand(cal, url, etag, &ics, window);
        let event = object
            .events
            .first()
            .cloned()
            .ok_or_else(|| CalendarError::Backend("the saved event couldn't be read back".into()))?;
        cache.calendars.resize(self.calendars.len(), None);
        let objects = cache.calendars[i].get_or_insert_with(Vec::new);
        objects.retain(|o| o.url != object.url);
        objects.push(object);
        self.remerge(&mut cache);
        Ok(event)
    }

    async fn remove(&self, id: &str) -> Result<(), CalendarError> {
        if !self.can_write() {
            return Err(CalendarError::ReadOnly);
        }
        let mut cache = self.cache.lock().await;
        // Where the event is: from the last read (or the add that made it) …
        let mut found = cache.calendars.iter().enumerate().find_map(|(i, objs)| {
            objs.as_ref()?
                .iter()
                .find(|o| o.events.iter().any(|e| e.local && e.id == id))
                .map(|o| (i, o.url.clone(), o.etag.clone()))
        });
        // … or, not seen yet (say, just after a restart), where the frame would have put it.
        let id_ok =
            !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if found.is_none() && id_ok {
            for (i, cal) in self.calendars.iter().enumerate().filter(|(_, c)| c.writable()) {
                let Ok(info) = cal.discover().await else { continue };
                let url = object_url(&info.url, id);
                match cal.client.etag(&url).await {
                    Ok(Some(etag)) => {
                        found = Some((i, url, Some(etag)));
                        break;
                    }
                    Ok(None) | Err(DavError::NotFound) => {}
                    Err(e) => return Err(self.backend(cal, e)),
                }
            }
        }
        let Some((i, url, etag)) = found else {
            return Err(CalendarError::NotFound);
        };
        let cal = &self.calendars[i];
        if cal.read_only {
            return Err(CalendarError::ReadOnly);
        }
        let etag = match etag {
            Some(t) => t,
            None => match cal.client.etag(&url).await {
                Ok(Some(t)) => t,
                Ok(None) => return Err(self.backend(cal, DavError::BadResponse("no ETag".into()))),
                // Gone already: nothing to delete.
                Err(DavError::NotFound) => String::new(),
                Err(e) => return Err(self.backend(cal, e)),
            },
        };
        if !etag.is_empty() {
            cal.client.delete(&url, &etag).await.map_err(|e| self.backend(cal, e))?;
            tracing::info!(calendar = cal.number, "event removed on the frame deleted from CalDAV");
        }
        if let Some(Some(objs)) = cache.calendars.get_mut(i) {
            objs.retain(|o| o.url != url);
        }
        self.remerge(&mut cache);
        Ok(())
    }
}
