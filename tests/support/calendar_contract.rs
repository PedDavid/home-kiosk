//! The shared `CalendarSource` contract suite. Every adapter must pass it.
//!
//! Use it from an adapter's test file:
//!
//! ```ignore
//! #[path = "support/calendar_contract.rs"]
//! mod calendar_contract;
//! use calendar_contract::{Harness, Seed};
//!
//! #[tokio::test]
//! async fn my_adapter_passes_the_calendar_contract() {
//!     calendar_contract::run(|seed: Seed| async move {
//!         // Serve each `seed.calendars[i].ics` from a fake backend (failing for `broken` ones),
//!         // with a clock fixed at `seed.now`, and return the adapter.
//!         Harness { source: Arc::new(my_adapter), secrets: vec![/* URLs, passwords */] }
//!     })
//!     .await;
//! }
//! ```
//!
//! The seed is iCalendar text because every calendar backend we target speaks it: an ICS
//! adapter serves it as a feed, a CalDAV fake as `calendar-data`.

#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use home_hq::calendar::{CalendarError, CalendarSource, Event, NewEvent, Range, When};

/// One person's (or a shared) calendar.
#[derive(Debug, Clone)]
pub struct SeedCalendar {
    /// Person ids the calendar belongs to.
    pub people: Vec<String>,
    /// Its contents, as an iCalendar document.
    pub ics: String,
    /// The backend must fail for this calendar (an ICS feed answers 404, a CalDAV server 500).
    pub broken: bool,
}

/// What a harness must set up an adapter with.
#[derive(Debug, Clone)]
pub struct Seed {
    /// The adapter's injected clock must read this.
    pub now: DateTime<Utc>,
    /// The household's zone.
    pub tz: Tz,
    /// Every person id in the settings.
    pub people: Vec<String>,
    pub calendars: Vec<SeedCalendar>,
}

/// An adapter set up from a [`Seed`].
pub struct Harness {
    pub source: Arc<dyn CalendarSource>,
    /// Strings that must never show in `errors` (URLs, tokens, passwords).
    pub secrets: Vec<String>,
}

pub const ALEX_ICS: &str = include_str!("../fixtures/calendar/contract/alex.ics");
pub const SAM_ICS: &str = include_str!("../fixtures/calendar/contract/sam.ics");

/// Thursday 5 November 2026, 17:30 UTC (17:30 in Lisbon, winter time).
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap()
}

pub fn seed(broken_sam: bool) -> Seed {
    Seed {
        now: now(),
        tz: chrono_tz::Europe::Lisbon,
        people: vec!["alex".into(), "sam".into()],
        calendars: vec![
            SeedCalendar {
                people: vec!["alex".into()],
                ics: ALEX_ICS.into(),
                broken: false,
            },
            SeedCalendar {
                people: vec!["sam".into()],
                ics: SAM_ICS.into(),
                broken: broken_sam,
            },
        ],
    }
}

/// From a day before `now` to three weeks after.
pub fn range() -> Range {
    Range::new(now() - Duration::days(1), now() + Duration::days(21))
}

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn titled<'a>(events: &'a [Event], title: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.title == title).collect()
}

fn starts(events: &[&Event]) -> Vec<DateTime<Utc>> {
    events
        .iter()
        .map(|e| match e.when {
            When::Timed { start, .. } => start,
            When::AllDay { .. } => panic!("{} should be timed", e.title),
        })
        .collect()
}

/// Runs every check, each against a fresh adapter from `make`.
pub async fn run<F, Fut>(make: F)
where
    F: Fn(Seed) -> Fut,
    Fut: Future<Output = Harness>,
{
    lists_the_range_sorted(make(seed(false)).await).await;
    attributes_and_merges(make(seed(false)).await).await;
    expands_recurrences(make(seed(false)).await).await;
    keeps_all_day_dates(make(seed(false)).await).await;
    ids_are_unique_and_stable(make(seed(false)).await).await;
    a_broken_calendar_hides_nothing_else(make(seed(true)).await).await;
    writes_follow_the_capability(make(seed(false)).await).await;
}

/// An unconfigured adapter (no calendars at all) must say so and list nothing.
pub async fn check_not_configured(source: &dyn CalendarSource) {
    assert!(!source.configured(), "an adapter with no calendars is not configured");
    assert!(source.list(range()).await.events.is_empty());
}

async fn lists_the_range_sorted(h: Harness) {
    let tz = chrono_tz::Europe::Lisbon;
    assert!(h.source.configured());
    let r = range();
    let l = h.source.list(r).await;
    assert!(l.errors.is_empty(), "unexpected errors: {:?}", l.errors);
    assert!(!l.events.is_empty());
    assert!(
        titled(&l.events, "Long ago").is_empty(),
        "events outside the range are left out"
    );
    for e in &l.events {
        assert!(e.when.overlaps(&r, tz), "{} is outside the range", e.title);
        if let When::Timed { start, end } = e.when {
            assert!(end >= start, "{} ends before it starts", e.title);
        }
    }
    let order: Vec<_> = l.events.iter().map(|e| e.when.instants(tz).0).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "events are sorted by start");

    // A narrower range narrows the listing.
    let day = Range::new(utc(2026, 11, 6, 0, 0), utc(2026, 11, 7, 0, 0));
    let titles: Vec<_> = h.source.list(day).await.events.into_iter().map(|e| e.title).collect();
    assert_eq!(titles, ["Dentist"]);
}

async fn attributes_and_merges(h: Harness) {
    let l = h.source.list(range()).await;
    let dentist = titled(&l.events, "Dentist");
    assert_eq!(dentist.len(), 1);
    assert_eq!(dentist[0].people, ["alex"]);
    assert_eq!(dentist[0].location.as_deref(), Some("Rua Augusta 10"));
    assert_eq!(dentist[0].travel, Some(20));
    assert!(
        dentist[0]
            .notes
            .as_deref()
            .unwrap()
            .starts_with("Bring the insurance card.")
    );
    for gym in titled(&l.events, "Gym") {
        assert_eq!(gym.people, ["sam"]);
    }
    let dinner = titled(&l.events, "Dinner with friends");
    assert_eq!(dinner.len(), 1, "the invite on both calendars is one event");
    let mut people = dinner[0].people.clone();
    people.sort();
    assert_eq!(people, ["alex", "sam"]);
    assert_eq!(starts(&dinner), [utc(2026, 11, 7, 20, 0)]);
}

async fn expands_recurrences(h: Harness) {
    let l = h.source.list(range()).await;
    // Mondays at 18:00 Lisbon (UTC in winter): the 9th is an EXDATE, the 16th was moved to 19:00.
    assert_eq!(starts(&titled(&l.events, "Swim")), [utc(2026, 11, 23, 18, 0)]);
    let late = titled(&l.events, "Swim (late)");
    assert_eq!(starts(&late), [utc(2026, 11, 16, 19, 0)]);
    assert_eq!(late[0].people, ["alex"]);
    // Tuesdays and Thursdays, six times from 3 November: the 3rd is before the range.
    assert_eq!(
        starts(&titled(&l.events, "Gym")),
        [
            utc(2026, 11, 5, 19, 0),
            utc(2026, 11, 10, 19, 0),
            utc(2026, 11, 12, 19, 0),
            utc(2026, 11, 17, 19, 0),
            utc(2026, 11, 19, 19, 0),
        ]
    );
}

async fn keeps_all_day_dates(h: Harness) {
    let l = h.source.list(range()).await;
    let conf = titled(&l.events, "Conference");
    assert_eq!(conf.len(), 1);
    assert_eq!(
        conf[0].when,
        When::AllDay {
            start: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(),
        }
    );
}

async fn ids_are_unique_and_stable(h: Harness) {
    let a = h.source.list(range()).await;
    let b = h.source.list(range()).await;
    let ids: Vec<_> = a.events.iter().map(|e| e.id.clone()).collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "ids are unique");
    assert_eq!(
        ids,
        b.events.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        "ids are stable"
    );
}

async fn a_broken_calendar_hides_nothing_else(h: Harness) {
    let l = h.source.list(range()).await;
    assert!(
        !titled(&l.events, "Dentist").is_empty(),
        "the healthy calendar still shows"
    );
    assert!(titled(&l.events, "Gym").is_empty(), "the broken calendar never loaded");
    assert!(!l.errors.is_empty(), "the failure is reported");
    for e in &l.errors {
        for s in &h.secrets {
            assert!(!e.contains(s.as_str()), "error {e:?} leaks a secret");
        }
    }
    let dinner = titled(&l.events, "Dinner with friends");
    assert_eq!(dinner[0].people, ["alex"]);
}

async fn writes_follow_the_capability(h: Harness) {
    let new = NewEvent {
        id: "contract-new-1".into(),
        title: "Haircut".into(),
        people: vec!["sam".into()],
        start: utc(2026, 11, 8, 10, 0),
        end: utc(2026, 11, 8, 11, 0),
    };
    if !h.source.can_write() {
        assert_eq!(h.source.add(new).await, Err(CalendarError::ReadOnly));
        assert_eq!(h.source.remove("anything").await, Err(CalendarError::ReadOnly));
        return;
    }
    let added = h.source.add(new).await.expect("add");
    let l = h.source.list(range()).await;
    let found = titled(&l.events, "Haircut");
    assert_eq!(found.len(), 1);
    assert!(found[0].local, "events added on the frame are marked local");
    assert_eq!(found[0].people, ["sam"]);
    h.source.remove(&added.id).await.expect("remove");
    assert!(titled(&h.source.list(range()).await.events, "Haircut").is_empty());
}
