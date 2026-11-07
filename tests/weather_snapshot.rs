//! A real Open-Meteo response, stored as a snapshot, run through the mapping.
//!
//! `tests/fixtures/weather/open-meteo-snapshot.json` holds the request this backend makes for the
//! bundled example settings and the response Open-Meteo gave. This test normally reads it and
//! makes no network calls. Set `HQ_RECORD_WEATHER=1` to fetch a fresh one from the live API and
//! overwrite it. CI does that when the file is missing, or on a manual run with
//! "record-weather-snapshot" ticked, and commits the result.
//!
//! When the request changes (fields, units, the example location), the stored one no longer
//! matches and this test fails until the snapshot is recorded again.

use std::{path::PathBuf, sync::Arc};

use chrono::Utc;
use home_hq::{
    settings::{EXAMPLE, Settings},
    tls,
    weather::{self, Fetch, Weather},
};
use serde_json::{Value, json};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/weather/open-meteo-snapshot.json")
}

fn weather_with(fetch: Fetch) -> Weather {
    let settings = Settings::parse(EXAMPLE).unwrap();
    Weather::new(&settings, weather::OPEN_METEO_URL, fetch, Arc::new(Utc::now)).unwrap()
}

/// The live request for the example settings, written as `{ request, recorded, response }`.
async fn record() -> Value {
    tls::install_crypto_provider();
    let fetch = weather::http_fetch().unwrap();
    let url = weather_with(fetch.clone()).request_url().unwrap().to_string();
    let body = fetch(url.clone())
        .await
        .unwrap_or_else(|e| panic!("recording the weather snapshot from {url}: {e:#}"));
    let response: Value = serde_json::from_str(&body).expect("Open-Meteo answered with JSON");
    let snapshot = json!({
        "request": url,
        "recorded": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "response": response,
    });
    std::fs::write(path(), serde_json::to_string_pretty(&snapshot).unwrap() + "\n").unwrap();
    eprintln!("recorded {}", path().display());
    snapshot
}

#[tokio::test]
async fn real_open_meteo_response_maps_cleanly() {
    let recording = std::env::var("HQ_RECORD_WEATHER").is_ok_and(|v| v == "1");
    let snapshot: Value = if recording {
        record().await
    } else {
        let text = std::fs::read_to_string(path()).unwrap_or_else(|_| {
            panic!(
                "{} is missing. Record it with HQ_RECORD_WEATHER=1 cargo test --test weather_snapshot \
                 (needs network), or let CI record it (the weather-snapshot job)",
                path().display()
            )
        });
        serde_json::from_str(&text).unwrap()
    };

    // The snapshot answers the request this build makes; otherwise it needs recording again.
    let body = snapshot["response"].to_string();
    let fetch: Fetch = Arc::new(move |_| {
        let body = body.clone();
        Box::pin(async move { Ok(body) })
    });
    let w = weather_with(fetch);
    assert_eq!(
        snapshot["request"].as_str(),
        w.request_url(),
        "the weather request changed: record the snapshot again (HQ_RECORD_WEATHER=1, or CI's record-weather-snapshot)"
    );

    let v = w.get().await;
    assert!(v.get("error").is_none(), "{v}");
    let raw = &snapshot["response"];
    let hours = raw["hourly"]["time"].as_array().unwrap();
    assert_eq!(hours.len(), 48, "2 days of hours");
    assert_eq!(v["day"].as_str(), hours[0].as_str().map(|t| &t[..10]));
    assert!(
        hours[0].as_str().unwrap().ends_with("T00:00"),
        "index 0 is local midnight"
    );
    assert_eq!(v["units"], "celsius");
    assert_eq!(v["warnings"], json!([]));

    let ints = |k: &str| -> Vec<i64> {
        v[k].as_array()
            .unwrap_or_else(|| panic!("{k} is not a list: {v}"))
            .iter()
            .map(|x| x.as_i64().unwrap_or_else(|| panic!("{k} has {x}")))
            .collect()
    };
    let (t, p) = (ints("t"), ints("p"));
    assert_eq!((t.len(), p.len()), (48, 48));
    assert!(t.iter().all(|t| (-60..=60).contains(t)), "temperatures in °C: {t:?}");
    assert!(p.iter().all(|p| (0..=100).contains(p)), "probabilities: {p:?}");
    let c = v["c"].as_array().unwrap();
    assert_eq!(c.len(), 48);
    for x in c {
        assert!(
            ["sun", "moon", "partly", "cloud", "snow", "rain"].contains(&x.as_str().unwrap()),
            "{x}"
        );
    }
    // Night and day both show up over 48 hours, so is_day is being read.
    assert!(c.iter().any(|x| x != "moon") && raw["hourly"]["is_day"].as_array().unwrap().contains(&json!(0)));

    for k in ["hi", "lo", "now", "feels", "wind", "humidity"] {
        assert!(v[k].is_number(), "{k} is {}", v[k]);
    }
    assert!(v["hi"].as_i64() >= v["lo"].as_i64());
    let humidity = v["humidity"].as_f64().unwrap();
    assert!((0.0..=100.0).contains(&humidity));
    // Today's high and low bracket today's hourly temperatures (allowing for rounding).
    let (hi, lo) = (v["hi"].as_i64().unwrap(), v["lo"].as_i64().unwrap());
    assert!(
        t[..24].iter().all(|t| (lo - 1..=hi + 1).contains(t)),
        "{lo}..{hi} vs {t:?}"
    );
}
