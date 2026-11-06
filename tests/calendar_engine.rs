//! The household layer of the calendar engine: attribution, merging, notes and "Travel:".
//!
//! What iCalendar features expand to is `crates/ical-expand`'s fixture table.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use home_hq::calendar::Range;
use home_hq::calendar::engine::{merge, parse_feed};

const LISBON: Tz = chrono_tz::Europe::Lisbon;

/// `contract/…` from this crate's fixtures, anything else from `ical-expand`'s.
fn fixture(name: &str) -> String {
    let dir = if name.starts_with("contract/") {
        "tests/fixtures/calendar"
    } else {
        "crates/ical-expand/tests/fixtures"
    };
    let path = format!("{}/{dir}/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn window() -> Range {
    Range::new(at("2026-11-01T00:00:00Z"), at("2026-12-01T00:00:00Z"))
}

#[test]
fn descriptions_give_notes_travel_and_walk() {
    let events = parse_feed(&fixture("google.ics"), &["alex".into()], window(), LISBON).unwrap();
    let dentist = events.iter().find(|e| e.title == "Dentist").unwrap();
    assert_eq!(
        dentist.notes.as_deref(),
        Some("Bring the insurance card and X-rays.\nTravel: 20\nWalk: 5")
    );
    assert_eq!(dentist.travel, Some(20));
    assert_eq!(dentist.walk, Some(5));
    assert_eq!(
        dentist.location.as_deref(),
        Some("Clínica Dentária, Rua Augusta 10, Lisboa")
    );
    let long = events.iter().find(|e| e.title == "Long notes").unwrap();
    let notes = long.notes.as_deref().unwrap();
    assert_eq!(
        notes.chars().count(),
        278,
        "277 characters and an ellipsis, as upstream"
    );
    assert!(notes.ends_with('…'));
    let busy = events.iter().find(|e| e.title == "(busy)").unwrap();
    assert_eq!(
        (busy.notes.as_deref(), busy.location.as_deref(), busy.travel),
        (None, None, None)
    );
}

#[test]
fn a_shared_invite_on_two_calendars_is_one_event_with_both_people() {
    let alex = parse_feed(&fixture("contract/alex.ics"), &["alex".into()], window(), LISBON).unwrap();
    let sam = parse_feed(&fixture("contract/sam.ics"), &["sam".into()], window(), LISBON).unwrap();
    let merged = merge([alex.clone(), sam.clone()], LISBON);
    let dinners: Vec<_> = merged.iter().filter(|e| e.title == "Dinner with friends").collect();
    assert_eq!(dinners.len(), 1);
    assert_eq!(dinners[0].people, ["alex", "sam"]);
    // The order of people follows the calendars' order.
    let merged = merge([sam, alex], LISBON);
    let dinner = merged.iter().find(|e| e.title == "Dinner with friends").unwrap();
    assert_eq!(dinner.people, ["sam", "alex"]);
    // Everything else keeps its one owner.
    for e in merged.iter().filter(|e| e.title != "Dinner with friends") {
        let owner = if e.title == "Gym" { "sam" } else { "alex" };
        assert_eq!(e.people, [owner], "{}", e.title);
    }
}

#[test]
fn attribution_is_the_calendars_people() {
    let people: Vec<String> = vec!["alex".into(), "sam".into()];
    let events = parse_feed(&fixture("allday.ics"), &people, window(), LISBON).unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e.people == people));
}

#[test]
fn ids_are_stable_and_unique_per_occurrence() {
    let a = parse_feed(&fixture("weekly.ics"), &["alex".into()], window(), LISBON).unwrap();
    let b = parse_feed(&fixture("weekly.ics"), &["alex".into()], window(), LISBON).unwrap();
    let ids: Vec<_> = a.iter().map(|e| &e.id).collect();
    assert_eq!(ids, b.iter().map(|e| &e.id).collect::<Vec<_>>());
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}
