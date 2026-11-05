//! Dummy data in the shapes upstream's Worker returns, so the unmodified frontend renders every
//! screen. Everything is generated relative to "now" in the household's timezone, and writes
//! (shopping, notes, bins, controls) change an in-memory copy so the UI feels alive. Nothing is
//! persisted. Each vertical replaces its part of this module with a real port.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};

use crate::settings::Settings;

pub struct Demo {
    settings: Settings,
    tz: Tz,
    groceries: Vec<Value>,
    notes: Vec<Value>,
    bins_out: Option<Value>,
    events: Vec<Value>,
    devices: BTreeMap<String, Value>,
    climate: Value,
    photos: Vec<Value>,
    frames: BTreeMap<String, Value>,
}

/// Hex colours for placeholder photos and camera snapshots.
const PHOTO_COLOURS: [(&str, &str); 8] = [
    ("#F6A35B", "#C2185B"),
    ("#4FC3F7", "#1A237E"),
    ("#AED581", "#00695C"),
    ("#FFD54F", "#E65100"),
    ("#CE93D8", "#4A148C"),
    ("#80CBC4", "#263238"),
    ("#FFAB91", "#5D4037"),
    ("#90CAF9", "#283593"),
];

impl Demo {
    pub fn new(settings: Settings, now: DateTime<Utc>) -> Self {
        let tz = settings.tz().unwrap_or(chrono_tz::UTC);
        let today = now.with_timezone(&tz).date_naive();
        let people: Vec<String> = settings.people.iter().map(|p| p.id.clone()).collect();
        let first = people.first().cloned().unwrap_or_else(|| "all".into());
        let last = people.last().cloned().unwrap_or_else(|| "all".into());
        let ms = now.timestamp_millis();

        let cats = settings.categories();
        let cat = |i: usize| cats.get(i).cloned().unwrap_or_else(|| "Other".into());
        let store = settings.stores()[0].id.clone();
        let groceries = [("Milk", 3, 2), ("Bananas", 0, 1), ("Bread", 1, 1), ("Coffee beans", 4, 1), ("Spinach", 0, 1), ("Eggs", 3, 12)]
            .iter()
            .enumerate()
            .map(|(i, (name, c, qty))| {
                json!({ "id": format!("demo-g{i}"), "name": name, "cat": cat(*c), "qty": qty, "store": store, "note": "", "done": i == 4 })
            })
            .collect();

        let notes = vec![
            json!({ "id": "demonote1", "text": "Plumber coming Thursday between 9 and 11", "from": first, "to": "all", "created": ms - 3_600_000, "expires": null }),
            json!({ "id": "demonote2", "text": "Parcel is with the neighbour at no. 12", "from": last, "to": first, "created": ms - 600_000, "expires": null }),
        ];

        let photos = (0..PHOTO_COLOURS.len())
            .map(|i| {
                let date = today
                    - Duration::days(match i {
                        0 => 365,
                        1 => 730,
                        _ => 40 * i as i64 + 3,
                    });
                json!({
                    "id": format!("demo{i}"), "o": if i % 3 == 2 { "p" } else { "l" }, "date": date.to_string(),
                    "place": PLACES[i],
                    "from": people.get(i % people.len().max(1)).cloned(), "fav": i % 4 == 0, "tag": date.to_string(),
                })
            })
            .collect();

        let mut devices = BTreeMap::new();
        for d in [
            json!({ "id": "light.kitchen", "room": "kitchen", "name": "Pendants", "offline": false, "type": "light", "on": true, "level": 80, "dims": true }),
            json!({ "id": "light.kitchen_counter", "room": "kitchen", "name": "Counter", "offline": false, "type": "light", "on": false, "level": 100, "dims": false }),
            json!({ "id": "cover.kitchen_blind", "room": "kitchen", "name": "Blind", "offline": false, "type": "blind", "pos": 60 }),
            json!({ "id": "light.living_room", "room": "living", "name": "Ceiling", "offline": false, "type": "light", "on": true, "level": 45, "dims": true }),
            json!({ "id": "light.living_lamp", "room": "living", "name": "Lamp", "offline": false, "type": "light", "on": false, "level": 100, "dims": true }),
            json!({ "id": "switch.tv_backlight", "room": "living", "name": "TV backlight", "offline": false, "type": "light", "on": false, "level": 100, "dims": false }),
            json!({ "id": "lock.front_door", "room": "entry", "name": "Front door", "offline": false, "type": "lock", "locked": true }),
            json!({ "id": "light.porch", "room": "entry", "name": "Porch", "offline": false, "type": "light", "on": false, "level": 100, "dims": false }),
        ] {
            devices.insert(d["id"].as_str().unwrap().to_string(), d);
        }

        let mut demo = Demo {
            tz,
            groceries,
            notes,
            bins_out: None,
            events: Vec::new(),
            devices,
            climate: json!({ "current": 20.5, "set": 21.0, "mode": "heat", "modes": ["off", "heat", "auto"], "humidity": 54, "step": 0.5, "min": 7, "max": 30 }),
            photos,
            frames: BTreeMap::new(),
            settings,
        };
        demo.events = demo.make_events(today, &people);
        demo
    }

    fn at(&self, day: NaiveDate, h: u32, m: u32) -> String {
        let t = NaiveTime::from_hms_opt(h, m, 0).unwrap();
        self.tz
            .from_local_datetime(&day.and_time(t))
            .earliest()
            .map(|d| {
                d.with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            })
            .unwrap_or_default()
    }

    fn make_events(&self, today: NaiveDate, people: &[String]) -> Vec<Value> {
        let p = |i: usize| {
            people
                .get(i % people.len().max(1))
                .cloned()
                .unwrap_or_else(|| "all".into())
        };
        let everyone: Vec<String> = people.to_vec();
        let mut out = Vec::new();
        let mut n = 0;
        let mut timed = |out: &mut Vec<Value>,
                         day: NaiveDate,
                         (h, m): (u32, u32),
                         mins: i64,
                         title: &str,
                         who: Vec<String>,
                         extra: Value| {
            let start = self.at(day, h, m);
            let end_naive = day.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()) + Duration::minutes(mins);
            let end = self.at(end_naive.date(), end_naive.hour(), end_naive.minute());
            let mut ev =
                json!({ "id": format!("demo-e{n}"), "title": title, "people": who, "start": start, "end": end });
            if let (Some(obj), Some(more)) = (ev.as_object_mut(), extra.as_object()) {
                obj.extend(more.clone());
            }
            n += 1;
            out.push(ev);
        };

        for offset in -21i64..=45 {
            let day = today + Duration::days(offset);
            let dow = day.weekday().num_days_from_monday();
            if dow < 5 {
                timed(&mut out, day, (9, 0), 30, "Stand-up", vec![p(0)], json!({}));
            }
            if dow == 1 || dow == 3 {
                timed(
                    &mut out,
                    day,
                    (19, 0),
                    60,
                    "Gym",
                    vec![p(1)],
                    json!({ "loc": "Fitness Hub, Av. da Liberdade", "travel": 15 }),
                );
            }
            if dow == 5 {
                timed(
                    &mut out,
                    day,
                    (10, 0),
                    90,
                    "Farmers' market",
                    everyone.clone(),
                    json!({ "loc": "Mercado de Campo de Ourique" }),
                );
            }
            if dow == 6 && offset % 14 == 0 {
                timed(
                    &mut out,
                    day,
                    (13, 0),
                    120,
                    "Lunch with family",
                    everyone.clone(),
                    json!({ "loc": "Sintra", "travel": 35 }),
                );
            }
        }
        // Today: enough to fill "next up" whatever time it is.
        timed(
            &mut out,
            today,
            (8, 15),
            30,
            "Coffee with Rita",
            vec![p(1)],
            json!({ "loc": "Café Lisboa" }),
        );
        timed(
            &mut out,
            today,
            (12, 30),
            60,
            "Dentist",
            vec![p(0)],
            json!({ "loc": "Clínica Dentária, Rua Augusta", "travel": 20, "notes": "Bring the insurance card" }),
        );
        timed(
            &mut out,
            today,
            (17, 30),
            45,
            "Pick up dry cleaning",
            vec![p(1)],
            json!({}),
        );
        timed(
            &mut out,
            today,
            (20, 0),
            120,
            "Dinner with friends",
            everyone.clone(),
            json!({ "loc": "Bairro Alto", "travel": 25 }),
        );
        timed(
            &mut out,
            today,
            (22, 30),
            30,
            "Call grandma",
            everyone.clone(),
            json!({}),
        );
        timed(
            &mut out,
            today + Duration::days(1),
            (11, 0),
            60,
            "Car service",
            vec![p(0)],
            json!({ "loc": "Garage, Rua do Sol" }),
        );
        timed(
            &mut out,
            today + Duration::days(2),
            (18, 30),
            90,
            "Parents' evening",
            everyone.clone(),
            json!({}),
        );

        let all_day = |id: &str, title: &str, from: NaiveDate, days: i64, who: Vec<String>| json!({ "id": id, "title": title, "people": who, "allDay": true, "start": from.to_string(), "end": (from + Duration::days(days)).to_string() });
        out.push(all_day("demo-ad1", "Recycling week", today, 1, everyone.clone()));
        out.push(all_day(
            "demo-ad2",
            "Conference",
            today + Duration::days(3),
            2,
            vec![p(0)],
        ));
        out.push(all_day("demo-ad3", "Holiday", today + Duration::days(18), 7, everyone));
        out
    }

    fn today(&self, now: DateTime<Utc>) -> NaiveDate {
        now.with_timezone(&self.tz).date_naive()
    }

    // ---------------------------------------------------------------- state

    pub fn state(&self, now: DateTime<Utc>) -> Value {
        let catalog: Vec<Value> = self
            .settings
            .catalog()
            .into_iter()
            .map(|(n, c)| json!([n, c]))
            .collect();
        json!({
            "people": self.settings.people(true),
            "stores": self.settings.stores(),
            "catOrder": self.settings.cat_order(),
            "catalog": catalog,
            "usuals": self.settings.shopping.usuals,
            "groceries": self.groceries,
            "photos": self.photos,
            "hasHome": true,
            "scenes": scenes(),
            "cameras": cameras(),
            "bins": self.bins(now),
            "notes": self.live_notes(now),
            "awayOffMinutes": self.settings.display.away_off_minutes,
        })
    }

    /// Dummy rotation: something goes out tomorrow (so the 4 PM highlight shows in the evening),
    /// then the types take turns weekly.
    fn bins(&self, now: DateTime<Utc>) -> Value {
        let Some(b) = &self.settings.bins else {
            return Value::Null;
        };
        let types: Vec<&String> = b.types.keys().collect();
        if types.is_empty() {
            return Value::Null;
        }
        let tomorrow = self.today(now) + Duration::days(1);
        let next: Vec<Value> = (0..4)
            .map(|i| {
                let mut bins = vec![types[0].clone()];
                if types.len() > 1 {
                    bins.push(types[1 + i % (types.len() - 1)].clone());
                }
                json!({ "date": (tomorrow + Duration::days(7 * i as i64)).to_string(), "bins": bins })
            })
            .collect();
        json!({ "types": b.types, "next": next, "out": self.bins_out })
    }

    pub fn set_bins_out(&mut self, date: &str, out: bool, now: DateTime<Utc>) -> Value {
        self.bins_out = out.then(|| json!({ "date": date, "at": now.timestamp_millis() }));
        self.bins(now)
    }

    // ---------------------------------------------------------------- notes

    pub fn live_notes(&self, now: DateTime<Utc>) -> Vec<Value> {
        let ms = now.timestamp_millis();
        let mut v: Vec<Value> = self
            .notes
            .iter()
            .filter(|n| n.get("cleared").is_none_or(Value::is_null))
            .filter(|n| n["expires"].as_i64().is_none_or(|e| e > ms))
            .map(|n| {
                let mut n = n.clone();
                n.as_object_mut().unwrap().remove("cleared");
                n
            })
            .collect();
        v.sort_by_key(|n| std::cmp::Reverse(n["created"].as_i64().unwrap_or(0)));
        v.truncate(20);
        v
    }

    pub fn put_note(&mut self, b: &Value, now: DateTime<Utc>) -> Result<Vec<Value>, &'static str> {
        let id = b["id"].as_str().unwrap_or("");
        let text: String = b["text"].as_str().unwrap_or("").trim().chars().take(140).collect();
        let from = b["from"].as_str().unwrap_or("");
        let is_person = |id: &str| self.settings.people.iter().any(|p| p.id == id);
        let valid_id = (6..=40).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !valid_id || text.is_empty() || !is_person(from) {
            return Err("id, text and from (a person) required");
        }
        let to = b["to"]
            .as_str()
            .filter(|t| *t == "all" || is_person(t))
            .unwrap_or("all");
        let ms = now.timestamp_millis();
        let expires = b["expires"].as_i64().filter(|e| *e > ms);
        let note = json!({ "id": id, "text": text, "from": from, "to": to, "created": ms, "expires": expires });
        match self.notes.iter_mut().find(|n| n["id"] == id) {
            Some(n) => {
                n["text"] = note["text"].clone();
                n["to"] = note["to"].clone();
                n["expires"] = note["expires"].clone();
                n["cleared"] = Value::Null;
            }
            None => self.notes.push(note),
        }
        Ok(self.live_notes(now))
    }

    pub fn clear_note(&mut self, id: &str, undo: bool, now: DateTime<Utc>) -> Vec<Value> {
        if let Some(n) = self.notes.iter_mut().find(|n| n["id"] == id) {
            n["cleared"] = if undo {
                Value::Null
            } else {
                now.timestamp_millis().into()
            };
        }
        self.live_notes(now)
    }

    // ---------------------------------------------------------------- groceries

    pub fn put_grocery(&mut self, b: &Value) -> Result<(), &'static str> {
        let id: String = b["id"].as_str().unwrap_or("").trim().chars().take(64).collect();
        let name: String = b["name"].as_str().unwrap_or("").trim().chars().take(80).collect();
        if id.is_empty() || name.is_empty() {
            return Err("id and name required");
        }
        let stores = self.settings.stores();
        let store = b["store"]
            .as_str()
            .filter(|s| stores.iter().any(|x| x.id == *s))
            .unwrap_or(&stores[0].id);
        let qty = b["qty"]
            .as_i64()
            .or_else(|| b["qty"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(1)
            .clamp(1, 99);
        let cat = b["cat"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Other");
        let item = json!({
            "id": id, "name": name, "cat": cat, "qty": qty, "store": store,
            "note": b["note"].as_str().unwrap_or("").trim(), "done": b["done"].as_bool().unwrap_or(false),
        });
        // Upstream orders by last update, so a changed item moves to the end.
        self.groceries.retain(|g| g["id"] != item["id"]);
        self.groceries.push(item);
        Ok(())
    }

    pub fn clear_groceries(&mut self, ids: &[String], restore: bool) {
        if restore {
            return; // cleared items are dropped in the demo; nothing to bring back
        }
        self.groceries.retain(|g| !ids.iter().any(|id| g["id"] == id.as_str()));
    }

    // ---------------------------------------------------------------- calendar

    pub fn calendar(&self, now: DateTime<Utc>) -> Value {
        json!({ "events": self.events, "errors": [], "checked": now.timestamp_millis() })
    }

    pub fn add_event(&mut self, b: &Value) -> Result<(), &'static str> {
        let id = b["id"].as_str().unwrap_or("");
        let title = b["title"].as_str().unwrap_or("").trim();
        let (start, end) = (b["start"].as_i64().unwrap_or(0), b["end"].as_i64().unwrap_or(0));
        let people: Vec<&str> = b["people"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| self.settings.people.iter().any(|p| p.id == *id))
            .collect();
        if id.is_empty() || title.is_empty() || end <= start || people.is_empty() {
            return Err("id, title, people, start < end required");
        }
        let iso = |ms: i64| {
            DateTime::from_timestamp_millis(ms)
                .unwrap_or_default()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        self.events.retain(|e| e["id"] != id);
        self.events.push(json!({ "id": id, "title": title, "people": people, "start": iso(start), "end": iso(end), "local": true, "google": false }));
        Ok(())
    }

    pub fn delete_event(&mut self, id: &str) {
        self.events.retain(|e| e["id"] != id);
    }

    // ---------------------------------------------------------------- weather

    pub fn weather(&self, now: DateTime<Utc>) -> Value {
        let celsius = self.settings.location.as_ref().and_then(|l| l.units.as_deref()) != Some("fahrenheit");
        let base = if celsius { 0.0 } else { 32.0 };
        let k = if celsius { 1.0 } else { 1.8 };
        let temp = |h: usize| {
            let hour = (h % 24) as f64;
            let c = 15.0 + 6.0 * ((hour - 9.0) / 24.0 * std::f64::consts::TAU).sin() - if h >= 24 { 1.0 } else { 0.0 };
            (base + c * k).round() as i64
        };
        let t: Vec<i64> = (0..48).map(temp).collect();
        let p: Vec<i64> = (0..48)
            .map(|h| {
                if (30..38).contains(&h) {
                    40 + (h as i64 - 30) * 5
                } else {
                    (h as i64 % 5) * 2
                }
            })
            .collect();
        let c: Vec<&str> = (0..48)
            .map(|h| {
                let hour = h % 24;
                let day = (7..20).contains(&hour);
                match h {
                    _ if (30..38).contains(&h) => "rain",
                    _ if (13..17).contains(&h) => "partly",
                    _ if day => "sun",
                    _ => "moon",
                }
            })
            .collect();
        let local_now = now.with_timezone(&self.tz);
        let hour = local_now.hour() as usize;
        json!({
            "day": local_now.date_naive().to_string(),
            "units": if celsius { "celsius" } else { "fahrenheit" },
            "hi": t[..24].iter().max(), "lo": t[..24].iter().min(),
            "now": t[hour], "feels": t[hour] - 1, "humidity": 62, "wind": 11,
            "t": t, "p": p, "c": c,
            "warnings": [],
        })
    }

    // ---------------------------------------------------------------- home

    pub fn home(&self, now: DateTime<Utc>) -> Value {
        let hour = now.with_timezone(&self.tz).hour();
        let presence: Vec<Value> = self
            .settings
            .people
            .iter()
            .filter(|p| p.ha.is_some())
            .enumerate()
            .map(|(i, p)| {
                let home = i == 0 || !(9..18).contains(&hour);
                json!({ "id": p.id, "home": home, "status": if home { "Home" } else { "Work" } })
            })
            .collect();
        let devices: Vec<&Value> = self.devices.values().collect();
        let mut devices = devices;
        devices.sort_by_key(|d| ROOM_ORDER.iter().position(|r| d["room"] == *r).unwrap_or(99));
        json!({
            "online": true,
            "rooms": [{ "id": "kitchen", "name": "Kitchen" }, { "id": "living", "name": "Living room" }, { "id": "entry", "name": "Entry" }],
            "devices": devices,
            "scenes": scenes(),
            "cameras": cameras(),
            "climate": self.climate,
            "presence": presence,
            "sensors": { "doorbell": null, "leak": false, "smoke": [], "garageOpenSince": null },
            "energy": { "cost": 1.84, "currency": "€", "watts": 640, "high": false },
            "players": [{
                "id": "media_player.living_room", "name": "Living room speaker", "state": "paused",
                "title": "Demo track", "artist": "Demo artist", "app": "Music", "loaded": true,
                "volume": 35, "muted": false, "art": null, "features": 0,
            }],
            "checks": { "leaksOffline": [], "leakTotal": 2, "smokeOffline": [], "smokeTotal": 2, "valveClosed": null, "lowBattery": [] },
            "via": "demo", "viaAt": now.timestamp_millis(),
            "alertConfig": { "doorbell": false, "doorbellCamera": null, "doorbellLock": null, "waterValve": false, "smoke": true, "garageOpenMinutes": 0 },
        })
    }

    pub fn energy(&self, now: DateTime<Utc>) -> Value {
        let local = now.with_timezone(&self.tz);
        let midnight = self
            .tz
            .from_local_datetime(&local.date_naive().and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        let hours: Vec<f64> = (0..=local.hour())
            .map(|h| match h {
                7..=8 => 0.9,
                12..=13 => 0.7,
                18..=21 => 1.2,
                _ => 0.25,
            })
            .collect();
        let total: f64 = hours.iter().sum();
        json!({
            "midnight": midnight, "now": now.timestamp_millis(), "hours": hours,
            "total": (total * 100.0).round() / 100.0, "yesterdayByNow": (total * 105.0).round() / 100.0,
            "nowW": 640, "peak": { "w": 3120, "at": midnight + 19 * 3_600_000 },
            "cost": (total * 0.16 * 100.0).round() / 100.0, "meterReported": null,
        })
    }

    pub fn control_device(&mut self, id: &str, b: &Value) -> Result<(), &'static str> {
        let d = self.devices.get_mut(id).ok_or("unknown device")?;
        if let Some(locked) = b["locked"].as_bool() {
            d["locked"] = locked.into();
        } else if let Some(pos) = b["pos"].as_f64() {
            d["pos"] = (pos.clamp(0.0, 100.0).round() as i64).into();
        } else if let Some(state) = b["state"].as_str() {
            d["pos"] = if state == "open" { 100 } else { 0 }.into();
        } else if let Some(on) = b["on"].as_bool() {
            d["on"] = on.into();
            if let Some(level) = b["level"].as_f64() {
                d["level"] = (level.clamp(1.0, 100.0).round() as i64).into();
            }
        } else {
            return Err("nothing to change");
        }
        Ok(())
    }

    pub fn home_action(&mut self, what: &str, b: &Value) -> Result<(), &'static str> {
        match what {
            "climate" => {
                if let Some(set) = b["set"].as_f64() {
                    self.climate["set"] = set.into();
                }
                if let Some(mode) = b["mode"].as_str() {
                    self.climate["mode"] = mode.into();
                }
                Ok(())
            }
            "scene" if scenes().iter().any(|s| s["id"] == b["id"]) => Ok(()),
            _ => Err("unknown action"),
        }
    }

    // ---------------------------------------------------------------- photos

    pub fn photo_svg(&self, id: &str, small: bool) -> Option<String> {
        let i: usize = id.strip_prefix("demo")?.parse().ok()?;
        let p = self.photos.get(i)?;
        let (w, h) = if p["o"] == "p" { (1080, 1440) } else { (1920, 1280) };
        let (w, h) = if small { (w / 2, h / 2) } else { (w, h) };
        Some(placeholder_svg(
            w,
            h,
            PHOTO_COLOURS[i % PHOTO_COLOURS.len()],
            &format!("Demo photo {}", i + 1),
        ))
    }

    pub fn set_favourite(&mut self, id: &str, fav: bool) {
        if let Some(p) = self.photos.iter_mut().find(|p| p["id"] == id) {
            p["fav"] = fav.into();
        }
    }

    // ---------------------------------------------------------------- frames

    pub fn frame_beat(&mut self, b: &Value, now: DateTime<Utc>) -> Result<(), &'static str> {
        let id = b["id"].as_str().unwrap_or("");
        if !(6..=40).contains(&id.len()) || !id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return Err("id required");
        }
        let s = |k: &str, max: usize| -> String { b[k].as_str().unwrap_or("").trim().chars().take(max).collect() };
        let name = Some(s("name", 40))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Frame".into());
        self.frames.insert(
            id.to_string(),
            json!({
                "id": id, "name": name, "version": s("version", 20), "build": s("build", 40), "kiosk": s("kiosk", 20),
                "rssi": b["rssi"].as_f64().map(|r| r.round() as i64), "model": s("model", 60), "android": s("android", 10),
                "loaded": b["loaded"].as_f64(), "seen": now.timestamp_millis(),
            }),
        );
        Ok(())
    }

    pub fn frames(&self) -> Vec<Value> {
        let mut v: Vec<Value> = self.frames.values().cloned().collect();
        v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        v
    }

    pub fn forget_frame(&mut self, id: &str) {
        self.frames.remove(id);
    }
}

const PLACES: [&str; 8] = ["Lisbon", "Sintra", "Porto", "", "Algarve", "", "Évora", "Cascais"];

const ROOM_ORDER: [&str; 3] = ["kitchen", "living", "entry"];

fn scenes() -> Vec<Value> {
    vec![
        json!({ "id": "scene.movie_night", "name": "Movie night", "icon": "film" }),
        json!({ "id": "scene.good_night", "name": "Good night", "icon": "moon" }),
        json!({ "id": "scene.all_off", "name": "All off", "icon": "bolt" }),
    ]
}

fn cameras() -> Vec<Value> {
    vec![
        json!({ "id": "camera.front_door", "name": "Front door" }),
        json!({ "id": "camera.garden", "name": "Garden" }),
    ]
}

pub fn camera_svg(id: &str, now: DateTime<Utc>) -> Option<String> {
    let (name, colours) = match id {
        "camera.front_door" => ("Front door", ("#607D8B", "#263238")),
        "camera.garden" => ("Garden", ("#8BC34A", "#1B5E20")),
        _ => return None,
    };
    Some(placeholder_svg(
        1280,
        720,
        colours,
        &format!("{name} · {}", now.format("%H:%M:%S UTC")),
    ))
}

fn placeholder_svg(w: u32, h: u32, (a, b): (&str, &str), label: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{a}"/><stop offset="1" stop-color="{b}"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="{cx}" cy="{cy}" r="{r}" fill="#ffffff" fill-opacity="0.18"/><text x="50%" y="92%" text-anchor="middle" font-family="sans-serif" font-size="{fs}" fill="#ffffff" fill-opacity="0.75">{label}</text></svg>"##,
        cx = w * 7 / 10,
        cy = h * 3 / 10,
        r = h / 6,
        fs = h / 24,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{EXAMPLE, Settings};

    fn demo(now: DateTime<Utc>) -> Demo {
        Demo::new(Settings::parse(EXAMPLE).unwrap(), now)
    }

    #[test]
    fn has_events_today_in_local_time() {
        let now = Utc.with_ymd_and_hms(2026, 11, 5, 10, 0, 0).unwrap();
        let cal = demo(now).calendar(now);
        let today: Vec<_> = cal["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["start"].as_str().unwrap().starts_with("2026-11-05"))
            .collect();
        assert!(today.len() >= 4);
        // Lisbon is UTC+0 in November: the 12:30 dentist starts at 12:30Z.
        assert!(
            today
                .iter()
                .any(|e| e["title"] == "Dentist" && e["start"] == "2026-11-05T12:30:00.000Z")
        );
    }

    #[test]
    fn bins_go_out_tomorrow_and_can_be_marked_out() {
        let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 0, 0).unwrap();
        let mut d = demo(now);
        let s = d.state(now);
        assert_eq!(s["bins"]["next"][0]["date"], "2026-11-06");
        assert!(s["bins"]["out"].is_null());
        let b = d.set_bins_out("2026-11-06", true, now);
        assert_eq!(b["out"]["date"], "2026-11-06");
        assert!(d.set_bins_out("2026-11-06", false, now)["out"].is_null());
    }

    #[test]
    fn notes_can_be_added_cleared_and_restored() {
        let now = Utc::now();
        let mut d = demo(now);
        let before = d.live_notes(now).len();
        let notes = d
            .put_note(
                &json!({ "id": "abc123", "text": "hello", "from": "alex", "to": "sam" }),
                now,
            )
            .unwrap();
        assert_eq!(notes.len(), before + 1);
        assert!(
            d.put_note(&json!({ "id": "abc124", "text": "x", "from": "nobody" }), now)
                .is_err()
        );
        assert_eq!(d.clear_note("abc123", false, now).len(), before);
        assert_eq!(d.clear_note("abc123", true, now).len(), before + 1);
    }

    #[test]
    fn groceries_update_in_place() {
        let mut d = demo(Utc::now());
        d.put_grocery(&json!({ "id": "x1", "name": "Lemons", "qty": 200 }))
            .unwrap();
        d.put_grocery(&json!({ "id": "x1", "name": "Lemons", "qty": 2, "done": true }))
            .unwrap();
        let s = d.state(Utc::now());
        let mine: Vec<_> = s["groceries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| g["id"] == "x1")
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0]["qty"], 2);
        assert_eq!(mine[0]["done"], true);
        d.clear_groceries(&["x1".into()], false);
        assert!(
            !d.state(Utc::now())["groceries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["id"] == "x1")
        );
    }

    #[test]
    fn weather_has_48_hours() {
        let now = Utc::now();
        let w = demo(now).weather(now);
        assert_eq!(w["t"].as_array().unwrap().len(), 48);
        assert_eq!(w["c"].as_array().unwrap().len(), 48);
    }
}
