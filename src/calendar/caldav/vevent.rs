//! iCalendar text for an event added on the frame (RFC 5545): one `VEVENT` in a `VCALENDAR`.
//!
//! Times are written in UTC: a one-off event needs no `VTIMEZONE`, and every client shows it in
//! its own zone. The people it was added for go in `X-HOME-HQ-PEOPLE`, so attribution survives a
//! write to a shared calendar (see [`people_of`]).

use chrono::{DateTime, Utc};

use crate::calendar::NewEvent;

/// Appended to the frontend's id to make the iCalendar `UID`, and how a listing recognises
/// events added on the frame.
pub const UID_SUFFIX: &str = "@home-hq";

const PEOPLE_PROP: &str = "X-HOME-HQ-PEOPLE";

/// The `DESCRIPTION` of events added on the frame, as upstream writes to Google Calendar.
pub const DESCRIPTION: &str = "Added on the Home HQ frame";

/// The `UID` for an event added on the frame with this id.
pub fn uid_for(id: &str) -> String {
    format!("{id}{UID_SUFFIX}")
}

/// The frontend id of an event added on the frame, from its `UID`.
pub fn frame_id(uid: &str) -> Option<&str> {
    uid.strip_suffix(UID_SUFFIX).filter(|id| !id.is_empty())
}

fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// TEXT value escaping (RFC 5545 §3.3.11). Control characters other than newlines are dropped.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Folds a content line at 75 octets, never inside a UTF-8 character, with CRLF endings.
pub fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        // The first line takes 75 octets; continuation lines start with a space, so 74 more.
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
    out
}

/// The `.ics` body for a new event.
pub fn serialize(e: &NewEvent, now: DateTime<Utc>) -> String {
    let lines = [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Home HQ//home-kiosk//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", uid_for(&e.id)),
        format!("DTSTAMP:{}", stamp(now)),
        format!("CREATED:{}", stamp(now)),
        format!("DTSTART:{}", stamp(e.start)),
        format!("DTEND:{}", stamp(e.end)),
        format!("SUMMARY:{}", escape(&e.title)),
        format!("DESCRIPTION:{DESCRIPTION}"),
        format!("{PEOPLE_PROP}:{}", escape(&e.people.join(","))),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
    ];
    lines.iter().map(|l| fold(l)).collect()
}

/// Unfolded content lines of an iCalendar text.
fn lines(ics: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), out.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => out.push(raw.to_string()),
        }
    }
    out
}

/// The value of the first `name` property (ignoring parameters), unescaped only for commas.
fn prop(ics: &str, name: &str) -> Option<String> {
    lines(ics).into_iter().find_map(|l| {
        let (head, value) = l.split_once(':')?;
        let n = head.split(';').next()?;
        n.eq_ignore_ascii_case(name).then(|| value.to_string())
    })
}

/// The `UID` of the (first) event in an object.
pub fn uid_of(ics: &str) -> Option<String> {
    prop(ics, "UID").map(|u| u.trim().to_string())
}

/// The people an event added on the frame was for, when it says.
pub fn people_of(ics: &str) -> Option<Vec<String>> {
    let v = prop(ics, PEOPLE_PROP)?;
    let people: Vec<String> = v
        .split("\\,")
        .flat_map(|p| p.split(','))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    (!people.is_empty()).then_some(people)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn new_event(title: &str) -> NewEvent {
        NewEvent {
            id: "m1abc2".into(),
            title: title.into(),
            people: vec!["alex".into(), "sam".into()],
            start: Utc.with_ymd_and_hms(2026, 11, 8, 10, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 11, 8, 11, 30, 0).unwrap(),
        }
    }

    #[test]
    fn writes_a_utc_vevent() {
        let now = Utc.with_ymd_and_hms(2026, 11, 6, 9, 15, 0).unwrap();
        let ics = serialize(&new_event("Haircut, then lunch; ok\\"), now);
        assert_eq!(
            ics,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Home HQ//home-kiosk//EN\r\nCALSCALE:GREGORIAN\r\n\
             BEGIN:VEVENT\r\nUID:m1abc2@home-hq\r\nDTSTAMP:20261106T091500Z\r\nCREATED:20261106T091500Z\r\n\
             DTSTART:20261108T100000Z\r\nDTEND:20261108T113000Z\r\n\
             SUMMARY:Haircut\\, then lunch\\; ok\\\\\r\nDESCRIPTION:Added on the Home HQ frame\r\n\
             X-HOME-HQ-PEOPLE:alex\\,sam\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        assert_eq!(uid_of(&ics).as_deref(), Some("m1abc2@home-hq"));
        assert_eq!(people_of(&ics), Some(vec!["alex".to_string(), "sam".to_string()]));
    }

    #[test]
    fn newlines_and_controls_are_escaped_or_dropped() {
        assert_eq!(escape("a\r\nb\u{7}c"), "a\\nbc");
    }

    #[test]
    fn folds_long_lines_on_character_boundaries() {
        let title = "Aniversário da avó 🎂 ".repeat(8);
        let folded = fold(&format!("SUMMARY:{title}"));
        for l in folded.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(l.len() <= 75, "{} octets: {l:?}", l.len());
        }
        let unfolded: String = folded.strip_suffix("\r\n").unwrap().split("\r\n ").collect();
        assert_eq!(unfolded, format!("SUMMARY:{title}"));
        assert_eq!(prop(&folded, "SUMMARY").as_deref(), Some(title.as_str()));
    }

    #[test]
    fn frame_ids_round_trip() {
        assert_eq!(frame_id(&uid_for("x1")), Some("x1"));
        assert_eq!(frame_id("abc@google.com"), None);
        assert_eq!(frame_id("@home-hq"), None);
    }
}
