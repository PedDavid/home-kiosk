//! Travel times: the shared `Drives` wrapper against a counting fake, the OSRM adapter against
//! recorded-format answers (tests/fixtures/travel), the settings, and `drive` on `/api/calendar`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{body::Body, http::Request};
use chrono::{DateTime, Duration, TimeZone, Utc};
use home_hq::calendar::fetch::{Fetch, Response};
use home_hq::calendar::{CalendarSource, Event, Listing, Range, When};
use home_hq::settings::{EXAMPLE, Settings};
use home_hq::travel::osrm::{Geocoder, Osrm};
use home_hq::travel::{self, Drive, Drives, Origins, Place, Rules, TravelError, TravelTime, Trip};
use home_hq::{AppState, Clock, router};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

// ------------------------------------------------------------------ helpers

fn t0() -> DateTime<Utc> {
    // Thursday 5 November 2026, 5:30 PM in Lisbon (UTC+0 in winter).
    Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap()
}

/// A clock the test moves.
#[derive(Clone)]
struct TestClock(Arc<Mutex<DateTime<Utc>>>);

impl TestClock {
    fn new(at: DateTime<Utc>) -> Self {
        TestClock(Arc::new(Mutex::new(at)))
    }
    fn advance(&self, d: Duration) {
        *self.0.lock().unwrap() += d;
    }
    fn clock(&self) -> Clock {
        let c = self.0.clone();
        Arc::new(move || *c.lock().unwrap())
    }
}

type Answers = HashMap<(String, String), Result<Option<u32>, TravelError>>;

/// The counting fake: answers from a table keyed by (origin key, destination), counting calls.
#[derive(Default)]
struct Fake {
    answers: Mutex<Answers>,
    calls: Mutex<Vec<(String, String)>>,
}

impl Fake {
    fn answer(self: &Arc<Self>, from: &Place, to: &str, r: Result<Option<u32>, TravelError>) -> Arc<Self> {
        self.answers.lock().unwrap().insert((from.key(), to.to_string()), r);
        self.clone()
    }
    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl TravelTime for Fake {
    fn configured(&self) -> bool {
        true
    }
    async fn minutes(&self, from: &Place, to: &str) -> Result<Option<u32>, TravelError> {
        self.calls.lock().unwrap().push((from.key(), to.to_string()));
        self.answers
            .lock()
            .unwrap()
            .get(&(from.key(), to.to_string()))
            .cloned()
            .unwrap_or(Ok(Some(15)))
    }
}

fn home() -> Place {
    Place::Coords {
        lat: 38.7223,
        lon: -9.1393,
    }
}

fn work() -> Place {
    Place::Coords { lat: 38.75, lon: -9.25 }
}

/// Home for everyone; Alex also has their own origin (work).
fn origins() -> Origins {
    Origins {
        home: Some(home()),
        people: BTreeMap::from([("alex".to_string(), ("Alex".to_string(), work()))]),
    }
}

fn drives(fake: &Arc<Fake>, rules: Rules, clock: &TestClock) -> Drives {
    Drives::new(fake.clone(), origins(), rules, chrono_tz::Europe::Lisbon, clock.clock())
}

struct Ev {
    id: &'static str,
    loc: Option<&'static str>,
    start: DateTime<Utc>,
    all_day: bool,
    people: Vec<String>,
}

fn ev(id: &'static str, loc: &'static str, in_mins: i64, people: &[&str]) -> Ev {
    Ev {
        id,
        loc: Some(loc),
        start: t0() + Duration::minutes(in_mins),
        all_day: false,
        people: people.iter().map(|s| s.to_string()).collect(),
    }
}

fn trips(evs: &[Ev]) -> Vec<Trip<'_>> {
    evs.iter()
        .map(|e| Trip {
            id: e.id,
            location: e.loc,
            start: e.start,
            all_day: e.all_day,
            people: &e.people,
        })
        .collect()
}

fn drive(mins: u32, from: &str) -> Drive {
    Drive {
        mins,
        from: from.into(),
    }
}

// ------------------------------------------------------------------ the wrapper

#[tokio::test]
async fn results_are_cached_for_ten_minutes() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default()).answer(&home(), "Sintra", Ok(Some(35)));
    let d = drives(&fake, Rules::default(), &clock);
    let evs = [ev("a", "Sintra", 90, &["sam"])];

    assert_eq!(d.for_trips(&trips(&evs)).await["a"], drive(35, "home"));
    assert_eq!(fake.calls(), 1);

    clock.advance(Duration::minutes(9));
    assert_eq!(d.for_trips(&trips(&evs)).await["a"], drive(35, "home"));
    assert_eq!(fake.calls(), 1, "still cached at 9 minutes");

    clock.advance(Duration::minutes(2));
    d.for_trips(&trips(&evs)).await;
    assert_eq!(fake.calls(), 2, "asked again after 10 minutes");
}

#[tokio::test]
async fn the_same_place_is_one_request_across_events() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default());
    let d = drives(&fake, Rules::default(), &clock);
    // Different spacing and case: the same destination.
    let evs = [ev("a", "Sintra", 60, &["sam"]), ev("b", "  sintra ", 120, &["sam"])];
    let got = d.for_trips(&trips(&evs)).await;
    assert_eq!(got.len(), 2);
    assert_eq!(fake.calls(), 1);
}

#[tokio::test]
async fn failures_and_unknown_places_are_cached_too() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default())
        .answer(&home(), "Broken", Err(TravelError("osrm: HTTP 502".into())))
        .answer(&home(), "Nowhere", Ok(None));
    let d = drives(&fake, Rules::default(), &clock);
    let evs = [ev("a", "Broken", 60, &["sam"]), ev("b", "Nowhere", 60, &["sam"])];

    assert!(d.for_trips(&trips(&evs)).await.is_empty(), "both fall back to Travel:");
    assert_eq!(fake.calls(), 2);
    clock.advance(Duration::minutes(5));
    d.for_trips(&trips(&evs)).await;
    assert_eq!(fake.calls(), 2, "not asked again while cached");
    clock.advance(Duration::minutes(6));
    d.for_trips(&trips(&evs)).await;
    assert_eq!(fake.calls(), 4, "asked again once expired");
}

#[tokio::test]
async fn only_timed_events_with_a_location_in_the_next_four_hours() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default());
    let d = drives(&fake, Rules::default(), &clock);
    let mut evs = vec![
        ev("soon", "A", 30, &["sam"]),
        ev("edge", "B", 4 * 60 - 1, &["sam"]),
        ev("later", "C", 4 * 60 + 1, &["sam"]),
        ev("past", "D", -10, &["sam"]),
        ev("now", "E", 0, &["sam"]),
        ev("blank", "   ", 30, &["sam"]),
        ev("allday", "F", 60, &["sam"]),
    ];
    evs[6].all_day = true;
    evs.push(Ev {
        id: "noloc",
        loc: None,
        start: t0() + Duration::minutes(30),
        all_day: false,
        people: vec!["sam".into()],
    });
    let got = d.for_trips(&trips(&evs)).await;
    let mut ids: Vec<&str> = got.keys().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, ["edge", "soon"]);
    assert_eq!(fake.calls(), 2, "nothing else is asked");
}

#[tokio::test]
async fn at_most_eight_events_soonest_first() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default());
    let d = drives(&fake, Rules::default(), &clock);
    let names = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9"];
    // Listed latest first, so "soonest" has to come from sorting.
    let evs: Vec<Ev> = (0..10)
        .rev()
        .map(|i| ev(names[i], names[i], 10 + i as i64 * 10, &["sam"]))
        .collect();
    let got = d.for_trips(&trips(&evs)).await;
    assert_eq!(got.len(), 8);
    assert!(!got.contains_key("p8") && !got.contains_key("p9"));
}

#[tokio::test]
async fn the_daily_cap_stops_requests_until_local_midnight() {
    // 11 PM in Lisbon.
    let clock = TestClock::new(Utc.with_ymd_and_hms(2026, 11, 5, 23, 0, 0).unwrap());
    let fake = Arc::new(Fake::default());
    let rules = Rules {
        daily_cap: 2,
        ..Rules::default()
    };
    let d = drives(&fake, rules, &clock);
    let at = |id, loc, mins| Ev {
        id,
        loc: Some(loc),
        start: Utc.with_ymd_and_hms(2026, 11, 5, 23, 0, 0).unwrap() + Duration::minutes(mins),
        all_day: false,
        people: vec!["sam".into()],
    };
    let evs = [at("a", "A", 30), at("b", "B", 40), at("c", "C", 50)];

    let got = d.for_trips(&trips(&evs)).await;
    assert_eq!(fake.calls(), 2);
    assert!(got.contains_key("a") && got.contains_key("b") && !got.contains_key("c"));
    assert_eq!(d.requests_today().await, 2);

    // Cached answers still come back once the cap is spent; nothing new is asked.
    clock.advance(Duration::minutes(11));
    let evs = [at("a", "A", 30), at("c", "C", 50)];
    assert!(d.for_trips(&trips(&evs)).await.is_empty());
    assert_eq!(fake.calls(), 2);

    // After midnight in Lisbon (00:15 on the 6th) the budget is back.
    clock.advance(Duration::minutes(64));
    let evs = [at("c", "C", 120)];
    assert!(d.for_trips(&trips(&evs)).await.contains_key("c"));
    assert_eq!(fake.calls(), 3);
    assert_eq!(d.requests_today().await, 1);
}

#[tokio::test]
async fn the_longest_drive_wins_when_several_people_attend() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default())
        .answer(&work(), "Dentist", Ok(Some(30)))
        .answer(&home(), "Dentist", Ok(Some(20)))
        .answer(&work(), "Gym", Ok(Some(5)))
        .answer(&home(), "Gym", Ok(Some(12)));
    let d = drives(&fake, Rules::default(), &clock);
    let evs = [
        ev("dentist", "Dentist", 60, &["alex", "sam"]),
        ev("gym", "Gym", 60, &["sam", "alex"]),
        ev("alone", "Gym", 90, &["alex"]),
    ];
    let got = d.for_trips(&trips(&evs)).await;
    assert_eq!(got["dentist"], drive(30, "Alex"), "from Alex's origin");
    assert_eq!(got["gym"], drive(12, "home"));
    assert_eq!(got["alone"], drive(5, "Alex"));
    assert_eq!(fake.calls(), 4, "each origin and place asked once");
}

#[tokio::test]
async fn people_who_start_from_home_share_one_request() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default());
    let d = drives(&fake, Rules::default(), &clock);
    let evs = [ev("a", "Park", 60, &["sam", "jo", "kim"])];
    assert_eq!(d.for_trips(&trips(&evs)).await["a"], drive(15, "home"));
    assert_eq!(fake.calls(), 1);
}

#[tokio::test]
async fn a_failed_origin_doesnt_hide_the_other_one() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default())
        .answer(&work(), "Dentist", Err(TravelError("timeout".into())))
        .answer(&home(), "Dentist", Ok(Some(20)));
    let d = drives(&fake, Rules::default(), &clock);
    let evs = [ev("a", "Dentist", 60, &["alex", "sam"])];
    assert_eq!(d.for_trips(&trips(&evs)).await["a"], drive(20, "home"));
}

#[tokio::test]
async fn an_event_whose_origins_the_cap_cut_short_gets_no_drive() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default())
        .answer(&work(), "Dentist", Ok(Some(10)))
        .answer(&home(), "Dentist", Ok(Some(40)));
    let rules = Rules {
        daily_cap: 1,
        ..Rules::default()
    };
    let d = drives(&fake, rules, &clock);
    let evs = [ev("a", "Dentist", 60, &["alex", "sam"])];
    // Only Alex's 10 minutes could be asked; Sam's 40 from home would be the real answer.
    assert!(d.for_trips(&trips(&evs)).await.is_empty());
    assert_eq!(fake.calls(), 1);
}

#[tokio::test]
async fn implausibly_long_drives_are_ignored() {
    let clock = TestClock::new(t0());
    let fake = Arc::new(Fake::default()).answer(&home(), "Sintra", Ok(Some(181)));
    let d = drives(&fake, Rules::default(), &clock);
    assert!(d.for_trips(&trips(&[ev("a", "Sintra", 60, &["sam"])])).await.is_empty());
}

#[tokio::test]
async fn nothing_is_asked_when_not_configured() {
    let clock = TestClock::new(t0());
    let d = Drives::not_configured(chrono_tz::Europe::Lisbon, clock.clock());
    assert!(!d.configured());
    assert!(d.for_trips(&trips(&[ev("a", "Sintra", 60, &["sam"])])).await.is_empty());

    let fake = Arc::new(Fake::default());
    let no_origin = Drives::new(
        fake.clone(),
        Origins::default(),
        Rules::default(),
        chrono_tz::UTC,
        clock.clock(),
    );
    assert!(!no_origin.configured());
    assert!(
        no_origin
            .for_trips(&trips(&[ev("a", "Sintra", 60, &["sam"])]))
            .await
            .is_empty()
    );
    assert_eq!(fake.calls(), 0);
}

// ------------------------------------------------------------------ the OSRM adapter

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/travel/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// A fake HTTP GET: the first rule whose prefix the URL starts with answers. Records each URL.
#[derive(Default)]
struct Http {
    rules: Vec<(String, u16, String)>,
    seen: Mutex<Vec<String>>,
}

impl Http {
    fn on(mut self, prefix: &str, status: u16, fixture_name: &str) -> Self {
        self.rules.push((prefix.into(), status, fixture(fixture_name)));
        self
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl Fetch for Http {
    async fn get(&self, url: &str) -> Result<Response, String> {
        self.seen.lock().unwrap().push(url.to_string());
        self.rules
            .iter()
            .find(|(p, _, _)| url.starts_with(p.as_str()))
            .map(|(_, status, body)| Response {
                status: *status,
                body: body.clone(),
            })
            .ok_or_else(|| "connection refused".to_string())
    }
}

const OSRM: &str = "http://osrm:5000";
const PHOTON: &str = "http://photon:2322";
const NOMINATIM: &str = "http://nominatim:8080";

fn osrm(http: &Arc<Http>, geocoder: Geocoder) -> Osrm {
    Osrm::new(
        http.clone(),
        format!("{OSRM}/"),
        "driving".into(),
        geocoder,
        Some((38.7223, -9.1393)),
    )
}

#[tokio::test]
async fn photon_then_osrm_gives_minutes_rounded_up() {
    let http = Arc::new(
        Http::default()
            .on(&format!("{PHOTON}/api?"), 200, "photon-sintra.json")
            .on(&format!("{OSRM}/route/v1/driving/"), 200, "osrm-route-ok.json"),
    );
    let a = osrm(&http, Geocoder::Photon(PHOTON.into()));
    // 1563.2 s is 26.05 minutes: 27, so "leave by" is never late.
    assert_eq!(a.minutes(&home(), "Sintra Palace, Sintra").await, Ok(Some(27)));
    let seen = http.seen();
    assert_eq!(
        seen[0],
        "http://photon:2322/api?q=Sintra+Palace%2C+Sintra&limit=1&lat=38.7223&lon=-9.1393"
    );
    // OSRM takes longitude first.
    assert_eq!(
        seen[1],
        "http://osrm:5000/route/v1/driving/-9.139300,38.722300;-9.381709,38.802869?overview=false&alternatives=false&steps=false"
    );
}

#[tokio::test]
async fn nominatim_works_too_and_prefers_places_near_home() {
    let http = Arc::new(
        Http::default()
            .on(&format!("{NOMINATIM}/search?"), 200, "nominatim-sintra.json")
            .on(&format!("{OSRM}/route/"), 200, "osrm-route-ok.json"),
    );
    let a = osrm(&http, Geocoder::Nominatim(format!("{NOMINATIM}/")));
    assert_eq!(a.minutes(&home(), "Sintra").await, Ok(Some(27)));
    let seen = http.seen();
    assert!(
        seen[0].starts_with("http://nominatim:8080/search?q=Sintra&format=jsonv2&limit=1&viewbox="),
        "{}",
        seen[0]
    );
    assert!(seen[1].contains(";-9.381709,38.802869?"));
}

#[tokio::test]
async fn an_address_origin_is_geocoded() {
    let http = Arc::new(
        Http::default()
            .on(&format!("{PHOTON}/api?q=Rua+do+Sol"), 200, "photon-home.json")
            .on(&format!("{PHOTON}/api?q=Sintra"), 200, "photon-sintra.json")
            .on(&format!("{OSRM}/route/"), 200, "osrm-route-ok.json"),
    );
    let a = osrm(&http, Geocoder::Photon(PHOTON.into()));
    let from = Place::Address("Rua do Sol, Lisboa".into());
    assert_eq!(a.minutes(&from, "Sintra").await, Ok(Some(27)));
    assert!(http.seen()[2].contains("/driving/-9.139337,38.722252;"));
}

#[tokio::test]
async fn an_unknown_place_is_unknown_without_routing() {
    for (geocoder, prefix, empty) in [
        (Geocoder::Photon(PHOTON.into()), PHOTON, "photon-empty.json"),
        (Geocoder::Nominatim(NOMINATIM.into()), NOMINATIM, "nominatim-empty.json"),
    ] {
        let http = Arc::new(Http::default().on(prefix, 200, empty));
        let a = osrm(&http, geocoder);
        assert_eq!(a.minutes(&home(), "Somewhere made up").await, Ok(None));
        assert_eq!(http.seen().len(), 1, "OSRM isn't asked");
    }
}

#[tokio::test]
async fn no_route_is_unknown_but_osrm_errors_are_failures() {
    let with = |status, name| {
        Arc::new(
            Http::default()
                .on(PHOTON, 200, "photon-sintra.json")
                .on(OSRM, status, name),
        )
    };
    let a = osrm(&with(400, "osrm-route-noroute.json"), Geocoder::Photon(PHOTON.into()));
    assert_eq!(a.minutes(&home(), "Sintra").await, Ok(None));

    let a = osrm(&with(400, "osrm-route-invalid.json"), Geocoder::Photon(PHOTON.into()));
    let e = a.minutes(&home(), "Sintra").await.unwrap_err();
    assert_eq!(e.0, "osrm: HTTP 400, InvalidQuery");

    let a = osrm(&with(502, "gateway-502.html"), Geocoder::Photon(PHOTON.into()));
    assert_eq!(
        a.minutes(&home(), "Sintra").await.unwrap_err().0,
        "osrm: HTTP 502, not JSON"
    );
}

#[tokio::test]
async fn geocoder_failures_are_failures() {
    let http = Arc::new(Http::default().on(PHOTON, 502, "gateway-502.html"));
    let a = osrm(&http, Geocoder::Photon(PHOTON.into()));
    assert_eq!(
        a.minutes(&home(), "Sintra").await.unwrap_err().0,
        "photon: HTTP 502, not JSON"
    );

    let http = Arc::new(Http::default());
    let a = osrm(&http, Geocoder::Photon(PHOTON.into()));
    assert_eq!(
        a.minutes(&home(), "Sintra").await.unwrap_err().0,
        "photon: connection refused"
    );
}

// ------------------------------------------------------------------ settings

fn settings(travel: &str, people: &str) -> Settings {
    Settings::parse(&format!(
        r#"{{"timezone":"Europe/Lisbon","location":{{"lat":38.7223,"lon":-9.1393}},
            "people":{people},"travel":{travel}}}"#
    ))
    .unwrap()
}

fn build(s: &Settings) -> anyhow::Result<Drives> {
    travel::from_settings(s, Arc::new(Http::default()), TestClock::new(t0()).clock())
}

#[test]
fn travel_settings() {
    let none = Settings::parse(EXAMPLE).unwrap();
    assert!(!build(&none).unwrap().configured(), "no travel section: not configured");

    let people = r#"[{"id":"alex","name":"Alex","origin":"Rua do Sol 1, Lisboa"},
                     {"id":"sam","name":"Sam","origin":{"lat":38.75,"lon":-9.25}}]"#;
    assert!(
        build(&settings(r#"{"osrm":"http://o","photon":"http://p"}"#, people))
            .unwrap()
            .configured()
    );
    assert!(
        build(&settings(
            r#"{"osrm":"http://o","nominatim":"http://n","home":"Rua do Sol 1"}"#,
            "[]"
        ))
        .unwrap()
        .configured()
    );

    let err = |travel: &str| build(&settings(travel, "[]")).err().unwrap().to_string();
    assert!(err(r#"{"photon":"http://p"}"#).contains("travel.osrm"));
    assert!(err(r#"{"osrm":"http://o"}"#).contains("travel.photon or travel.nominatim"));
    assert!(err(r#"{"osrm":"http://o","photon":"http://p","nominatim":"http://n"}"#).contains("only one"));
    assert!(err(r#"{"provider":"google","osrm":"http://o","photon":"http://p"}"#).contains("unknown provider"));
    assert!(
        Settings::parse(r#"{"travel":{"osrm":"http://o","photn":"http://p"}}"#).is_err(),
        "typos are refused"
    );
}

#[test]
fn travel_needs_an_origin() {
    let s = Settings::parse(r#"{"travel":{"osrm":"http://o","photon":"http://p"}}"#).unwrap();
    assert!(build(&s).err().unwrap().to_string().contains("no origin"));
}

// ------------------------------------------------------------------ /api/calendar

struct OneCalendar(Vec<Event>);

#[async_trait]
impl CalendarSource for OneCalendar {
    fn configured(&self) -> bool {
        true
    }
    async fn list(&self, _range: Range) -> Listing {
        Listing {
            events: self.0.clone(),
            errors: vec![],
            checked: t0(),
        }
    }
}

fn event(id: &str, loc: Option<&str>, in_mins: i64, travel: Option<u32>) -> Event {
    let start = t0() + Duration::minutes(in_mins);
    Event {
        id: id.into(),
        key: id.into(),
        title: id.into(),
        people: vec!["alex".into()],
        when: When::Timed {
            start,
            end: start + Duration::hours(1),
        },
        location: loc.map(String::from),
        notes: None,
        travel,
        walk: None,
        local: false,
    }
}

async fn calendar_body(state: AppState) -> Value {
    let req = Request::get("/api/calendar").body(Body::empty()).unwrap();
    let res = router(state).oneshot(req).await.unwrap();
    serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn calendar_events_get_a_drive_and_keep_travel() {
    let http = Arc::new(
        Http::default()
            .on(PHOTON, 200, "photon-sintra.json")
            .on(OSRM, 200, "osrm-route-ok.json"),
    );
    let s = settings(
        &format!(r#"{{"osrm":"{OSRM}","photon":"{PHOTON}"}}"#),
        r##"[{"id":"alex","name":"Alex","color":"#2D5BFF"}]"##,
    );
    let clock: Clock = Arc::new(t0);
    let drives = travel::from_settings(&s, http.clone(), clock.clone()).unwrap();
    let cal = OneCalendar(vec![
        event("sintra", Some("Sintra"), 90, Some(40)),
        event("tomorrow", Some("Sintra"), 26 * 60, Some(40)),
        event("home", None, 60, None),
    ]);
    let state = AppState::new(s, None, clock)
        .with_calendar(Arc::new(cal))
        .with_drives(Arc::new(drives));
    let body = calendar_body(state).await;
    let evs = body["events"].as_array().unwrap();
    assert_eq!(evs[0]["drive"], serde_json::json!({ "mins": 27, "from": "home" }));
    assert_eq!(evs[0]["travel"], 40, "Travel: stays as the fallback");
    assert!(evs[1].get("drive").is_none(), "more than 4 hours away");
    assert!(evs[2].get("drive").is_none());
}

#[tokio::test]
async fn without_travel_settings_calendar_events_have_no_drive() {
    let s = Settings::parse(EXAMPLE).unwrap();
    let cal = OneCalendar(vec![event("sintra", Some("Sintra"), 90, Some(40))]);
    let state = AppState::new(s, None, Arc::new(t0)).with_calendar(Arc::new(cal));
    let body = calendar_body(state).await;
    assert!(body["events"][0].get("drive").is_none());
    assert_eq!(body["events"][0]["travel"], 40);
}
