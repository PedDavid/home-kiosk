//! CalDAV entries in `HQ_CALENDARS`, and which calendar an event added on the frame goes to.
//!
//! `HQ_CALENDARS` (in the Secret) mixes ICS feeds and CalDAV calendars. A CalDAV entry has
//! `"type": "caldav"`; anything else is an ICS feed and goes to the ICS adapter unchanged:
//!
//! ```json
//! [ { "url": "https://calendar.google.com/calendar/ical/…/basic.ics", "people": ["sam"] },
//!   { "type": "caldav", "id": "alex", "url": "https://dav.example/alex/personal/",
//!     "username": "alex", "password": "…", "people": ["alex"] },
//!   { "type": "caldav", "id": "family", "url": "https://dav.example/alex/family/",
//!     "username": "alex", "password": "…", "people": "all" } ]
//! ```
//!
//! `url` is a calendar collection; an account URL with exactly one event calendar also works.
//! Credentials can also be given in the URL (`https://user:pass@host/…`). `readOnly: true`
//! keeps the frame from writing to a calendar.
//!
//! Which calendar a new event goes to works like upstream's `calendarFor`: the calendar for
//! exactly the event's people (`"all"` when it's for everyone), else the `"all"` one. The
//! settings can name calendars for particular groups (`calendar.writeTo`, by `id`), which win
//! over the calendars' own `people`:
//!
//! ```json
//! "calendar": { "writeTo": [ { "people": ["alex", "sam"], "calendar": "family" } ] }
//! ```

use serde::Deserialize;

use crate::calendar::ics::People;
use crate::settings::WriteRule;

/// One `"type": "caldav"` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalDavConfig {
    /// A name for `calendar.writeTo` in the settings. Not secret.
    #[serde(default)]
    pub id: Option<String>,
    pub url: String,
    #[serde(default)]
    pub people: People,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    /// Position in `HQ_CALENDARS`, from 1, for "Calendar N" on the Status panel.
    #[serde(skip)]
    pub number: usize,
    /// A name for the Status panel instead of "Calendar N" (calendars added on the calendar page).
    #[serde(default)]
    pub label: Option<String>,
}

impl CalDavConfig {
    /// Moves credentials out of the URL (`https://user:pass@host/…`) into the fields, so
    /// requests and errors never carry them in the URL. Percent-escapes in them are decoded.
    pub fn normalise(mut self) -> Self {
        if let Some(scheme_end) = self.url.find("://") {
            let rest = &self.url[scheme_end + 3..];
            let authority_end = rest.find('/').unwrap_or(rest.len());
            if let Some(at) = rest[..authority_end].rfind('@') {
                let userinfo = rest[..at].to_string();
                let (user, pass) = userinfo.split_once(':').unwrap_or((&userinfo, ""));
                if self.username.is_none() {
                    self.username = Some(decode(user));
                    if self.password.is_none() && !pass.is_empty() {
                        self.password = Some(decode(pass));
                    }
                }
                self.url = format!("{}{}", &self.url[..scheme_end + 3], &rest[at + 1..]);
            }
        }
        self.url = self.url.trim().to_string();
        self
    }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `HQ_CALENDARS` split by kind.
#[derive(Debug, Default)]
pub struct Split {
    /// The ICS entries, as JSON for [`crate::calendar::ics::IcsSource::from_env_value`]. `None`
    /// when there are none.
    pub ics: Option<String>,
    pub caldav: Vec<CalDavConfig>,
    /// Why a CalDAV entry couldn't be read (never echoing it: it holds URLs and passwords).
    pub errors: Vec<String>,
}

/// Splits `HQ_CALENDARS` into ICS feeds and CalDAV calendars. A value that isn't a JSON list is
/// passed to the ICS adapter whole, which reports it.
pub fn split(value: Option<&str>) -> Split {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Split::default();
    };
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(value) else {
        return Split {
            ics: Some(value.to_string()),
            ..Default::default()
        };
    };
    let mut out = Split::default();
    let mut ics = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let is_caldav = entry
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.eq_ignore_ascii_case("caldav"));
        if !is_caldav {
            ics.push(entry);
            continue;
        }
        match serde_json::from_value::<CalDavConfig>(entry) {
            Ok(mut c) => {
                c.number = i + 1;
                out.caldav.push(c.normalise());
            }
            Err(_) => out.errors.push(format!(
                r#"Calendar {}: CalDAV entries must look like {{ "type": "caldav", "url": "…", "people": ["id"] | "all", "username": "…", "password": "…" }}"#,
                i + 1
            )),
        }
    }
    if !ics.is_empty() {
        out.ics = Some(serde_json::Value::Array(ics).to_string());
    }
    out
}

/// The ids a `people` value names (`"all"` is kept as is).
pub fn ids(p: &People) -> Vec<String> {
    match p {
        People::None => Vec::new(),
        People::One(s) => vec![s.clone()],
        People::Many(v) => v.clone(),
    }
}

/// A calendar the frame might write to.
#[derive(Debug, Clone)]
pub struct Target<'a> {
    pub id: Option<&'a str>,
    /// Its `people` as configured: ids, or `["all"]`.
    pub people: Vec<String>,
    pub writable: bool,
}

/// Whether `wanted` (as configured: ids, or containing `"all"`) means exactly `people`.
fn matches(wanted: &[String], people: &[String], everyone: bool) -> bool {
    if wanted.is_empty() || wanted.iter().any(|w| w == "all") {
        return everyone;
    }
    wanted.len() == people.len() && people.iter().all(|p| wanted.contains(p))
}

fn is_all(wanted: &[String]) -> bool {
    wanted.is_empty() || wanted.iter().any(|w| w == "all")
}

/// Which of `targets` an event for `people` goes to (upstream's `calendarFor`): exactly these
/// people, by a settings rule and then by a calendar's own `people`; else the `"all"` rule and
/// then the `"all"` calendar. Only writable calendars count. `household` is every person id.
pub fn calendar_for(people: &[String], household: &[String], targets: &[Target], rules: &[WriteRule]) -> Option<usize> {
    let everyone = household.iter().all(|h| people.contains(h));
    let by_id = |id: &str| targets.iter().position(|t| t.writable && t.id.is_some_and(|t| t == id));
    let rule_people: Vec<(Vec<String>, &str)> = rules.iter().map(|r| (ids(&r.people), r.calendar.as_str())).collect();

    let exact_rule = rule_people
        .iter()
        .filter(|(w, _)| !is_all(w) && matches(w, people, everyone))
        .find_map(|(_, id)| by_id(id));
    let exact_cal = || {
        targets
            .iter()
            .position(|t| t.writable && !is_all(&t.people) && matches(&t.people, people, everyone))
    };
    let all_rule = || {
        rule_people
            .iter()
            .filter(|(w, _)| is_all(w))
            .find_map(|(_, id)| by_id(id))
    };
    let all_cal = || targets.iter().position(|t| t.writable && is_all(&t.people));
    exact_rule.or_else(exact_cal).or_else(all_rule).or_else(all_cal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn rule(people: &[&str], calendar: &str) -> WriteRule {
        WriteRule {
            people: People::Many(v(people)),
            calendar: calendar.into(),
        }
    }

    const HOUSE: &[&str] = &["alex", "sam", "maya"];

    fn targets() -> Vec<Target<'static>> {
        vec![
            Target {
                id: Some("alex"),
                people: v(&["alex"]),
                writable: true,
            },
            Target {
                id: Some("sam"),
                people: v(&["sam"]),
                writable: false,
            },
            Target {
                id: Some("family"),
                people: v(&["all"]),
                writable: true,
            },
            Target {
                id: Some("parents"),
                people: v(&["alex", "sam"]),
                writable: true,
            },
        ]
    }

    #[test]
    fn picks_like_upstream_calendar_for() {
        let t = targets();
        let pick = |people: &[&str]| calendar_for(&v(people), &v(HOUSE), &t, &[]);
        assert_eq!(pick(&["alex"]), Some(0), "exactly these people");
        assert_eq!(pick(&["sam", "alex"]), Some(3), "order doesn't matter");
        assert_eq!(
            pick(&["sam"]),
            Some(2),
            "read-only calendars are skipped, falling back to all"
        );
        assert_eq!(pick(&["maya"]), Some(2));
        assert_eq!(pick(&["alex", "sam", "maya"]), Some(2), "everyone");
    }

    #[test]
    fn settings_rules_win() {
        let t = targets();
        let rules = [rule(&["alex", "sam"], "family"), rule(&["maya"], "alex")];
        let pick = |people: &[&str]| calendar_for(&v(people), &v(HOUSE), &t, &rules);
        assert_eq!(pick(&["alex", "sam"]), Some(2));
        assert_eq!(pick(&["maya"]), Some(0));
        assert_eq!(pick(&["alex"]), Some(0));
        // A rule naming a read-only or unknown calendar is ignored.
        let rules = [rule(&["alex"], "sam"), rule(&["alex"], "nope")];
        assert_eq!(calendar_for(&v(&["alex"]), &v(HOUSE), &t, &rules), Some(0));
    }

    #[test]
    fn an_all_rule_replaces_the_all_calendar() {
        let t = targets();
        let rules = [WriteRule {
            people: People::One("all".into()),
            calendar: "parents".into(),
        }];
        assert_eq!(calendar_for(&v(&["maya"]), &v(HOUSE), &t, &rules), Some(3));
        assert_eq!(calendar_for(&v(&["alex"]), &v(HOUSE), &t, &rules), Some(0));
    }

    #[test]
    fn none_when_nothing_fits() {
        let t = vec![Target {
            id: None,
            people: v(&["alex"]),
            writable: true,
        }];
        assert_eq!(calendar_for(&v(&["sam"]), &v(HOUSE), &t, &[]), None);
    }

    #[test]
    fn splits_ics_from_caldav() {
        let s = split(Some(
            r#"[{"url":"https://ics.example/a.ics","people":["sam"]},
                {"type":"caldav","id":"fam","url":"https://bob:p%40ss@dav.example/bob/fam/","people":"all"},
                {"type":"CalDAV","people":"all"}]"#,
        ));
        let ics: serde_json::Value = serde_json::from_str(s.ics.as_deref().unwrap()).unwrap();
        assert_eq!(ics.as_array().unwrap().len(), 1);
        assert_eq!(s.caldav.len(), 1);
        let c = &s.caldav[0];
        assert_eq!(c.url, "https://dav.example/bob/fam/");
        assert_eq!(c.username.as_deref(), Some("bob"));
        assert_eq!(c.password.as_deref(), Some("p@ss"));
        assert_eq!(c.number, 2);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].starts_with("Calendar 3:"));
    }

    #[test]
    fn non_lists_go_to_ics_to_report() {
        assert_eq!(split(Some("{oops")).ics.as_deref(), Some("{oops"));
        assert!(split(Some("  ")).ics.is_none());
        assert!(split(None).caldav.is_empty());
    }
}
