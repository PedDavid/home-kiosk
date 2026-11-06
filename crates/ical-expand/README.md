# ical-expand

An .ics file (iCalendar, RFC 5545, text) in, the occurrences that overlap a time window out. Recurrences are
expanded, exceptions and edited instances applied, and every time is in UTC.

```rust
let occurrences = ical_expand::expand(&ics_text, from, to, chrono_tz::Europe::Lisbon)?;
for o in occurrences {
    // o.uid, o.summary, o.description, o.location: as in the .ics file
    // o.time: Time::Timed { start, end } in UTC, or Time::AllDay { start, end } (end exclusive)
}
```

Home HQ reads its family calendars through it (`src/calendar/`). This crate covers only
iCalendar. Attributing events to people, reading "Travel: 15" from descriptions and merging the
same invite across calendars are the app's job. Its only dependencies are `chrono`, `chrono-tz`
and [`rrule`](https://crates.io/crates/rrule), which does the recurrence arithmetic.

## Why this isn't a library

We would rather depend on a library than own about 800 lines of iCalendar code. So before
keeping this crate, we ran the candidates over this crate's own fixture table
(`tests/fixtures.rs`). Each fixture is a quirk that real Google, iCloud, Outlook or Nextcloud
.ics files contain. A library had to pass, or come close enough to need only a thin wrapper. None
did. Evaluated on 2026-11-06:

| Fixture case | **this crate** | [`calcard`](https://github.com/stalwartlabs/calcard) 0.3.14 | [`icalendar`](https://github.com/hoodie/icalendar) 0.17.17 (`recurrence` feature) |
|---|---|---|---|
| Weekly and monthly rules, `COUNT`, `EXDATE`, all-day and multi-day | ✅ | ✅ | ✅ |
| Weekly event across both DST changes (local wall time kept) | ✅ | ✅ | ✅ |
| Edited and cancelled instances (`RECURRENCE-ID`), an instance moved in from outside the window | ✅ | ✅, but cancelled instances are returned with their `STATUS` for the caller to filter | ❌ not applied: the caller writes it |
| `UNTIL` written as a local time (common in exports) | ✅ | ✅ | ❌ `rrule` rejects it and the whole series is dropped |
| `TZID` with a vendor prefix (`/mozilla.org/…/Europe/London`) or a Windows name (`GMT Standard Time`) | ✅ | ✅ | ❌ the event is dropped |
| A single event at 01:30 on the autumn fold or in the spring gap | ✅ RFC 5545 §3.3.5 | ❌ dropped (`InvalidDtStart`) | ❌ dropped (`AmbiguousDateTime`) |
| Several open-ended series in one .ics file | ✅ | ❌ a later series silently disappears | ✅ |

**calcard** was the closest. It is Stalwart's iCalendar library, actively maintained, and
reads `VTIMEZONE` definitions, which this crate doesn't. Two things stopped us:

- **The occurrence limit is per calendar, not per series or window.**
  `ICalendar::expand_dates(tz, limit)` expands every series from its `DTSTART` and shares one
  budget. In `dst.ics`, two open-ended weekly series used it all and "New York call" vanished
  without an error. That fits Stalwart, a CalDAV server where each stored object is one series.
  A published .ics file bundles every series into one calendar. Working around it means
  splitting the file per series and expanding each one far past the window.
- **Times on DST edges are rejected.** It converts with chrono's `.single()`, which returns
  nothing both for an ambiguous time (the autumn fold) and for a nonexistent one (the spring
  gap), so the event is dropped.

It also ends a `DTSTART`-only timed event at the end of its day instead of at its start, and
falls back to the caller's zone rather than the file's `X-WR-TIMEZONE`. It would add
`mail-parser`, `mail-builder`, `ahash` and `hashify` as dependencies. Neither of the two
problems above was reported in its issue tracker at the time.

**icalendar** is popular and light, but its recurrence support is a bridge to `rrule`
(`get_recurrence()` builds an `RRuleSet` from `DTSTART`, `RRULE`, `RDATE` and `EXDATE`).
Applying edited instances, accepting real-world `UNTIL` and `TZID` forms, and handling DST
edges would still be ours to write, which is most of this crate. It also reads floating times
and dates in the machine's local zone (`Tz::LOCAL`), which is UTC in a container.

Also looked at, not adopted:

- [`ical`](https://crates.io/crates/ical) 0.11 is a parser only (last release March 2024). It
  would replace about 160 lines of line reading and none of the rest.
- `ical_rust` 0.8 and `eventix` 0.5 claim parsing and expansion, but had about 130 and about
  3,700 downloads. That's too young to trust over the fixture table.

What other ecosystems use for this, Mozilla's `ical.js` (upstream Home HQ's choice) and
Python's `recurring_ical_events`, has no Rust equivalent yet.

### When to revisit

Swap this crate for calcard if it expands per series or within a window, and keeps fold and
gap times as RFC 5545 §3.3.5 says. Its tracker fixes recurrence bugs quickly (#10, #12, #14).
To check, run the fixture table through a thin adapter: `expand` maps onto
`ICalendar::parse(text)?.expand_dates(tz, limit)`. Only this crate would change, because the
app depends on `expand` and `Occurrence` alone.

## What it covers

- Content lines with folding, quoted parameters and text escapes. A line or event that doesn't
  parse is skipped, never the whole file.
- `RRULE` (through `rrule`), `RDATE`, and `EXDATE` in TZID, UTC and DATE forms, comma lists
  included.
- `RECURRENCE-ID` edits and cancellations, including an instance moved into the window from
  outside it, and cancelled series (`STATUS:CANCELLED`).
- All-day and multi-day events (`VALUE=DATE`, `DTEND` or `DURATION`).
- `TZID` as an IANA name, with a vendor prefix, or as a common Windows name. Unknown names fall
  back to the file's `X-WR-TIMEZONE`, then the caller's zone, which is also used for floating
  times.
- DST per RFC 5545 §3.3.5: recurrences keep their local wall time, a gap time moves forward by
  the gap, and an ambiguous time takes the first instant.
- `UNTIL` in UTC, as a local time or as a date, inclusive.

Not covered: `VTIMEZONE` definitions are not interpreted (zones resolve by name),
`RANGE=THISANDFUTURE` counts as a single edited instance, and `EXRULE` is ignored. A rule that
`rrule` can't read keeps only its first occurrence. At most `MAX_PER_SERIES` (20,000)
occurrences are generated per series.

## Tests

```bash
cargo test -p ical-expand
```

`tests/fixtures.rs` is the fixture table over `tests/fixtures/*.ics`. Add a case for every
quirk found in a real .ics file. `cargo test` at the repository root runs it too, because the
crate is a default workspace member.
