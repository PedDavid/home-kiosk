//! An .ics file (iCalendar, RFC 5545, text) in, the occurrences that overlap a time window out.
//!
//! This is the part of Home HQ's calendar that knows iCalendar: it reads an .ics file, expands
//! recurring events and normalises times to UTC. It knows nothing about the household (people,
//! "Travel:" notes, merging calendars); the app does that on top of [`Occurrence`]s.
//!
//! ```
//! use chrono::{TimeZone, Utc};
//! let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:swim\r\nSUMMARY:Swim\r\n\
//!     DTSTART;TZID=Europe/Lisbon:20261005T180000\r\nDTEND;TZID=Europe/Lisbon:20261005T190000\r\n\
//!     RRULE:FREQ=WEEKLY\r\nEXDATE;TZID=Europe/Lisbon:20261012T180000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
//! let from = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
//! let to = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
//! let swims = ical_expand::expand(ics, from, to, chrono_tz::Europe::Lisbon).unwrap();
//! // 5 and 19 October (summer time, 17:00 UTC), then 26 October in winter time (18:00 UTC);
//! // the 12th is an EXDATE.
//! let starts: Vec<_> = swims.iter().map(|o| o.time.instants(chrono_tz::UTC).0.to_rfc3339()).collect();
//! assert_eq!(starts, ["2026-10-05T17:00:00+00:00", "2026-10-19T17:00:00+00:00", "2026-10-26T18:00:00+00:00"]);
//! ```
//!
//! What it covers, for what family calendars (Google, iCloud, Outlook, Nextcloud) put in their .ics
//! files:
//!
//! - content lines with folding, quoted parameters and text escapes. A line or event that
//!   doesn't parse is skipped, never the whole file;
//! - `RRULE` (expanded with the [`rrule`] crate), `RDATE` and `EXDATE` (TZID, UTC and DATE forms);
//! - edited and cancelled instances (`RECURRENCE-ID`), including one moved into the window from
//!   outside it, and cancelled series (`STATUS:CANCELLED`);
//! - all-day and multi-day events (`VALUE=DATE`, `DTEND` or `DURATION`);
//! - `TZID`s as IANA names, with vendor prefixes (`/mozilla.org/…/Europe/London`), or as common
//!   Windows names (`GMT Standard Time`). Unknown names fall back to the file's
//!   `X-WR-TIMEZONE`, then the caller's zone, which is also used for floating times;
//! - daylight saving per RFC 5545 §3.3.5: recurrences keep their local wall time, a time in a
//!   spring-forward gap moves forward by the gap, and an ambiguous autumn time takes the first
//!   instant;
//! - `UNTIL` in the forms exporters actually write (UTC, local time, a date), inclusive.
//!
//! Not covered: `VTIMEZONE` definitions are not interpreted (zones resolve by name),
//! `RANGE=THISANDFUTURE` is treated as a single edited instance, and `EXRULE` is ignored.
//!
//! Why this exists instead of a library is in the crate's `README.md`.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;

/// Occurrences generated per series at most, so a malformed rule can't spin.
pub const MAX_PER_SERIES: usize = 20_000;

/// When an occurrence happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Time {
    /// A timed occurrence, as instants. `end >= start`.
    Timed { start: DateTime<Utc>, end: DateTime<Utc> },
    /// An all-day occurrence. `end` is exclusive and `end > start`.
    AllDay { start: NaiveDate, end: NaiveDate },
}

impl Time {
    /// The instants this covers; all-day dates start at midnight in `tz`.
    pub fn instants(&self, tz: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
        match *self {
            Time::Timed { start, end } => (start, end),
            Time::AllDay { start, end } => (midnight(start, tz), midnight(end, tz)),
        }
    }

    /// Whether this overlaps `[from, to)`. All-day dates are read in `tz`. A zero-length
    /// occurrence overlaps when its start is inside the range.
    pub fn overlaps(&self, from: DateTime<Utc>, to: DateTime<Utc>, tz: Tz) -> bool {
        let (start, end) = self.instants(tz);
        if start == end {
            from <= start && start < to
        } else {
            start < to && end > from
        }
    }
}

/// One occurrence of an event. Text fields are unescaped but otherwise as the .ics file has them
/// (a `DESCRIPTION` may be HTML); empty ones are `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub uid: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub time: Time,
}

/// The text isn't iCalendar (no `BEGIN:VCALENDAR`), e.g. an HTML sign-in page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotICalendar;

impl std::fmt::Display for NotICalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not an iCalendar file (no BEGIN:VCALENDAR)")
    }
}

impl std::error::Error for NotICalendar {}

/// Expands every event in `text` into the occurrences overlapping `[from, to)`, in file order.
///
/// `tz` is the caller's zone: floating times, all-day dates and unknown `TZID`s (after the
/// file's `X-WR-TIMEZONE`) are read in it. Fails only when the text isn't iCalendar at all.
pub fn expand(text: &str, from: DateTime<Utc>, to: DateTime<Utc>, tz: Tz) -> Result<Vec<Occurrence>, NotICalendar> {
    let roots = parse_components(text);
    let cals: Vec<&Component> = roots.iter().filter(|c| c.name == "VCALENDAR").collect();
    if cals.is_empty() {
        return Err(NotICalendar);
    }
    let mut out = Vec::new();
    for cal in cals {
        let fallback = cal
            .prop("X-WR-TIMEZONE")
            .and_then(|p| resolve_tz(&p.value))
            .unwrap_or(tz);
        let ctx = Ctx {
            household: tz,
            fallback,
        };
        expand_calendar(cal, &ctx, (from, to), &mut out);
    }
    Ok(out)
}

// ------------------------------------------------------------------ content lines

#[derive(Debug, Clone)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Default)]
struct Component {
    name: String,
    props: Vec<Prop>,
    children: Vec<Component>,
}

impl Component {
    fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    fn props<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> + 'a {
        self.props.iter().filter(move |p| p.name == name)
    }
}

/// Splits into logical lines: CRLF or LF, and a line starting with a space or tab continues the
/// previous one (RFC 5545 §3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// `NAME;PARAM=a,"b:c";P2=x:value`. Lines that don't parse are skipped, not fatal.
fn parse_line(line: &str) -> Option<Prop> {
    let bytes = line.as_bytes();
    let mut i = bytes.iter().position(|&c| c == b';' || c == b':')?;
    let name = line[..i].trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    while bytes[i] == b';' {
        i += 1;
        let eq = i + line[i..].find('=')?;
        let key = line[i..eq].trim().to_ascii_uppercase();
        i = eq + 1;
        let mut value = String::new();
        loop {
            if i >= bytes.len() {
                return None;
            }
            if bytes[i] == b'"' {
                let close = i + 1 + line[i + 1..].find('"')?;
                value.push_str(&line[i + 1..close]);
                i = close + 1;
            } else {
                let stop = line[i..].find([',', ';', ':']).map(|n| i + n).unwrap_or(bytes.len());
                value.push_str(&line[i..stop]);
                i = stop;
            }
            if i < bytes.len() && bytes[i] == b',' {
                value.push(',');
                i += 1;
                continue;
            }
            break;
        }
        params.push((key, value));
        if i >= bytes.len() {
            return None;
        }
    }
    if bytes[i] != b':' {
        return None;
    }
    Some(Prop {
        name,
        params,
        value: line[i + 1..].to_string(),
    })
}

/// Every top-level component in the text (normally one `VCALENDAR`).
fn parse_components(text: &str) -> Vec<Component> {
    let mut stack: Vec<Component> = Vec::new();
    let mut roots = Vec::new();
    for line in unfold(text) {
        let Some(p) = parse_line(&line) else { continue };
        match p.name.as_str() {
            "BEGIN" => stack.push(Component {
                name: p.value.trim().to_ascii_uppercase(),
                ..Default::default()
            }),
            "END" => {
                let name = p.value.trim().to_ascii_uppercase();
                // Close up to the matching BEGIN; an unmatched END is ignored.
                if let Some(pos) = stack.iter().rposition(|c| c.name == name) {
                    while stack.len() > pos {
                        let c = stack.pop().unwrap();
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(c),
                            None => roots.push(c),
                        }
                    }
                }
            }
            _ => {
                if let Some(c) = stack.last_mut() {
                    c.props.push(p);
                }
            }
        }
    }
    // A truncated file: keep what was read.
    while let Some(c) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(c),
            None => roots.push(c),
        }
    }
    roots
}

/// TEXT value escapes: `\n`, `\,`, `\;`, `\\`.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

// ------------------------------------------------------------------ time values

/// A DATE or DATE-TIME value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Dt {
    Date(NaiveDate),
    Instant(DateTime<Utc>),
    /// A wall time in a zone (`TZID=…`, or floating in the household's zone).
    Local(NaiveDateTime, Tz),
}

/// Midnight starting `date` in `tz`, as an instant.
pub fn midnight(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    local_to_utc(date.and_time(NaiveTime::MIN), tz)
}

/// A wall time in `tz` as an instant. In a spring-forward gap the time moves forward by the gap;
/// an ambiguous autumn time takes the earlier instant (RFC 5545 §3.3.5).
pub fn local_to_utc(t: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&t).earliest() {
        Some(d) => d.with_timezone(&Utc),
        None => {
            // In a gap: read it with the offset in force just before (02:30 → 03:30 summer time).
            let before = tz.offset_from_utc_datetime(&(t - Duration::hours(12))).fix();
            Utc.from_utc_datetime(&(t - Duration::seconds(before.local_minus_utc() as i64)))
        }
    }
}

impl Dt {
    fn instant(&self, tz: Tz) -> DateTime<Utc> {
        match *self {
            Dt::Date(d) => midnight(d, tz),
            Dt::Instant(t) => t,
            Dt::Local(t, z) => local_to_utc(t, z),
        }
    }
}

/// How a value is matched against an occurrence (`EXDATE`, `RECURRENCE-ID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Match {
    Date(NaiveDate),
    Instant(DateTime<Utc>),
}

fn match_key(dt: &Dt) -> Match {
    match *dt {
        Dt::Date(d) => Match::Date(d),
        other => Match::Instant(other.instant(chrono_tz::UTC)),
    }
}

/// The calendar-wide context for reading values.
struct Ctx {
    /// The household's zone: floating times and all-day dates.
    household: Tz,
    /// Unknown `TZID`s fall back to this (`X-WR-TIMEZONE`, else the household's).
    fallback: Tz,
}

const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("Romance Standard Time", "Europe/Paris"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("India Standard Time", "Asia/Kolkata"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
];

/// A `TZID` as a tz-database zone: an IANA name, a name with a vendor prefix
/// (`/mozilla.org/20050126_1/Europe/Lisbon`), or a common Windows name. `None` if unknown.
pub fn resolve_tz(tzid: &str) -> Option<Tz> {
    let name = tzid.trim().trim_matches('"').trim_start_matches('/');
    if let Ok(tz) = Tz::from_str(name) {
        return Some(tz);
    }
    // Vendor prefixes: try the trailing segments.
    let parts: Vec<&str> = name.split('/').collect();
    for i in 1..parts.len() {
        if let Ok(tz) = Tz::from_str(&parts[i..].join("/")) {
            return Some(tz);
        }
    }
    WINDOWS_ZONES
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(name))
        .and_then(|(_, iana)| Tz::from_str(iana).ok())
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(..8)?, "%Y%m%d").ok()
}

/// One DATE or DATE-TIME (`20261106`, `20261106T090000`, `20261106T090000Z`).
fn parse_dt(s: &str, tzid: Option<&str>, ctx: &Ctx) -> Option<Dt> {
    let s = s.trim();
    // A PERIOD (RDATE) counts from its start.
    let s = s.split('/').next().unwrap_or(s);
    if s.len() == 8 {
        return parse_date(s).map(Dt::Date);
    }
    let (body, utc) = match s.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (s, false),
    };
    let t = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M"))
        .ok()?;
    if utc {
        return Some(Dt::Instant(Utc.from_utc_datetime(&t)));
    }
    let tz = match tzid {
        Some(id) => resolve_tz(id).unwrap_or(ctx.fallback),
        None => ctx.household,
    };
    if tz == chrono_tz::UTC {
        return Some(Dt::Instant(Utc.from_utc_datetime(&t)));
    }
    Some(Dt::Local(t, tz))
}

fn prop_dt(p: &Prop, ctx: &Ctx) -> Option<Dt> {
    let dt = parse_dt(&p.value, p.param("TZID"), ctx)?;
    // VALUE=DATE with a date-time value: take the date.
    if p.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) {
        return Some(match dt {
            Dt::Date(d) => Dt::Date(d),
            Dt::Instant(t) => Dt::Date(t.date_naive()),
            Dt::Local(t, _) => Dt::Date(t.date()),
        });
    }
    Some(dt)
}

fn prop_dts(p: &Prop, ctx: &Ctx) -> Vec<Dt> {
    p.value
        .split(',')
        .filter_map(|v| {
            let mut one = p.clone();
            one.value = v.to_string();
            prop_dt(&one, ctx)
        })
        .collect()
}

/// An RFC 5545 DURATION (`PT1H30M`, `P1D`, `-P1W`).
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut seen = false;
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            'T' => {}
            d if d.is_ascii_digit() => num.push(d),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                seen = true;
                total += match unit {
                    'W' => Duration::weeks(n),
                    'D' => Duration::days(n),
                    'H' => Duration::hours(n),
                    'M' => Duration::minutes(n),
                    'S' => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    seen.then_some(if neg { -total } else { total })
}

// ------------------------------------------------------------------ events

#[derive(Debug, Clone)]
struct VEvent {
    uid: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    cancelled: bool,
    start: Dt,
    end: Option<Dt>,
    duration: Option<Duration>,
    rrule: Option<String>,
    rdates: Vec<Dt>,
    exdates: Vec<Dt>,
    recurrence_id: Option<Dt>,
}

fn read_vevent(c: &Component, ctx: &Ctx) -> Option<VEvent> {
    let text = |name: &str| {
        c.prop(name)
            .map(|p| unescape(&p.value))
            .filter(|v| !v.trim().is_empty())
    };
    let start = prop_dt(c.prop("DTSTART")?, ctx)?;
    Some(VEvent {
        uid: c
            .prop("UID")
            .map(|p| p.value.trim().to_string())
            .filter(|u| !u.is_empty()),
        summary: text("SUMMARY"),
        description: text("DESCRIPTION"),
        location: text("LOCATION"),
        cancelled: c
            .prop("STATUS")
            .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("CANCELLED")),
        start,
        end: c.prop("DTEND").and_then(|p| prop_dt(p, ctx)),
        duration: c.prop("DURATION").and_then(|p| parse_duration(&p.value)),
        rrule: c.prop("RRULE").map(|p| p.value.trim().to_string()),
        rdates: c.props("RDATE").flat_map(|p| prop_dts(p, ctx)).collect(),
        exdates: c.props("EXDATE").flat_map(|p| prop_dts(p, ctx)).collect(),
        recurrence_id: c.prop("RECURRENCE-ID").and_then(|p| prop_dt(p, ctx)),
    })
}

/// An event's length, applied to each occurrence.
#[derive(Debug, Clone, Copy)]
enum Span {
    /// All-day events: whole days.
    Days(i64),
    /// Wall-clock length, for local times (09:00–10:00 stays that across a DST change).
    Nominal(Duration),
    /// Exact length, for UTC times.
    Exact(Duration),
}

impl VEvent {
    fn span(&self, household: Tz) -> Span {
        match (self.start, self.end, self.duration) {
            (Dt::Date(s), Some(end), _) => {
                let e = match end {
                    Dt::Date(d) => d,
                    other => other.instant(household).with_timezone(&household).date_naive(),
                };
                Span::Days((e - s).num_days().max(1))
            }
            (Dt::Date(_), None, Some(d)) => Span::Days(d.num_days().max(1)),
            (Dt::Date(_), None, None) => Span::Days(1),
            (Dt::Local(s, tz), Some(end), _) => {
                let e = match end {
                    Dt::Local(t, z) if z == tz => t,
                    other => other.instant(tz).with_timezone(&tz).naive_local(),
                };
                Span::Nominal((e - s).max(Duration::zero()))
            }
            (Dt::Local(..), None, Some(d)) => Span::Nominal(d.max(Duration::zero())),
            (Dt::Instant(s), Some(end), _) => Span::Exact((end.instant(household) - s).max(Duration::zero())),
            (Dt::Instant(_), None, Some(d)) => Span::Exact(d.max(Duration::zero())),
            (_, None, None) => Span::Exact(Duration::zero()),
        }
    }
}

/// An occurrence's start and end.
fn time_of(start: Dt, span: Span, household: Tz) -> Time {
    match (start, span) {
        (Dt::Date(d), Span::Days(n)) => Time::AllDay {
            start: d,
            end: d + Duration::days(n.max(1)),
        },
        (Dt::Date(d), _) => Time::AllDay {
            start: d,
            end: d + Duration::days(1),
        },
        (Dt::Local(t, tz), Span::Nominal(len)) => Time::Timed {
            start: local_to_utc(t, tz),
            end: local_to_utc(t + len, tz),
        },
        (other, Span::Exact(len) | Span::Nominal(len)) => {
            let s = other.instant(household);
            Time::Timed { start: s, end: s + len }
        }
        (other, Span::Days(n)) => {
            let s = other.instant(household);
            Time::Timed {
                start: s,
                end: s + Duration::days(n),
            }
        }
    }
}

// ------------------------------------------------------------------ expansion

fn rrule_tz(start: &Dt) -> rrule::Tz {
    match start {
        Dt::Local(_, tz) => rrule::Tz::Tz(*tz),
        _ => rrule::Tz::UTC,
    }
}

fn to_rrule_dt(dt: &Dt, tz: rrule::Tz) -> DateTime<rrule::Tz> {
    match *dt {
        Dt::Date(d) => Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)).with_timezone(&tz),
        other => other.instant(chrono_tz::UTC).with_timezone(&tz),
    }
}

fn from_rrule_dt(t: DateTime<rrule::Tz>, like: &Dt) -> Dt {
    match like {
        Dt::Date(_) => Dt::Date(t.date_naive()),
        Dt::Instant(_) => Dt::Instant(t.with_timezone(&Utc)),
        Dt::Local(_, tz) => Dt::Local(t.with_timezone(tz).naive_local(), *tz),
    }
}

/// The occurrence starts of a series from DTSTART up to `until` (inclusive), in order.
fn occurrences(ev: &VEvent, until: DateTime<Utc>, ctx: &Ctx) -> Vec<Dt> {
    let Some(rule) = &ev.rrule else {
        let mut all = vec![ev.start];
        all.extend(ev.rdates.iter().copied());
        all.sort_by_key(|d| d.instant(ctx.household));
        all.dedup();
        return all;
    };
    let tz = rrule_tz(&ev.start);
    let dtstart = to_rrule_dt(&ev.start, tz);

    // UNTIL is read here: exports often give it in a form the rrule crate rejects (a local time,
    // a date for a timed event), and RFC 5545 makes it inclusive whatever its form.
    let mut parts = Vec::new();
    let mut rule_until = None;
    for part in rule.split(';').filter(|p| !p.trim().is_empty()) {
        match part.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("UNTIL") => {
                let tzid = match ev.start {
                    Dt::Local(_, z) => Some(z.name()),
                    _ => None,
                };
                rule_until = parse_dt(v, tzid, ctx).map(|u| match (u, ev.start) {
                    // A date UNTIL on a timed series: through the end of that day.
                    (Dt::Date(d), Dt::Local(_, z)) => midnight(d + Duration::days(1), z) - Duration::seconds(1),
                    (Dt::Date(d), Dt::Instant(_)) => {
                        midnight(d + Duration::days(1), ctx.household) - Duration::seconds(1)
                    }
                    (Dt::Date(d), Dt::Date(_)) => Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN)),
                    (other, Dt::Date(_)) => {
                        let d = other.instant(ctx.household).with_timezone(&ctx.household).date_naive();
                        Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN))
                    }
                    (other, _) => other.instant(ctx.household),
                });
            }
            _ => parts.push(part.trim()),
        }
    }
    let single = || vec![ev.start];
    let parsed = match rrule::RRule::from_str(&parts.join(";")) {
        Ok(r) => r,
        // An unreadable rule keeps the first occurrence only.
        Err(_) => return single(),
    };
    let parsed = match rule_until {
        Some(u) if u < dtstart.with_timezone(&Utc) => return single(),
        Some(u) => parsed.until(u.with_timezone(&rrule::Tz::UTC)),
        None => parsed,
    };
    let mut set = match parsed.build(dtstart) {
        Ok(s) => s,
        // A rule rrule rejects (invalid BY* parts, …) keeps the first occurrence only.
        Err(_) => return single(),
    };
    for r in &ev.rdates {
        set = set.rdate(to_rrule_dt(r, tz));
    }
    let mut out = Vec::new();
    for t in set.into_iter().take(MAX_PER_SERIES) {
        if t.with_timezone(&Utc) > until {
            break;
        }
        out.push(from_rrule_dt(t, &ev.start));
    }
    out
}

/// Whether an `EXDATE` (or `RECURRENCE-ID`) value names this occurrence. A date names every
/// occurrence on that day.
fn names(value: &Dt, occ: &Dt) -> bool {
    match (match_key(value), match_key(occ)) {
        (Match::Date(d), Match::Instant(_)) => {
            let day = match *occ {
                Dt::Local(t, _) => t.date(),
                other => other.instant(chrono_tz::UTC).date_naive(),
            };
            day == d
        }
        (a, b) => a == b,
    }
}

fn expand_calendar(cal: &Component, ctx: &Ctx, window: (DateTime<Utc>, DateTime<Utc>), out: &mut Vec<Occurrence>) {
    let mut masters: Vec<VEvent> = Vec::new();
    let mut overrides: HashMap<String, Vec<(VEvent, bool)>> = HashMap::new();
    let mut orphans: Vec<VEvent> = Vec::new();
    for c in cal.children.iter().filter(|c| c.name == "VEVENT") {
        let Some(ev) = read_vevent(c, ctx) else { continue };
        match (&ev.recurrence_id, &ev.uid) {
            (Some(_), Some(uid)) => overrides.entry(uid.clone()).or_default().push((ev, false)),
            (Some(_), None) => orphans.push(ev),
            (None, _) => masters.push(ev),
        }
    }

    let (from, to) = window;
    let emit = |ev: &VEvent, start: Dt, out: &mut Vec<Occurrence>| {
        let time = time_of(start, ev.span(ctx.household), ctx.household);
        if time.overlaps(from, to, ctx.household) {
            out.push(Occurrence {
                uid: ev.uid.clone(),
                summary: ev.summary.clone(),
                description: ev.description.clone(),
                location: ev.location.clone(),
                time,
            });
        }
    };

    let mut seen_uids = HashSet::new();
    for master in &masters {
        if let Some(uid) = &master.uid
            && !seen_uids.insert(uid.clone())
        {
            continue; // a repeated master (some exporters duplicate): the first wins
        }
        let mut edits = master.uid.as_ref().and_then(|u| overrides.get_mut(u));
        if master.cancelled {
            // The whole series is cancelled, edited instances included.
            if let Some(list) = edits.as_deref_mut() {
                list.iter_mut().for_each(|(_, used)| *used = true);
            }
            continue;
        }
        let span = master.span(ctx.household);
        let lead = match span {
            Span::Days(n) => Duration::days(n + 1),
            Span::Nominal(d) | Span::Exact(d) => d + Duration::days(1),
        };
        for occ in occurrences(master, to, ctx) {
            // Cheap skip before building anything: ends before the window.
            if occ.instant(ctx.household) + lead < from {
                continue;
            }
            if master.exdates.iter().any(|x| names(x, &occ)) {
                continue;
            }
            let edit = edits.as_deref_mut().and_then(|list| {
                list.iter_mut()
                    .find(|(e, _)| e.recurrence_id.as_ref().is_some_and(|r| names(r, &occ)))
            });
            match edit {
                Some((e, used)) => {
                    *used = true;
                    if !e.cancelled {
                        emit(e, e.start, out);
                    }
                }
                None => emit(master, occ, out),
            }
        }
    }
    // Edited instances whose original slot wasn't generated (moved in from outside the window,
    // or the master is missing from the file) stand on their own.
    for list in overrides.values() {
        for (e, used) in list {
            if !used && !e.cancelled {
                emit(e, e.start, out);
            }
        }
    }
    for e in orphans.iter().filter(|e| !e.cancelled) {
        emit(e, e.start, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_and_reads_quoted_params() {
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Long \r\n title\\, with comma\r\nDTSTART;TZID=\"Europe/Lisbon\":20261106T090000\r\nATTENDEE;CN=\"Doe; Jane\";ROLE=REQ:mailto:j@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let roots = parse_components(text);
        let ev = &roots[0].children[0];
        assert_eq!(unescape(&ev.prop("SUMMARY").unwrap().value), "Long title, with comma");
        assert_eq!(ev.prop("DTSTART").unwrap().param("TZID"), Some("Europe/Lisbon"));
        assert_eq!(ev.prop("ATTENDEE").unwrap().param("CN"), Some("Doe; Jane"));
        assert_eq!(ev.prop("ATTENDEE").unwrap().value, "mailto:j@example.com");
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(parse_duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn zones_by_name() {
        assert_eq!(resolve_tz("Europe/Lisbon"), Some(chrono_tz::Europe::Lisbon));
        assert_eq!(
            resolve_tz("/mozilla.org/20050126_1/Europe/London"),
            Some(chrono_tz::Europe::London)
        );
        assert_eq!(resolve_tz("GMT Standard Time"), Some(chrono_tz::Europe::London));
        assert_eq!(resolve_tz("Customized Time Zone"), None);
    }

    #[test]
    fn gap_times_move_forward_and_ambiguous_take_the_first() {
        let lisbon = chrono_tz::Europe::Lisbon;
        let gap = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(local_to_utc(gap, lisbon).to_rfc3339(), "2026-03-29T01:30:00+00:00");
        let fold = NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(local_to_utc(fold, lisbon).to_rfc3339(), "2026-10-25T00:30:00+00:00");
    }
}
