//! Household settings, in upstream Home HQ's `settings.json` format.
//!
//! Only the parts this delivery uses are typed; unknown sections are ignored so the same file can
//! grow as integrations land.

use std::collections::BTreeMap;

use anyhow::Context;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// The settings bundled with the binary, used when `HQ_SETTINGS` is not set.
pub const EXAMPLE: &str = include_str!("../config/settings.example.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default = "default_tz")]
    pub timezone: String,
    pub location: Option<Location>,
    #[serde(default)]
    pub people: Vec<PersonConfig>,
    pub bins: Option<Bins>,
    #[serde(default)]
    pub shopping: Shopping,
    #[serde(default)]
    pub display: Display,
    #[serde(default)]
    pub calendar: CalendarSettings,
}

fn default_tz() -> String {
    "UTC".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub units: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PersonConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub ha: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Bins {
    #[serde(default)]
    pub types: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shopping {
    #[serde(default)]
    pub stores: Vec<Store>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub store_order: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub usuals: Vec<String>,
    /// Category -> item names. Order of categories follows the file.
    #[serde(default)]
    pub catalog: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    #[serde(default)]
    pub away_off_minutes: u32,
}

/// `calendar`: which writable calendar events added on the frame go to (CalDAV).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSettings {
    /// Checked before the calendars' own `people`; see `calendar::caldav::config`.
    #[serde(default)]
    pub write_to: Vec<WriteRule>,
}

/// Events for exactly `people` (or `"all"`) go to the `HQ_CALENDARS` entry with this `id`.
#[derive(Debug, Clone, Deserialize)]
pub struct WriteRule {
    pub people: crate::calendar::ics::People,
    pub calendar: String,
}

/// A person as the frontend sees it (`/api/state` → `people`).
#[derive(Debug, Clone, Serialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub color: String,
    pub rgb: String,
    pub ink: String,
    pub tracked: bool,
}

impl Settings {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let s: Settings = serde_json::from_str(text).context("settings are not valid")?;
        s.tz()?;
        Ok(s)
    }

    pub fn load(path: Option<&str>) -> anyhow::Result<Self> {
        match path {
            Some(p) => {
                let text = std::fs::read_to_string(p).with_context(|| format!("reading {p}"))?;
                Self::parse(&text).with_context(|| format!("in {p}"))
            }
            None => Self::parse(EXAMPLE),
        }
    }

    pub fn tz(&self) -> anyhow::Result<Tz> {
        self.timezone
            .parse()
            .map_err(|_| anyhow::anyhow!("unknown timezone {:?}", self.timezone))
    }

    /// People with derived colours, as upstream's Worker reports them.
    pub fn people(&self, home_tracking: bool) -> Vec<Person> {
        self.people
            .iter()
            .map(|p| {
                let color = p.color.clone().unwrap_or_else(|| "#888888".into());
                let (r, g, b) = hex_rgb(&color);
                let light = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64) / 255.0 > 0.62;
                Person {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    rgb: format!("{r} {g} {b}"),
                    ink: if light { "#111111" } else { "#FFFFFF" }.into(),
                    color,
                    tracked: home_tracking && p.ha.is_some(),
                }
            })
            .collect()
    }

    pub fn stores(&self) -> Vec<Store> {
        if self.shopping.stores.is_empty() {
            vec![Store {
                id: "grocer".into(),
                name: "Groceries".into(),
            }]
        } else {
            self.shopping.stores.clone()
        }
    }

    pub fn categories(&self) -> Vec<String> {
        if self.shopping.categories.is_empty() {
            vec!["Other".into()]
        } else {
            self.shopping.categories.clone()
        }
    }

    /// `{ all: [...categories], <store>: [...categories in that store's aisle order] }`
    pub fn cat_order(&self) -> serde_json::Value {
        let cats = self.categories();
        let mut m = serde_json::Map::new();
        m.insert("all".into(), cats.clone().into());
        for s in self.stores() {
            let order = self
                .shopping
                .store_order
                .get(&s.id)
                .cloned()
                .unwrap_or_else(|| cats.clone());
            m.insert(s.id, order.into());
        }
        m.into()
    }

    /// `[[name, category], ...]`
    pub fn catalog(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (cat, items) in &self.shopping.catalog {
            for name in items.as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                out.push((name.to_string(), cat.clone()));
            }
        }
        out
    }
}

fn hex_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    let full: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    let n = u32::from_str_radix(&full, 16).unwrap_or(0x888888);
    ((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_settings_parse() {
        let s = Settings::parse(EXAMPLE).unwrap();
        assert_eq!(s.tz().unwrap(), chrono_tz::Europe::Lisbon);
        assert!(!s.people.is_empty());
    }

    #[test]
    fn people_get_ink_for_their_colour() {
        let s = Settings::parse(
            r##"{"people":[{"id":"a","name":"A","color":"#FFB000"},{"id":"b","name":"B","color":"#123"}]}"##,
        )
        .unwrap();
        let p = s.people(false);
        assert_eq!(p[0].rgb, "255 176 0");
        assert_eq!(p[0].ink, "#111111");
        assert_eq!(p[1].rgb, "17 34 51");
        assert_eq!(p[1].ink, "#FFFFFF");
    }

    #[test]
    fn rejects_unknown_timezone() {
        assert!(Settings::parse(r#"{"timezone":"Mars/Olympus"}"#).is_err());
    }
}
