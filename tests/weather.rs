//! Open-Meteo weather against a recorded response, an injected clock and a fake fetch.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use axum::{body::Body, http::Request};
use chrono::{DateTime, Duration, TimeZone, Utc};
use home_hq::{
    AppState, Clock, router,
    settings::{EXAMPLE, Settings},
    weather::{self, Fetch, Weather},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const LISBON: &str = include_str!("fixtures/weather/lisbon-celsius.json");

/// A clock the test moves by hand.
fn clock() -> (Clock, Arc<Mutex<DateTime<Utc>>>) {
    let now = Arc::new(Mutex::new(Utc.with_ymd_and_hms(2026, 11, 6, 9, 20, 0).unwrap()));
    let c = now.clone();
    (Arc::new(move || *c.lock().unwrap()), now)
}

/// A fetch that answers from `replies` in turn (the last one repeats), counting calls and URLs.
struct Fake {
    calls: AtomicUsize,
    urls: Mutex<Vec<String>>,
    replies: Mutex<Vec<Result<String, String>>>,
}

impl Fake {
    fn new(replies: Vec<Result<&str, &str>>) -> Arc<Self> {
        Arc::new(Fake {
            calls: AtomicUsize::new(0),
            urls: Mutex::new(vec![]),
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|r| r.map(String::from).map_err(String::from))
                    .collect(),
            ),
        })
    }

    fn fetch(self: &Arc<Self>) -> Fetch {
        let me = self.clone();
        Arc::new(move |url| {
            let me = me.clone();
            Box::pin(async move {
                let n = me.calls.fetch_add(1, Ordering::SeqCst);
                me.urls.lock().unwrap().push(url);
                let replies = me.replies.lock().unwrap();
                match &replies[n.min(replies.len() - 1)] {
                    Ok(body) => Ok(body.clone()),
                    Err(e) => Err(anyhow::anyhow!(e.clone())),
                }
            })
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn settings() -> Settings {
    Settings::parse(EXAMPLE).unwrap()
}

#[tokio::test]
async fn maps_the_recorded_forecast_to_upstreams_shape() {
    let (clock, _) = clock();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    let v = w.get().await;

    assert_eq!(v["day"], "2026-11-06");
    assert_eq!(v["units"], "celsius");
    assert_eq!(v["now"], 13); // 12.6
    assert_eq!(v["feels"], 10); // 10.4
    assert_eq!(v["humidity"], 81);
    assert_eq!(v["wind"], 15); // 14.5 rounds up, as Math.round does
    assert_eq!(v["hi"], 19); // 18.6
    assert_eq!(v["lo"], 7); // 7.4
    assert_eq!(v["warnings"], json!([]));

    let t = v["t"].as_array().unwrap();
    let p = v["p"].as_array().unwrap();
    let c = v["c"].as_array().unwrap();
    assert_eq!((t.len(), p.len(), c.len()), (48, 48, 48));
    assert_eq!(&t[..4], &[json!(9), json!(8), json!(8), json!(7)]); // 8.9 8.2 7.9 7.4
    // null probabilities become 0, others pass through
    assert_eq!(&p[44..], &[json!(12), json!(0), json!(0), json!(0)]);
    assert_eq!(p[16], 63);
    // codes 0,1,0,1 at night, then 2,3,45,48 (night), 3 2 1 0 by day ...
    let first: Vec<&str> = c[..12].iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(
        first,
        [
            "moon", "moon", "moon", "moon", "partly", "cloud", "cloud", "cloud", "cloud", "partly", "sun", "sun"
        ]
    );
    // tomorrow's snow block and storms
    let snow: Vec<&str> = c[30..38].iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(snow, ["snow", "snow", "snow", "snow", "rain", "rain", "rain", "rain"]);
}

#[tokio::test]
async fn requests_upstreams_fields_for_the_configured_place() {
    let (clock, _) = clock();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    w.get().await;
    let url = fake.urls.lock().unwrap()[0].clone();
    assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?"), "{url}");
    for part in [
        "latitude=38.7223",
        "longitude=-9.1393",
        "timezone=Europe%2FLisbon",
        "forecast_days=2",
        "temperature_unit=celsius",
        "wind_speed_unit=kmh",
        "hourly=temperature_2m%2Cprecipitation_probability%2Cweather_code%2Cis_day",
        "daily=temperature_2m_max%2Ctemperature_2m_min",
        "current=temperature_2m%2Capparent_temperature%2Crelative_humidity_2m%2Cwind_speed_10m",
    ] {
        assert!(url.contains(part), "{url} lacks {part}");
    }
}

#[tokio::test]
async fn fahrenheit_asks_for_fahrenheit_and_mph() {
    let (clock, _) = clock();
    let s =
        Settings::parse(r#"{"timezone":"America/New_York","location":{"lat":40.7,"lon":-74,"units":"fahrenheit"}}"#)
            .unwrap();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let w = Weather::new(&s, weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    assert_eq!(w.get().await["units"], "fahrenheit");
    let url = fake.urls.lock().unwrap()[0].clone();
    assert!(
        url.contains("temperature_unit=fahrenheit") && url.contains("wind_speed_unit=mph"),
        "{url}"
    );
    assert!(url.contains("timezone=America%2FNew_York"), "{url}");
}

#[tokio::test]
async fn caches_for_fifteen_minutes() {
    let (clock, now) = clock();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    w.get().await;
    *now.lock().unwrap() += Duration::minutes(14) + Duration::seconds(59);
    w.get().await;
    assert_eq!(fake.calls(), 1, "still fresh");
    *now.lock().unwrap() += Duration::seconds(1);
    w.get().await;
    assert_eq!(fake.calls(), 2, "refetched at 15 minutes");
}

#[tokio::test]
async fn serves_stale_when_a_refresh_fails() {
    let (clock, now) = clock();
    let fake = Fake::new(vec![Ok(LISBON), Err("weather: HTTP 503"), Ok("not json")]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    let fresh = w.get().await;
    *now.lock().unwrap() += Duration::minutes(16);
    assert_eq!(w.get().await, fresh, "HTTP error serves the last good value");
    assert_eq!(fake.calls(), 2);
    *now.lock().unwrap() += Duration::minutes(1);
    assert_eq!(
        w.get().await,
        fresh,
        "a bad body does too, and stale data isn't cached as fresh"
    );
    assert_eq!(fake.calls(), 3);
}

#[tokio::test]
async fn error_with_nothing_cached() {
    let (clock, _) = clock();
    let fake = Fake::new(vec![Err("weather: HTTP 503")]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    assert_eq!(w.get().await, json!({ "error": "weather: HTTP 503", "warnings": [] }));

    let fake = Fake::new(vec![Ok(r#"{"hourly":{}}"#)]);
    let (clock, _) = self::clock();
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    assert!(w.get().await["error"].as_str().unwrap().starts_with("weather:"));
}

#[tokio::test]
async fn no_location_is_none_without_fetching() {
    let (clock, _) = clock();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let s = Settings::parse(r#"{"timezone":"Europe/Lisbon"}"#).unwrap();
    let w = Weather::new(&s, weather::OPEN_METEO_URL, fake.fetch(), clock).unwrap();
    assert!(w.request_url().is_none());
    assert_eq!(w.get().await, json!({ "none": true, "warnings": [] }));
    assert_eq!(fake.calls(), 0);
}

#[tokio::test]
async fn the_route_serves_live_weather_when_configured() {
    let (clock, _) = clock();
    let fake = Fake::new(vec![Ok(LISBON)]);
    let w = Weather::new(&settings(), weather::OPEN_METEO_URL, fake.fetch(), clock.clone()).unwrap();
    let app = router(AppState::new(settings(), None, clock).with_weather(w));
    let res = app
        .oneshot(Request::get("/api/weather").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let v: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["now"], 13);
    assert_eq!(v["day"], "2026-11-06");
}
