//! The ICS adapter: the shared contract suite, plus caching and failure handling.

#[path = "support/calendar_contract.rs"]
mod calendar_contract;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use calendar_contract::{Harness, Seed};
use chrono::{DateTime, Duration, Utc};
use home_hq::calendar::fetch::{Fetch, Response};
use home_hq::calendar::ics::{FeedConfig, IcsSource, People, parse_config};
use home_hq::calendar::{CalendarSource, Range};

/// Serves fixed responses by URL and counts requests.
#[derive(Default)]
struct FakeFetch {
    routes: Mutex<HashMap<String, Result<Response, String>>>,
    hits: AtomicUsize,
}

impl FakeFetch {
    fn serve(&self, url: &str, status: u16, body: &str) {
        self.routes.lock().unwrap().insert(
            url.into(),
            Ok(Response {
                status,
                body: body.into(),
            }),
        );
    }

    fn fail(&self, url: &str, err: &str) {
        self.routes.lock().unwrap().insert(url.into(), Err(err.into()));
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Fetch for FakeFetch {
    async fn get(&self, url: &str) -> Result<Response, String> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        self.routes
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err("connection refused".into()))
    }
}

/// A clock the test moves.
#[derive(Clone)]
struct TestClock(Arc<Mutex<DateTime<Utc>>>);

impl TestClock {
    fn new(t: DateTime<Utc>) -> Self {
        TestClock(Arc::new(Mutex::new(t)))
    }
    fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
    fn clock(&self) -> home_hq::Clock {
        let c = self.0.clone();
        Arc::new(move || *c.lock().unwrap())
    }
}

fn url(i: usize) -> String {
    format!("https://calendar.example/ical/secret-token-{i}/basic.ics")
}

fn feeds(seed: &Seed) -> Vec<FeedConfig> {
    seed.calendars
        .iter()
        .enumerate()
        .map(|(i, c)| FeedConfig {
            url: url(i),
            people: People::Many(c.people.clone()),
        })
        .collect()
}

#[tokio::test]
async fn ics_passes_the_calendar_contract() {
    calendar_contract::run(|seed: Seed| async move {
        let fetch = Arc::new(FakeFetch::default());
        for (i, c) in seed.calendars.iter().enumerate() {
            if c.broken {
                fetch.serve(&url(i), 404, "Not Found");
            } else {
                fetch.serve(&url(i), 200, &c.ics);
            }
        }
        let clock = TestClock::new(seed.now);
        let source = IcsSource::new(feeds(&seed), &seed.people, seed.tz, fetch, clock.clock());
        Harness {
            source: Arc::new(source),
            secrets: (0..seed.calendars.len())
                .map(url)
                .chain(["secret-token".into()])
                .collect(),
        }
    })
    .await;
}

#[tokio::test]
async fn not_configured_without_calendars() {
    calendar_contract::check_not_configured(&home_hq::calendar::NotConfigured).await;
    let fetch: Arc<dyn Fetch> = Arc::new(FakeFetch::default());
    let clock = TestClock::new(calendar_contract::now());
    for unset in [None, Some(""), Some("  ")] {
        assert!(IcsSource::from_env_value(unset, &[], chrono_tz::UTC, fetch.clone(), clock.clock()).is_none());
    }
}

struct Setup {
    fetch: Arc<FakeFetch>,
    clock: TestClock,
    source: IcsSource,
}

fn setup(entries: &[(usize, &str)]) -> Setup {
    let seed = calendar_contract::seed(false);
    let fetch = Arc::new(FakeFetch::default());
    fetch.serve(&url(0), 200, calendar_contract::ALEX_ICS);
    fetch.serve(&url(1), 200, calendar_contract::SAM_ICS);
    let clock = TestClock::new(seed.now);
    let configs = entries
        .iter()
        .map(|(i, people)| FeedConfig {
            url: url(*i),
            people: serde_json::from_str(people).unwrap(),
        })
        .collect();
    let source = IcsSource::new(configs, &seed.people, seed.tz, fetch.clone(), clock.clock());
    Setup { fetch, clock, source }
}

fn window() -> Range {
    Range::around(calendar_contract::now())
}

#[tokio::test]
async fn feeds_are_cached_for_ten_minutes() {
    let s = setup(&[(0, r#"["alex"]"#), (1, r#""sam""#)]);
    s.source.list(window()).await;
    assert_eq!(s.fetch.hits(), 2);
    s.clock.advance(Duration::minutes(9));
    let l = s.source.list(window()).await;
    assert_eq!(s.fetch.hits(), 2, "still cached");
    assert_eq!(l.checked, calendar_contract::now());
    s.clock.advance(Duration::minutes(1));
    let l = s
        .source
        .list(Range::around(calendar_contract::now() + Duration::minutes(10)))
        .await;
    assert_eq!(s.fetch.hits(), 4, "read again after the TTL");
    assert_eq!(l.checked, calendar_contract::now() + Duration::minutes(10));
}

#[tokio::test]
async fn a_range_outside_the_cached_window_reads_again() {
    let s = setup(&[(0, r#"["alex"]"#)]);
    s.source.list(window()).await;
    let far = calendar_contract::now() + Duration::days(200);
    s.source.list(Range::new(far, far + Duration::days(1))).await;
    assert_eq!(s.fetch.hits(), 2);
}

#[tokio::test]
async fn a_failing_feed_keeps_its_last_events_and_reports_without_its_url() {
    let s = setup(&[(0, r#"["alex"]"#), (1, r#"["sam"]"#)]);
    let before = s.source.list(window()).await;
    assert!(before.events.iter().any(|e| e.title == "Gym"));

    for (status, body, expected) in [
        (
            404,
            "Not Found",
            "Calendar 2: HTTP 404 (link wrong, or reset in Google)",
        ),
        (500, "oops", "Calendar 2: HTTP 500"),
        (
            200,
            "<!doctype html><html><body>Sign in</body></html>",
            r#"Calendar 2: not an iCal feed (use "Secret address in iCal format")"#,
        ),
    ] {
        s.fetch.serve(&url(1), status, body);
        s.clock.advance(Duration::minutes(11));
        let now = calendar_contract::now();
        let l = s.source.list(Range::around(now)).await;
        assert_eq!(l.errors, [expected]);
        assert!(l.events.iter().any(|e| e.title == "Gym"), "kept after {status}");
        assert!(l.events.iter().any(|e| e.title == "Dentist"));
    }

    s.fetch.fail(&url(1), "error sending request: dns error");
    s.clock.advance(Duration::minutes(11));
    let l = s.source.list(window()).await;
    assert_eq!(l.errors, ["Calendar 2: error sending request: dns error"]);
    for e in &l.errors {
        assert!(!e.contains("secret-token"));
    }

    // Recovers on the next read.
    s.fetch.serve(&url(1), 200, calendar_contract::SAM_ICS);
    s.clock.advance(Duration::minutes(11));
    assert!(s.source.list(window()).await.errors.is_empty());
}

#[tokio::test]
async fn a_feed_that_never_loaded_has_no_events() {
    let s = setup(&[(0, r#"["alex"]"#), (5, r#"["sam"]"#)]);
    let l = s.source.list(window()).await;
    assert_eq!(l.errors, ["Calendar 2: connection refused"]);
    assert!(l.events.iter().all(|e| e.people == ["alex"]));
}

#[tokio::test]
async fn people_all_one_or_unknown() {
    // "all" is everyone; an unknown id is reported and the known ones kept.
    let s = setup(&[(0, r#""all""#), (1, r#"["sam", "robin"]"#)]);
    let l = s.source.list(window()).await;
    assert_eq!(
        l.errors,
        ["Calendar 2: unknown people robin (ids must match settings.json)"]
    );
    let dentist = l.events.iter().find(|e| e.title == "Dentist").unwrap();
    assert_eq!(dentist.people, ["alex", "sam"]);
    let gym = l.events.iter().find(|e| e.title == "Gym").unwrap();
    assert_eq!(gym.people, ["sam"]);
}

#[tokio::test]
async fn webcal_links_are_read_over_https() {
    let fetch = Arc::new(FakeFetch::default());
    fetch.serve("https://p01.example/published/abc", 200, calendar_contract::SAM_ICS);
    let clock = TestClock::new(calendar_contract::now());
    let source = IcsSource::from_env_value(
        Some(r#"[{"url": " webcal://p01.example/published/abc ", "people": ["sam"]}]"#),
        &["sam".into()],
        chrono_tz::Europe::Lisbon,
        fetch.clone(),
        clock.clock(),
    )
    .unwrap();
    let l = source.list(window()).await;
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    assert!(!l.events.is_empty());
}

#[tokio::test]
async fn a_broken_setting_is_reported_without_echoing_it() {
    for (value, expected) in [
        ("https://secret.example/x.ics", "HQ_CALENDARS is not valid JSON"),
        (
            r#"{"url": "https://secret.example/x.ics"}"#,
            "HQ_CALENDARS must be a list [ … ]",
        ),
    ] {
        assert_eq!(parse_config(value).unwrap_err(), expected);
        let fetch: Arc<dyn Fetch> = Arc::new(FakeFetch::default());
        let clock = TestClock::new(calendar_contract::now());
        let source = IcsSource::from_env_value(Some(value), &[], chrono_tz::UTC, fetch, clock.clock()).unwrap();
        assert!(source.configured());
        let l = source.list(window()).await;
        assert_eq!(l.errors, [expected]);
        assert!(l.events.is_empty());
    }
    let err = parse_config(r#"[{"people": ["alex"]}]"#).unwrap_err();
    assert!(!err.contains("alex") && err.starts_with("HQ_CALENDARS entries"));
}

#[tokio::test]
async fn the_listing_has_upstreams_shape() {
    let s = setup(&[(0, r#"["alex"]"#), (1, r#"["sam"]"#)]);
    let v = serde_json::to_value(s.source.list(window()).await).unwrap();
    assert_eq!(v["checked"], calendar_contract::now().timestamp_millis());
    assert_eq!(v["errors"], serde_json::json!([]));
    let ev = |t: &str| {
        v["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["title"] == t)
            .unwrap()
            .clone()
    };
    let dentist = ev("Dentist");
    assert_eq!(dentist["start"], "2026-11-06T12:30:00.000Z");
    assert_eq!(dentist["end"], "2026-11-06T13:30:00.000Z");
    assert_eq!(dentist["people"], serde_json::json!(["alex"]));
    assert_eq!(dentist["loc"], "Rua Augusta 10");
    assert_eq!(dentist["notes"], "Bring the insurance card.\nTravel: 20");
    assert_eq!(dentist["travel"], 20);
    assert!(dentist.get("key").is_none() && dentist.get("allDay").is_none() && dentist.get("walk").is_none());
    let conf = ev("Conference");
    assert_eq!(conf["allDay"], true);
    assert_eq!(conf["start"], "2026-11-10");
    assert_eq!(conf["end"], "2026-11-12");
}
