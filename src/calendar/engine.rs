//! The household layer over [`ical_expand`]: occurrences become [`Event`]s attributed to people,
//! with `notes`, "Travel:" and "Walk:" read from descriptions as upstream's `ics.js` does, a
//! stable id, and the same invite on several calendars merged into one event.
//!
//! Pure: no fetching, no clock. iCalendar itself (parsing, recurrence, timezones) is the
//! `ical-expand` crate in `crates/ical-expand`, which documents why it isn't a library.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono_tz::Tz;
pub use ical_expand::{MAX_PER_SERIES, local_to_utc, midnight, resolve_tz};
use regex::Regex;

use super::{Event, Range, When};

/// Longest `notes`, in characters, as upstream.
const NOTES_MAX: usize = 280;

/// Reads one .ics file (the iCalendar text a calendar link returns) and expands it into the
/// events overlapping `window`, attributed to `people`.
///
/// `household` is the household's zone, for floating times and all-day dates. Errors only when
/// the text isn't iCalendar at all; malformed events inside the file are skipped.
pub fn parse_feed(text: &str, people: &[String], window: Range, household: Tz) -> Result<Vec<Event>, String> {
    let occurrences = ical_expand::expand(text, window.from, window.to, household).map_err(|e| e.to_string())?;
    Ok(occurrences.into_iter().map(|o| to_event(o, people)).collect())
}

// ------------------------------------------------------------------ description extras

static TRAVEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)travel(?:\s*time)?\s*[:\-\x{2013}=]?\s*(\d{1,3})").unwrap());
static WALK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)walk(?:ing)?(?:\s*time)?\s*[:\-\x{2013}=]?\s*(\d{1,3})").unwrap());
static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

/// A description as plain text: Google keeps formatted descriptions as HTML.
pub fn plain_text(desc: &str) -> String {
    let s = BR.replace_all(desc, "\n");
    let s = TAG.replace_all(&s, "");
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn minutes(re: &Regex, text: &str) -> Option<u32> {
    re.captures(text).and_then(|c| c[1].parse().ok()).filter(|n| *n > 0)
}

/// "Travel: 15", "travel 15", "Travel time - 15 min": the leave-by fallback, in minutes.
pub fn travel_from(text: &str) -> Option<u32> {
    minutes(&TRAVEL, text)
}

/// "Walk: 5": minutes added to a live drive time (parking, the walk to the door).
pub fn walk_from(text: &str) -> Option<u32> {
    minutes(&WALK, text)
}

/// A short, stable id for a key (FNV-1a, 64 bits).
pub fn id_for(key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("ev-{h:016x}")
}

fn start_string(w: &When) -> String {
    match w {
        When::Timed { start, .. } => super::iso(*start),
        When::AllDay { start, .. } => start.to_string(),
    }
}

fn to_event(o: ical_expand::Occurrence, people: &[String]) -> Event {
    let when = o.time.into();
    let desc = o.description.as_deref().map(plain_text).unwrap_or_default();
    let desc = desc.trim();
    let notes = (!desc.is_empty()).then(|| {
        if desc.chars().count() > NOTES_MAX {
            desc.chars().take(NOTES_MAX - 3).collect::<String>() + "…"
        } else {
            desc.to_string()
        }
    });
    let uid = o
        .uid
        .unwrap_or_else(|| format!("~{}", o.summary.as_deref().unwrap_or("")));
    let key = format!("{uid}|{}", start_string(&when));
    Event {
        id: id_for(&key),
        key,
        title: o.summary.unwrap_or_else(|| "(busy)".into()),
        people: people.to_vec(),
        when,
        location: o.location,
        notes,
        travel: travel_from(desc),
        walk: walk_from(desc),
        local: false,
    }
}

/// Merges events from several calendars: the same invite (same [`Event::key`]) on two people's
/// calendars becomes one event with both people, in the order the calendars were given. Sorted
/// by start (all-day dates at midnight in `tz`), then title.
pub fn merge(feeds: impl IntoIterator<Item = Vec<Event>>, tz: Tz) -> Vec<Event> {
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Event> = Vec::new();
    for ev in feeds.into_iter().flatten() {
        match by_key.get(&ev.key) {
            Some(&i) => {
                for p in ev.people {
                    if !out[i].people.contains(&p) {
                        out[i].people.push(p);
                    }
                }
            }
            None => {
                by_key.insert(ev.key.clone(), out.len());
                out.push(ev);
            }
        }
    }
    out.sort_by(|a, b| {
        a.when
            .instants(tz)
            .0
            .cmp(&b.when.instants(tz).0)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_and_walk_like_upstream() {
        assert_eq!(travel_from("Travel: 15"), Some(15));
        assert_eq!(travel_from("travel time - 20 min"), Some(20));
        assert_eq!(travel_from("Travel\u{2013}7"), Some(7));
        assert_eq!(travel_from("no travel here"), None);
        assert_eq!(travel_from("Travel: 0"), None);
        assert_eq!(walk_from("Walking time: 5"), Some(5));
        assert_eq!(plain_text("a<br>b&nbsp;<b>c</b> &amp; d"), "a\nb c & d");
    }
}
