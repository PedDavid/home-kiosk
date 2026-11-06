//! ICS feeds and CalDAV calendars side by side, from one `HQ_CALENDARS`.
//!
//! Listings are merged with the engine's rules, so the same invite on an ICS feed and a CalDAV
//! calendar is one event. Writes go to the sources that can take them (only CalDAV).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::DateTime;
use chrono_tz::Tz;

use super::caldav::{self, CalDavSource, dav::Transport};
use super::fetch::Fetch;
use super::ics::IcsSource;
use super::{CalendarError, CalendarSource, Event, Listing, NewEvent, NotConfigured, Range, engine};
use crate::{Clock, settings::Settings};

pub struct Mixed {
    sources: Vec<Arc<dyn CalendarSource>>,
    tz: Tz,
}

impl Mixed {
    pub fn new(sources: Vec<Arc<dyn CalendarSource>>, tz: Tz) -> Self {
        Mixed { sources, tz }
    }
}

/// The calendar source for a raw `HQ_CALENDARS` value: ICS entries to [`IcsSource`],
/// `"type": "caldav"` ones to [`CalDavSource`], combined when both are present.
pub fn from_env_value(
    value: Option<&str>,
    settings: &Settings,
    fetch: Arc<dyn Fetch>,
    transport: Arc<dyn Transport>,
    clock: Clock,
) -> Arc<dyn CalendarSource> {
    let tz = settings.tz().unwrap_or(chrono_tz::UTC);
    let household: Vec<String> = settings.people.iter().map(|p| p.id.clone()).collect();
    let split = caldav::config::split(value);
    let mut sources: Vec<Arc<dyn CalendarSource>> = Vec::new();
    if let Some(ics) = IcsSource::from_env_value(split.ics.as_deref(), &household, tz, fetch, clock.clone()) {
        sources.push(Arc::new(ics));
    }
    if !split.caldav.is_empty() || !split.errors.is_empty() {
        for e in &split.errors {
            tracing::error!(error = %e, "calendars");
        }
        sources.push(Arc::new(
            CalDavSource::new(
                split.caldav,
                &household,
                settings.calendar.write_to.clone(),
                tz,
                transport,
                clock,
            )
            .with_config_errors(split.errors),
        ));
    }
    match sources.len() {
        0 => Arc::new(NotConfigured),
        1 => sources.remove(0),
        _ => Arc::new(Mixed::new(sources, tz)),
    }
}

#[async_trait]
impl CalendarSource for Mixed {
    fn configured(&self) -> bool {
        self.sources.iter().any(|s| s.configured())
    }

    fn can_write(&self) -> bool {
        self.sources.iter().any(|s| s.can_write())
    }

    async fn list(&self, range: Range) -> Listing {
        let listings = futures_util::future::join_all(self.sources.iter().map(|s| s.list(range))).await;
        let checked = listings.iter().map(|l| l.checked).max().unwrap_or(DateTime::UNIX_EPOCH);
        let errors = listings.iter().flat_map(|l| l.errors.iter().cloned()).collect();
        let events: Vec<Vec<Event>> = listings.into_iter().map(|l| l.events).collect();
        Listing {
            events: engine::merge(events, self.tz),
            errors,
            checked,
        }
    }

    async fn add(&self, new: NewEvent) -> Result<Event, CalendarError> {
        match self.sources.iter().find(|s| s.can_write()) {
            Some(s) => s.add(new).await,
            None => Err(CalendarError::ReadOnly),
        }
    }

    async fn remove(&self, id: &str) -> Result<(), CalendarError> {
        let mut result = Err(CalendarError::ReadOnly);
        for s in self.sources.iter().filter(|s| s.can_write()) {
            result = s.remove(id).await;
            if result != Err(CalendarError::NotFound) {
                return result;
            }
        }
        result
    }
}
