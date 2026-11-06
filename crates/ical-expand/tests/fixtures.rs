//! The fixture table: what each iCalendar feature in `tests/fixtures/` expands to.
//!
//! Every case lists the occurrences (title, then UTC start/end or all-day dates) expected in a
//! window. Add a case here for every .ics quirk found in the wild.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use ical_expand::{Occurrence, Time, expand};

const LISBON: Tz = chrono_tz::Europe::Lisbon;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn title(o: &Occurrence) -> String {
    o.summary.clone().unwrap_or_else(|| "(busy)".into())
}

/// `Title 2026-11-02T18:00Z/2026-11-02T19:00Z`, or `Title 2026-11-20/2026-11-27` for all-day.
fn render(o: &Occurrence) -> String {
    match o.time {
        Time::Timed { start, end } => format!(
            "{} {}/{}",
            title(o),
            start.format("%Y-%m-%dT%H:%MZ"),
            end.format("%Y-%m-%dT%H:%MZ")
        ),
        Time::AllDay { start, end } => format!("{} {start}/{end}", title(o)),
    }
}

struct Case {
    name: &'static str,
    fixture: &'static str,
    from: &'static str,
    to: &'static str,
    /// Only events with this title (all when `None`).
    title: Option<&'static str>,
    expect: &'static [&'static str],
}

const CASES: &[Case] = &[
    // ---------------------------------------------------------- weekly
    Case {
        name: "fortnightly on two weekdays, UNTIL inclusive (given as local time)",
        fixture: "weekly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-01-01T00:00:00Z",
        title: Some("Piano"),
        expect: &[
            "Piano 2026-11-03T17:00Z/2026-11-03T17:45Z",
            "Piano 2026-11-05T17:00Z/2026-11-05T17:45Z",
            "Piano 2026-11-17T17:00Z/2026-11-17T17:45Z",
            "Piano 2026-11-19T17:00Z/2026-11-19T17:45Z",
            "Piano 2026-12-01T17:00Z/2026-12-01T17:45Z",
            "Piano 2026-12-03T17:00Z/2026-12-03T17:45Z",
        ],
    },
    Case {
        name: "EXDATE (TZID and a UTC list), an edited and a cancelled instance",
        fixture: "weekly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2026-12-22T00:00:00Z",
        title: None,
        expect: &[
            "Swim 2026-11-02T18:00Z/2026-11-02T19:00Z",
            "Piano 2026-11-03T17:00Z/2026-11-03T17:45Z",
            "Book club 2026-11-04T20:00Z/2026-11-04T21:30Z",
            "Piano 2026-11-05T17:00Z/2026-11-05T17:45Z",
            "Swim (moved) 2026-11-16T19:30Z/2026-11-16T20:30Z",
            "Piano 2026-11-17T17:00Z/2026-11-17T17:45Z",
            "Piano 2026-11-19T17:00Z/2026-11-19T17:45Z",
            "Piano 2026-12-01T17:00Z/2026-12-01T17:45Z",
            "Piano 2026-12-03T17:00Z/2026-12-03T17:45Z",
            "Swim 2026-12-07T18:00Z/2026-12-07T19:00Z",
            "Swim 2026-12-21T18:00Z/2026-12-21T19:00Z",
        ],
    },
    Case {
        name: "an edited instance moved out of the window is gone from its old slot",
        fixture: "weekly.ics",
        from: "2026-09-29T00:00:00Z",
        to: "2026-10-01T00:00:00Z",
        title: Some("Book club"),
        expect: &[],
    },
    // ---------------------------------------------------------- monthly and yearly
    Case {
        name: "monthly by day of month, all-day",
        fixture: "monthly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-01-01T00:00:00Z",
        title: Some("Rent"),
        expect: &["Rent 2026-11-01/2026-11-02", "Rent 2026-12-01/2026-12-02"],
    },
    Case {
        name: "monthly on the 31st skips short months",
        fixture: "monthly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-01-01T00:00:00Z",
        title: Some("Month-end report"),
        expect: &["Month-end report 2026-12-31T09:00Z/2026-12-31T10:00Z"],
    },
    Case {
        name: "monthly on the first Thursday, COUNT ends it",
        fixture: "monthly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-03-01T00:00:00Z",
        title: Some("Parents' evening"),
        expect: &[
            "Parents' evening 2026-11-05T18:30Z/2026-11-05T19:30Z",
            "Parents' evening 2026-12-03T18:30Z/2026-12-03T19:30Z",
        ],
    },
    Case {
        name: "monthly on the last Friday",
        fixture: "monthly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-01-01T00:00:00Z",
        title: Some("Payday"),
        expect: &[
            "Payday 2026-11-27T12:00Z/2026-11-27T12:15Z",
            "Payday 2026-12-25T12:00Z/2026-12-25T12:15Z",
        ],
    },
    Case {
        name: "yearly all-day without DTEND",
        fixture: "monthly.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2027-01-01T00:00:00Z",
        title: Some("Anniversary"),
        expect: &["Anniversary 2026-11-20/2026-11-21"],
    },
    // ---------------------------------------------------------- all-day and multi-day
    Case {
        name: "all-day, multi-day, open-ended, DATE EXDATE, overlapping the window's start",
        fixture: "allday.ics",
        from: "2026-11-01T00:00:00Z",
        to: "2026-12-01T00:00:00Z",
        title: None,
        expect: &[
            "Trip 2026-10-30/2026-11-03",
            "No school run 2026-11-06/2026-11-07",
            "Recycling 2026-11-06/2026-11-07",
            "Day off 2026-11-11/2026-11-12",
            "Holiday 2026-11-20/2026-11-27",
            "No school run 2026-11-20/2026-11-21",
            "Same-day end 2026-11-25/2026-11-26",
            "Festival 2026-11-27/2026-11-30",
        ],
    },
    Case {
        name: "an all-day event ending at the window's start is out",
        fixture: "allday.ics",
        from: "2026-11-03T00:00:00Z",
        to: "2026-11-04T00:00:00Z",
        title: None,
        expect: &[],
    },
    // ---------------------------------------------------------- daylight saving
    Case {
        name: "autumn: a weekly 09:00 stays 09:00 local when summer time ends",
        fixture: "dst.ics",
        from: "2026-10-15T00:00:00Z",
        to: "2026-11-02T00:00:00Z",
        title: Some("Sunday run"),
        expect: &[
            "Sunday run 2026-10-18T08:00Z/2026-10-18T09:00Z",
            "Sunday run 2026-10-25T09:00Z/2026-10-25T10:00Z",
            "Sunday run 2026-11-01T09:00Z/2026-11-01T10:00Z",
        ],
    },
    Case {
        name: "spring: a weekly 09:00 stays 09:00 local when summer time starts",
        fixture: "dst.ics",
        from: "2027-03-15T00:00:00Z",
        to: "2027-04-05T00:00:00Z",
        title: Some("Spring run"),
        expect: &[
            "Spring run 2027-03-21T09:00Z/2027-03-21T10:00Z",
            "Spring run 2027-03-28T08:00Z/2027-03-28T09:00Z",
            "Spring run 2027-04-04T08:00Z/2027-04-04T09:00Z",
        ],
    },
    Case {
        name: "autumn: an ambiguous 01:30 is the first one (summer time)",
        fixture: "dst.ics",
        from: "2026-10-24T00:00:00Z",
        to: "2026-10-26T00:00:00Z",
        title: Some("Night shift"),
        expect: &["Night shift 2026-10-25T00:30Z/2026-10-25T03:00Z"],
    },
    Case {
        name: "spring: 01:30 in the gap moves forward by the gap",
        fixture: "dst.ics",
        from: "2027-03-27T00:00:00Z",
        to: "2027-03-29T00:00:00Z",
        title: Some("In the gap"),
        expect: &["In the gap 2027-03-28T01:30Z/2027-03-28T02:00Z"],
    },
    Case {
        name: "another zone's transition (New York, 1 November)",
        fixture: "dst.ics",
        from: "2026-10-15T00:00:00Z",
        to: "2026-11-15T00:00:00Z",
        title: Some("New York call"),
        expect: &[
            "New York call 2026-10-21T14:00Z/2026-10-21T14:30Z",
            "New York call 2026-10-28T14:00Z/2026-10-28T14:30Z",
            "New York call 2026-11-04T15:00Z/2026-11-04T15:30Z",
            "New York call 2026-11-11T15:00Z/2026-11-11T15:30Z",
        ],
    },
    // ---------------------------------------------------------- zone names
    Case {
        name: "TZIDs: vendor prefix, Windows names, floating, unknown, DURATION, mixed zones",
        fixture: "timezones.ics",
        from: "2026-11-10T00:00:00Z",
        to: "2026-11-11T00:00:00Z",
        title: None,
        expect: &[
            "Mozilla zone 2026-11-10T10:00Z/2026-11-10T10:30Z",
            "Outlook Berlin 2026-11-10T11:00Z/2026-11-10T11:30Z",
            "Outlook London 2026-11-10T11:00Z/2026-11-10T11:30Z",
            // No zone: the household's (Lisbon).
            "Floating 2026-11-10T13:00Z/2026-11-10T14:00Z",
            // Not a known zone: the file's X-WR-TIMEZONE (New York).
            "Unknown zone 2026-11-10T14:00Z/2026-11-10T14:30Z",
            "Duration 2026-11-10T15:00Z/2026-11-10T15:45Z",
            "Paris to London 2026-11-10T17:00Z/2026-11-10T18:00Z",
            "No end 2026-11-10T20:00Z/2026-11-10T20:00Z",
        ],
    },
    // ---------------------------------------------------------- a Google export
    Case {
        name: "a Google export: CRLF, folding, no SUMMARY",
        fixture: "google.ics",
        from: "2026-11-06T00:00:00Z",
        to: "2026-11-09T00:00:00Z",
        title: None,
        expect: &[
            "Dentist 2026-11-06T12:30Z/2026-11-06T13:30Z",
            "(busy) 2026-11-07T15:00Z/2026-11-07T16:00Z",
            "Long notes 2026-11-08T10:00Z/2026-11-08T11:00Z",
        ],
    },
];

#[test]
fn fixture_table() {
    let mut failures = Vec::new();
    for c in CASES {
        let mut found =
            expand(&fixture(c.fixture), at(c.from), at(c.to), LISBON).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        found.sort_by_key(|o| (o.time.instants(LISBON).0, title(o)));
        let got: Vec<String> = found
            .iter()
            .filter(|o| c.title.is_none_or(|t| title(o) == t))
            .map(render)
            .collect();
        if got != c.expect {
            failures.push(format!("{}\n  expected {:#?}\n  got      {:#?}", c.name, c.expect, got));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn html_and_garbage_are_not_ics() {
    for text in [fixture("not-ics.html"), String::new(), "Not Found".into()] {
        assert!(expand(&text, at("2026-11-01T00:00:00Z"), at("2026-12-01T00:00:00Z"), LISBON).is_err());
    }
}

#[test]
fn a_daily_series_from_years_ago_expands_quickly() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:daily\nSUMMARY:Pills\nDTSTART;TZID=Europe/Lisbon:20000101T080000\nRRULE:FREQ=DAILY\nEND:VEVENT\nEND:VCALENDAR\n";
    let started = std::time::Instant::now();
    let found = expand(text, at("2026-11-01T00:00:00Z"), at("2026-12-01T00:00:00Z"), LISBON).unwrap();
    assert_eq!(found.len(), 30);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}
