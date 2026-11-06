//! The `CalendarSource` port: events in a time range, attributed to the household's people.
//!
//! Everything calendar-shaped the API serves goes through [`CalendarSource`]. An adapter turns
//! some backend (ICS feeds in [`ics`], CalDAV later) into [`Event`]s; the API layer only ever sees
//! this module's types, and [`Listing`] serialises to upstream's `/api/calendar` shape:
//!
//! ```json
//! { "events": [ { "id": "…", "title": "Dentist", "people": ["alex"],
//!                 "start": "2026-11-06T12:30:00.000Z", "end": "2026-11-06T13:30:00.000Z",
//!                 "loc": "Rua Augusta", "notes": "Travel: 20", "travel": 20 },
//!               { "id": "…", "title": "Holiday", "people": ["alex", "sam"], "allDay": true,
//!                 "start": "2026-11-20", "end": "2026-11-27" } ],
//!   "errors": ["Calendar 2: HTTP 404 (link wrong, or reset in Google)"],
//!   "checked": 1762450200000 }
//! ```
//!
//! The parsing and recurrence expansion shared by every iCalendar-speaking adapter lives in
//! [`engine`], which is pure: it never fetches or reads the clock.
//!
//! Every adapter must pass the shared contract suite in `tests/support/calendar_contract.rs`.

pub mod caldav;
pub mod engine;
pub mod fetch;
pub mod ics;
pub mod mixed;

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// The window upstream keeps around "now": 45 days back, 75 days ahead.
pub const PAST_DAYS: i64 = 45;
pub const FUTURE_DAYS: i64 = 75;

/// A half-open time range `[from, to)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl Range {
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Range { from, to }
    }

    /// Upstream's window around `now` (−45 to +75 days), the one `/api/calendar` serves.
    pub fn around(now: DateTime<Utc>) -> Self {
        Range::new(now - Duration::days(PAST_DAYS), now + Duration::days(FUTURE_DAYS))
    }

    pub fn contains(&self, other: &Range) -> bool {
        self.from <= other.from && other.to <= self.to
    }

    /// The smallest range covering both.
    pub fn union(&self, other: &Range) -> Range {
        Range::new(self.from.min(other.from), self.to.max(other.to))
    }
}

/// When an event happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// A timed event, as instants. `end >= start`.
    Timed { start: DateTime<Utc>, end: DateTime<Utc> },
    /// An all-day event on the household's calendar days. `end` is exclusive and `end > start`
    /// (a one-day event on the 6th is `start: 6th, end: 7th`), as upstream sends it.
    AllDay { start: NaiveDate, end: NaiveDate },
}

impl When {
    /// Whether this overlaps `range`. All-day dates are read in `tz`, the household's zone.
    /// A zero-length timed event overlaps when its start is inside the range.
    pub fn overlaps(&self, range: &Range, tz: chrono_tz::Tz) -> bool {
        let (start, end) = self.instants(tz);
        if start == end {
            range.from <= start && start < range.to
        } else {
            start < range.to && end > range.from
        }
    }

    /// The instants this covers; all-day dates start at midnight in `tz`.
    pub fn instants(&self, tz: chrono_tz::Tz) -> (DateTime<Utc>, DateTime<Utc>) {
        match *self {
            When::Timed { start, end } => (start, end),
            When::AllDay { start, end } => (engine::midnight(start, tz), engine::midnight(end, tz)),
        }
    }
}

/// One occurrence of an event, as the frontend draws it. Recurring events arrive expanded: every
/// occurrence is its own `Event` with its own `id`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Unique within a [`Listing`] and stable across refreshes (it is derived from `key`), so
    /// the frontend and a future `DELETE /api/events/:id` can refer to it.
    pub id: String,
    /// What makes two copies "the same event": the iCalendar `UID` and the occurrence's start
    /// (`<uid>|<start>`). The same invite on two people's calendars shares a key and is merged
    /// into one event listing both people ([`engine::merge`]). Not sent to the frontend.
    pub key: String,
    /// `SUMMARY`, or `(busy)` when the feed hides it.
    pub title: String,
    /// Person ids from the settings; never empty for an attributed event.
    pub people: Vec<String>,
    pub when: When,
    /// `LOCATION` (`loc` on the wire).
    pub location: Option<String>,
    /// `DESCRIPTION` as plain text; longer than 280 characters, cut to 277 plus "…" (as upstream).
    pub notes: Option<String>,
    /// Minutes, from "Travel: 15" in the description: the leave-by fallback.
    pub travel: Option<u32>,
    /// Minutes, from "Walk: 5" in the description: added to a live drive time.
    pub walk: Option<u32>,
    /// Added on the frame through [`CalendarSource::add`], so the frontend offers "Remove".
    pub local: bool,
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Serialize for Event {
    /// Upstream's event shape: `start`/`end` as ISO instants, or `allDay: true` with
    /// `YYYY-MM-DD` dates; optional fields are left out when empty.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("id", &self.id)?;
        m.serialize_entry("title", &self.title)?;
        m.serialize_entry("people", &self.people)?;
        match self.when {
            When::Timed { start, end } => {
                m.serialize_entry("start", &iso(start))?;
                m.serialize_entry("end", &iso(end))?;
            }
            When::AllDay { start, end } => {
                m.serialize_entry("allDay", &true)?;
                m.serialize_entry("start", &start.to_string())?;
                m.serialize_entry("end", &end.to_string())?;
            }
        }
        if let Some(v) = &self.location {
            m.serialize_entry("loc", v)?;
        }
        if let Some(v) = &self.notes {
            m.serialize_entry("notes", v)?;
        }
        if let Some(v) = self.travel {
            m.serialize_entry("travel", &v)?;
        }
        if let Some(v) = self.walk {
            m.serialize_entry("walk", &v)?;
        }
        if self.local {
            m.serialize_entry("local", &true)?;
        }
        m.end()
    }
}

/// What [`CalendarSource::list`] returns: `/api/calendar`'s body.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    /// Events overlapping the requested range, sorted by start.
    pub events: Vec<Event>,
    /// Problems for the Status panel, such as "Calendar 2: HTTP 404". A calendar that fails
    /// keeps its last good events. Errors never contain a calendar's URL or credentials.
    pub errors: Vec<String>,
    /// When the backend was last read.
    pub checked: DateTime<Utc>,
}

impl Serialize for Listing {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(3))?;
        m.serialize_entry("events", &self.events)?;
        m.serialize_entry("errors", &self.errors)?;
        m.serialize_entry("checked", &self.checked.timestamp_millis())?;
        m.end()
    }
}

/// An event added on the frame (`POST /api/events`): timed, with a client-chosen id.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub id: String,
    pub title: String,
    pub people: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarError {
    /// The source can't add or remove events ([`CalendarSource::can_write`] is false).
    ReadOnly,
    NotFound,
    /// The request itself is wrong (no title, end before start, unknown people, …).
    Invalid(String),
    /// The backend failed. The message must not contain URLs or credentials.
    Backend(String),
}

impl fmt::Display for CalendarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalendarError::ReadOnly => f.write_str("the calendar is read-only"),
            CalendarError::NotFound => f.write_str("no such event"),
            CalendarError::Invalid(m) | CalendarError::Backend(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CalendarError {}

/// A source of calendar events (the port).
///
/// Contract, checked by `tests/support/calendar_contract.rs` for every adapter:
///
/// - [`list`](Self::list) never fails as a whole. One broken calendar adds a line to
///   [`Listing::errors`] (without its URL) and keeps its last good events; the others still show.
/// - Events overlap the requested range and are sorted by start; recurring events are expanded
///   into occurrences, with exceptions (`EXDATE`) removed and edited instances (`RECURRENCE-ID`)
///   applied. Timed events are UTC instants; all-day events are dates with an exclusive end.
/// - Each event names the people its calendar belongs to; the same invite on several people's
///   calendars is one event listing all of them.
/// - Ids are unique in a listing and stable between calls.
/// - Without [`can_write`](Self::can_write), [`add`](Self::add) and [`remove`](Self::remove)
///   return [`CalendarError::ReadOnly`] (the defaults). With it, an added event shows up in the
///   next `list` with `local: true`, and a removed one is gone.
/// - An adapter may cache; the clock it caches against is injected, not read from the system.
///
/// The trait is object-safe (`Arc<dyn CalendarSource>`) through `async_trait`.
#[async_trait]
pub trait CalendarSource: Send + Sync {
    /// Whether any calendar is set up. When not, the API keeps serving the demo calendar.
    fn configured(&self) -> bool;

    /// Whether [`add`](Self::add) and [`remove`](Self::remove) work.
    fn can_write(&self) -> bool {
        false
    }

    /// Events overlapping `range`, plus errors to show on the Status panel.
    async fn list(&self, range: Range) -> Listing;

    /// Adds an event to the calendar of exactly `new.people`.
    async fn add(&self, new: NewEvent) -> Result<Event, CalendarError> {
        let _ = new;
        Err(CalendarError::ReadOnly)
    }

    /// Removes an event by its [`Event::id`].
    async fn remove(&self, id: &str) -> Result<(), CalendarError> {
        let _ = id;
        Err(CalendarError::ReadOnly)
    }
}

/// The source used when nothing is configured: `configured()` is false and the API falls back
/// to the demo calendar.
pub struct NotConfigured;

#[async_trait]
impl CalendarSource for NotConfigured {
    fn configured(&self) -> bool {
        false
    }

    async fn list(&self, _range: Range) -> Listing {
        Listing {
            events: Vec::new(),
            errors: Vec::new(),
            checked: DateTime::UNIX_EPOCH,
        }
    }
}
