//! The CalDAV adapter: the shared contract suite against an in-memory CalDAV server, write and
//! failure handling, the `/api` write routes, and a scenario replayed from responses recorded
//! from a real Radicale.
//!
//! No test here touches the network. `radicale_live` (ignored) runs the contract suite and the
//! scenario against a real server and re-records the fixture:
//!
//! ```sh
//! pip install radicale   # then start it with htpasswd user alex:secret-pw, see the fixture README
//! HQ_TEST_RADICALE=http://127.0.0.1:5232 HQ_RECORD=1 cargo test --test calendar_caldav -- --ignored
//! ```

#[path = "support/caldav_fake.rs"]
mod caldav_fake;
#[path = "support/calendar_contract.rs"]
mod calendar_contract;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use caldav_fake::{Collection, Exchange, FakeDav, Replay};
use calendar_contract::{Harness, Seed};
use chrono::{DateTime, Duration, TimeZone, Utc};
use home_hq::calendar::caldav::CalDavSource;
use home_hq::calendar::caldav::config::CalDavConfig;
use home_hq::calendar::caldav::dav::{HttpRequest, HttpResponse, Transport};
use home_hq::calendar::fetch::{Fetch, Response};
use home_hq::calendar::ics::People;
use home_hq::calendar::{CalendarError, CalendarSource, NewEvent, mixed};
use home_hq::settings::{Settings, WriteRule};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const USER: &str = "alex";
const PASSWORD: &str = "secret-pw";
/// `Basic base64("alex:secret-pw")`.
const AUTH: &str = "Basic YWxleDpzZWNyZXQtcHc=";

fn clock(t: DateTime<Utc>) -> home_hq::Clock {
    Arc::new(move || t)
}

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn people(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn config(url: &str, who: People, id: Option<&str>) -> CalDavConfig {
    CalDavConfig {
        id: id.map(String::from),
        url: url.into(),
        people: who,
        username: Some(USER.into()),
        password: Some(PASSWORD.into()),
        read_only: false,
        number: 0,
    }
}

fn cal_path(i: usize) -> String {
    format!("/alex/cal{i}/")
}

// ------------------------------------------------------------------ the contract

#[tokio::test]
async fn caldav_passes_the_calendar_contract() {
    calendar_contract::run(|seed: Seed| async move {
        let dav = Arc::new(FakeDav::new(Some(AUTH)));
        let mut configs = Vec::new();
        for (i, c) in seed.calendars.iter().enumerate() {
            let mut col = Collection::events(&format!("Calendar {i}"));
            col.broken = c.broken;
            dav.add_collection(&cal_path(i), col);
            dav.seed(&cal_path(i), &c.ics);
            configs.push(config(
                &FakeDav::url(&cal_path(i)),
                People::Many(c.people.clone()),
                None,
            ));
        }
        let source = CalDavSource::new(configs, &seed.people, Vec::new(), seed.tz, dav, clock(seed.now));
        Harness {
            source: Arc::new(source),
            secrets: vec![caldav_fake::ORIGIN.into(), PASSWORD.into(), AUTH.into(), "cal1".into()],
        }
    })
    .await;
}

#[tokio::test]
async fn not_configured_without_calendars() {
    let dav = Arc::new(FakeDav::new(None));
    let source = CalDavSource::new(
        Vec::new(),
        &people(&["alex"]),
        Vec::new(),
        chrono_tz::UTC,
        dav,
        clock(calendar_contract::now()),
    );
    calendar_contract::check_not_configured(&source).await;
    assert!(!source.can_write());
}

// ------------------------------------------------------------------ writes

const HOUSE: &[&str] = &["alex", "sam", "maya"];

/// `/alex/cal0/` is Alex's, `/alex/cal1/` Sam's, `/alex/cal2/` the family's ("all").
fn household() -> (Arc<FakeDav>, CalDavSource) {
    household_with(Vec::new(), |_, _| {})
}

fn household_with(rules: Vec<WriteRule>, tweak: impl Fn(usize, &mut Collection)) -> (Arc<FakeDav>, CalDavSource) {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    let who = [
        People::Many(people(&["alex"])),
        People::One("sam".into()),
        People::One("all".into()),
    ];
    let ids = ["alex", "sam", "family"];
    let mut configs = Vec::new();
    for (i, w) in who.into_iter().enumerate() {
        let mut col = Collection::events(ids[i]);
        tweak(i, &mut col);
        dav.add_collection(&cal_path(i), col);
        configs.push(config(&FakeDav::url(&cal_path(i)), w, Some(ids[i])));
    }
    dav.seed(&cal_path(0), calendar_contract::ALEX_ICS);
    let source = CalDavSource::new(
        configs,
        &people(HOUSE),
        rules,
        chrono_tz::Europe::Lisbon,
        dav.clone(),
        clock(calendar_contract::now()),
    );
    (dav, source)
}

fn haircut(id: &str, who: &[&str]) -> NewEvent {
    NewEvent {
        id: id.into(),
        title: "Haircut".into(),
        people: people(who),
        start: utc(2026, 11, 8, 10, 0),
        end: utc(2026, 11, 8, 11, 0),
    }
}

async fn titles(source: &dyn CalendarSource) -> Vec<String> {
    source
        .list(calendar_contract::range())
        .await
        .events
        .into_iter()
        .map(|e| e.title)
        .collect()
}

#[tokio::test]
async fn adds_a_vevent_to_the_calendar_for_those_people() {
    let (dav, source) = household();
    assert!(source.can_write());
    let added = source.add(haircut("m1abc", &["sam"])).await.unwrap();
    assert_eq!(added.id, "m1abc", "the frame's own id, so Undo works");
    assert!(added.local);

    let puts = dav.requests("PUT");
    assert_eq!(puts.len(), 1);
    let put = &puts[0];
    assert_eq!(put.url, FakeDav::url("/alex/cal1/m1abc.ics"), "Sam's calendar");
    assert_eq!(put.header("If-None-Match"), Some("*"), "never overwrites");
    assert_eq!(put.header("Authorization"), Some(AUTH));
    assert_eq!(put.header("Content-Type"), Some("text/calendar; charset=utf-8"));
    let body = put.body.as_deref().unwrap();
    for line in [
        "UID:m1abc@home-hq\r\n",
        "DTSTART:20261108T100000Z\r\n",
        "DTEND:20261108T110000Z\r\n",
        "SUMMARY:Haircut\r\n",
        "DTSTAMP:20261105T173000Z\r\n",
    ] {
        assert!(body.contains(line), "{line:?} in {body}");
    }
    assert!(dav.object("/alex/cal1/", "m1abc.ics").is_some());
}

#[tokio::test]
async fn groups_without_their_own_calendar_go_to_the_family_one_and_keep_their_people() {
    let (dav, source) = household();
    source.add(haircut("pair1", &["alex", "maya"])).await.unwrap();
    assert!(dav.object("/alex/cal2/", "pair1.ics").is_some());
    let l = source.list(calendar_contract::range()).await;
    let e = l.events.iter().find(|e| e.title == "Haircut").unwrap();
    assert_eq!(
        e.people,
        ["alex", "maya"],
        "not everyone, although the calendar is everyone's"
    );
    assert!(e.local);
}

#[tokio::test]
async fn settings_rules_choose_the_calendar() {
    let rules = vec![WriteRule {
        people: People::Many(people(&["sam"])),
        calendar: "family".into(),
    }];
    let (dav, source) = household_with(rules, |_, _| {});
    source.add(haircut("r1", &["sam"])).await.unwrap();
    assert!(dav.object("/alex/cal2/", "r1.ics").is_some());
    assert!(dav.object("/alex/cal1/", "r1.ics").is_none());
}

#[tokio::test]
async fn added_events_show_at_once_and_after_a_re_read() {
    let (_dav, source) = household();
    let before = titles(&source).await;
    source.add(haircut("m1", &["alex"])).await.unwrap();
    // From the cache, before the TTL is up.
    let cached = source.list(calendar_contract::range()).await;
    assert_eq!(cached.events.len(), before.len() + 1);
    // And as the server returns it.
    let fresh = source.with_ttl(Duration::zero());
    let l = fresh.list(calendar_contract::range()).await;
    let e = l.events.iter().find(|e| e.title == "Haircut").unwrap();
    assert_eq!((e.id.as_str(), e.local), ("m1", true));
    assert_eq!(e.people, ["alex"]);
    // Events from elsewhere are not removable from the frame.
    assert!(l.events.iter().filter(|e| e.title != "Haircut").all(|e| !e.local));
}

#[tokio::test]
async fn removes_with_if_match_and_undo_works_straight_after_adding() {
    let (dav, source) = household();
    source.add(haircut("undo1", &["alex"])).await.unwrap();
    let (etag, _) = dav.object("/alex/cal0/", "undo1.ics").unwrap();
    source.remove("undo1").await.unwrap();
    let del = dav.requests("DELETE");
    assert_eq!(del.len(), 1);
    assert_eq!(del[0].url, FakeDav::url("/alex/cal0/undo1.ics"));
    assert_eq!(
        del[0].header("If-Match"),
        Some(etag.as_str()),
        "the ETag the PUT returned"
    );
    assert!(dav.object("/alex/cal0/", "undo1.ics").is_none());
    assert!(!titles(&source).await.contains(&"Haircut".to_string()));
    assert_eq!(source.remove("undo1").await, Err(CalendarError::NotFound));
}

#[tokio::test]
async fn an_etag_mismatch_is_a_conflict_and_keeps_the_event() {
    let (dav, source) = household();
    source.add(haircut("c1", &["alex"])).await.unwrap();
    dav.touch("/alex/cal0/", "c1.ics"); // edited on a phone meanwhile
    let err = source.remove("c1").await.unwrap_err();
    assert_eq!(
        err,
        CalendarError::Backend("the event changed on the calendar server, try again".into())
    );
    assert!(dav.object("/alex/cal0/", "c1.ics").is_some(), "not deleted");
    assert!(titles(&source).await.contains(&"Haircut".to_string()), "still shown");
}

#[tokio::test]
async fn a_failed_save_keeps_the_view() {
    let (dav, source) = household_with(Vec::new(), |i, c| c.broken_writes = i == 0);
    let before = source.list(calendar_contract::range()).await;
    let err = source.add(haircut("f1", &["alex"])).await.unwrap_err();
    assert_eq!(err, CalendarError::Backend("calendar server error (HTTP 500)".into()));
    let after = source.list(calendar_contract::range()).await;
    assert_eq!(after.events, before.events, "nothing lost, nothing half-added");
    assert!(after.errors.is_empty(), "a failed write isn't a broken calendar");
    assert!(dav.object("/alex/cal0/", "f1.ics").is_none());
}

#[tokio::test]
async fn removes_events_it_has_not_listed_yet() {
    // After a restart the cache is empty, but the frame still has the event (and its id).
    let (dav, first) = household();
    first.add(haircut("old1", &["sam"])).await.unwrap();
    drop(first);
    let source = CalDavSource::new(
        vec![config(
            &FakeDav::url("/alex/cal1/"),
            People::One("sam".into()),
            Some("sam"),
        )],
        &people(HOUSE),
        Vec::new(),
        chrono_tz::Europe::Lisbon,
        dav.clone(),
        clock(calendar_contract::now()),
    );
    source.remove("old1").await.unwrap();
    assert!(dav.object("/alex/cal1/", "old1.ics").is_none());
    assert_eq!(source.remove("../etc").await, Err(CalendarError::NotFound));
}

#[tokio::test]
async fn rejects_bad_events_before_any_request() {
    let (dav, source) = household();
    let bad = [
        NewEvent {
            title: "  ".into(),
            ..haircut("a", &["alex"])
        },
        NewEvent {
            end: utc(2026, 11, 8, 9, 0),
            ..haircut("a", &["alex"])
        },
        haircut("a", &["nobody"]),
        haircut("a", &[]),
        haircut("../x", &["alex"]),
    ];
    for e in bad {
        assert!(matches!(source.add(e).await, Err(CalendarError::Invalid(_))));
    }
    assert!(dav.requests("PUT").is_empty());
}

#[tokio::test]
async fn read_only_calendars_never_get_writes() {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection("/alex/cal0/", Collection::events("a"));
    let mut c = config(&FakeDav::url("/alex/cal0/"), People::One("all".into()), None);
    c.read_only = true;
    let source = CalDavSource::new(
        vec![c],
        &people(HOUSE),
        Vec::new(),
        chrono_tz::UTC,
        dav.clone(),
        clock(calendar_contract::now()),
    );
    assert!(source.configured());
    assert!(!source.can_write());
    assert_eq!(source.add(haircut("a", &["alex"])).await, Err(CalendarError::ReadOnly));
    assert!(dav.requests("PUT").is_empty());
}

#[tokio::test]
async fn calendars_the_server_wont_let_us_write_turn_the_capability_off() {
    let (_dav, source) = household_with(Vec::new(), |_, c| c.writable = false);
    assert!(source.can_write(), "not known before discovery");
    source.list(calendar_contract::range()).await;
    assert!(!source.can_write());
}

#[tokio::test]
async fn finds_the_calendar_from_an_account_url() {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection("/alex/personal/", Collection::events("Personal"));
    dav.add_collection(
        "/alex/tasks/",
        Collection {
            components: vec!["VTODO"],
            ..Collection::events("Tasks")
        },
    );
    dav.seed("/alex/personal/", calendar_contract::ALEX_ICS);
    let make = |dav: Arc<FakeDav>| {
        CalDavSource::new(
            vec![config(&FakeDav::url("/alex"), People::One("alex".into()), None)],
            &people(HOUSE),
            Vec::new(),
            chrono_tz::Europe::Lisbon,
            dav,
            clock(calendar_contract::now()),
        )
    };
    let l = make(dav.clone()).list(calendar_contract::range()).await;
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    assert!(l.events.iter().any(|e| e.title == "Dentist"));

    dav.add_collection("/alex/work/", Collection::events("Work"));
    let l = make(dav).list(calendar_contract::range()).await;
    assert_eq!(
        l.errors,
        ["Calendar 1: the URL is not a calendar but has 2 calendars under it: use one of their URLs"]
    );
}

#[tokio::test]
async fn reports_failures_without_urls_or_passwords() {
    let dav = Arc::new(FakeDav::new(Some("Basic something-else")));
    dav.add_collection("/alex/cal0/", Collection::events("a"));
    dav.add_collection(
        "/alex/tasks/",
        Collection {
            components: vec!["VTODO"],
            ..Collection::events("Tasks")
        },
    );
    let source = CalDavSource::new(
        vec![
            config(&FakeDav::url("/alex/cal0/"), People::One("all".into()), None),
            config(&FakeDav::url("/alex/missing/"), People::One("all".into()), None),
        ],
        &people(HOUSE),
        Vec::new(),
        chrono_tz::UTC,
        dav,
        clock(calendar_contract::now()),
    );
    let l = source.list(calendar_contract::range()).await;
    assert_eq!(
        l.errors,
        [
            "Calendar 1: the calendar server refused the credentials",
            "Calendar 2: the calendar server refused the credentials"
        ]
    );
}

// ------------------------------------------------------------------ mixing with ICS

struct OneFeed(String);

#[async_trait]
impl Fetch for OneFeed {
    async fn get(&self, _url: &str) -> Result<Response, String> {
        Ok(Response {
            status: 200,
            body: self.0.clone(),
        })
    }
}

fn settings(extra: Value) -> Settings {
    let mut s = json!({
        "timezone": "Europe/Lisbon",
        "people": [
            { "id": "alex", "name": "Alex", "color": "#2D5BFF" },
            { "id": "sam", "name": "Sam", "color": "#E4007C" }
        ]
    });
    s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    Settings::parse(&s.to_string()).unwrap()
}

#[tokio::test]
async fn ics_and_caldav_calendars_coexist_and_writes_go_to_caldav() {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection("/alex/family/", Collection::events("Family"));
    dav.seed("/alex/family/", calendar_contract::ALEX_ICS);
    let env = json!([
        { "url": "https://ics.example/secret/sam.ics", "people": ["sam"] },
        { "type": "caldav", "id": "family", "url": "https://alex:secret-pw@dav.example/alex/family/", "people": ["alex"] }
    ])
    .to_string();
    let source = mixed::from_env_value(
        Some(&env),
        &settings(json!({ "calendar": { "writeTo": [ { "people": "all", "calendar": "family" } ] } })),
        Arc::new(OneFeed(calendar_contract::SAM_ICS.into())),
        dav.clone(),
        clock(calendar_contract::now()),
    );
    assert!(source.configured() && source.can_write());
    let l = source.list(calendar_contract::range()).await;
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    let dinner: Vec<_> = l.events.iter().filter(|e| e.title == "Dinner with friends").collect();
    assert_eq!(
        dinner.len(),
        1,
        "the invite on the ICS feed and the CalDAV calendar is one event"
    );
    assert_eq!(dinner[0].people, ["sam", "alex"]);
    assert!(l.events.iter().any(|e| e.title == "Gym"));

    // Sam has no calendar of their own that takes writes: the writeTo rule for "all" applies.
    source.add(haircut("mix1", &["sam"])).await.unwrap();
    assert!(dav.object("/alex/family/", "mix1.ics").is_some());
    source.remove("mix1").await.unwrap();
    assert!(dav.object("/alex/family/", "mix1.ics").is_none());
}

#[tokio::test]
async fn only_ics_stays_read_only() {
    let env = json!([{ "url": "https://ics.example/secret/sam.ics", "people": ["sam"] }]).to_string();
    let source = mixed::from_env_value(
        Some(&env),
        &settings(json!({})),
        Arc::new(OneFeed(calendar_contract::SAM_ICS.into())),
        Arc::new(FakeDav::new(None)),
        clock(calendar_contract::now()),
    );
    assert!(source.configured());
    assert!(!source.can_write());
}

// ------------------------------------------------------------------ the API

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let req = req
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn api(source: Arc<dyn CalendarSource>) -> axum::Router {
    let state =
        home_hq::AppState::new(settings(json!({})), None, clock(calendar_contract::now())).with_calendar(source);
    home_hq::router(state)
}

fn two_person_dav(broken_writes: bool) -> (Arc<FakeDav>, Arc<dyn CalendarSource>) {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection(
        "/alex/family/",
        Collection {
            broken_writes,
            ..Collection::events("Family")
        },
    );
    let source = CalDavSource::new(
        vec![config(&FakeDav::url("/alex/family/"), People::One("all".into()), None)],
        &people(&["alex", "sam"]),
        Vec::new(),
        chrono_tz::Europe::Lisbon,
        dav.clone(),
        clock(calendar_contract::now()),
    );
    (dav, Arc::new(source))
}

fn post_body(id: &str) -> Value {
    json!({ "id": id, "title": "Haircut", "people": ["sam"],
            "start": utc(2026, 11, 8, 10, 0).timestamp_millis(), "end": utc(2026, 11, 8, 11, 0).timestamp_millis() })
}

#[tokio::test]
async fn api_adds_and_removes_on_caldav_and_reports_the_capability() {
    let (dav, source) = two_person_dav(false);
    let app = api(source);
    let (_, state) = call(&app, "GET", "/api/state", None).await;
    assert_eq!(state["capabilities"], json!({ "addEvents": true }));

    let (status, body) = call(&app, "POST", "/api/events", Some(post_body("api1"))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "ok": true })));
    assert!(dav.object("/alex/family/", "api1.ics").is_some());
    let (_, cal) = call(&app, "GET", "/api/calendar", None).await;
    let e = cal["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "api1")
        .unwrap();
    assert_eq!(e["local"], true);
    assert_eq!(e["people"], json!(["sam"]));

    let (status, _) = call(&app, "DELETE", "/api/events/api1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(dav.object("/alex/family/", "api1.ics").is_none());
    // Gone now.
    assert_eq!(
        call(&app, "DELETE", "/api/events/api1", None).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn api_server_errors_are_a_bad_gateway_with_a_message() {
    let (_dav, source) = two_person_dav(true);
    let app = api(source);
    let (status, body) = call(&app, "POST", "/api/events", Some(post_body("api2"))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    // The frame shows "Couldn't save · <error>".
    assert_eq!(body, json!({ "error": "calendar server error (HTTP 500)" }));
    let (status, cal) = call(&app, "GET", "/api/calendar", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(cal["errors"].as_array().unwrap().is_empty());

    let (status, _) = call(
        &app,
        "POST",
        "/api/events",
        Some(json!({ "id": "x", "title": "No times" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn api_hides_add_for_read_only_calendars() {
    let env = json!([{ "url": "https://ics.example/secret/sam.ics", "people": ["sam"] }]).to_string();
    let source = mixed::from_env_value(
        Some(&env),
        &settings(json!({})),
        Arc::new(OneFeed(calendar_contract::SAM_ICS.into())),
        Arc::new(FakeDav::new(None)),
        clock(calendar_contract::now()),
    );
    let app = api(source);
    let (_, state) = call(&app, "GET", "/api/state", None).await;
    assert_eq!(state["capabilities"], json!({ "addEvents": false }));
    // The demo calendar (nothing configured) takes writes.
    let app = api(Arc::new(home_hq::calendar::NotConfigured));
    let (_, state) = call(&app, "GET", "/api/state", None).await;
    assert_eq!(state["capabilities"], json!({ "addEvents": true }));
}

// ------------------------------------------------------------------ recorded from Radicale

const RECORDING: &str = "tests/fixtures/calendar/caldav/radicale-scenario.json";

/// A second client (a phone) that sets up and edits the calendars behind the adapter's back.
/// Not part of the recording.
struct Phone {
    transport: Arc<dyn Transport>,
    base: String,
}

impl Phone {
    async fn send(
        &self,
        method: &'static str,
        path: &str,
        headers: Vec<(&'static str, String)>,
        body: Option<String>,
    ) -> u16 {
        let mut headers = headers;
        headers.push(("Authorization", AUTH.into()));
        self.transport
            .send(HttpRequest {
                method,
                url: format!("{}{path}", self.base),
                headers,
                body,
            })
            .await
            .unwrap()
            .status
    }

    async fn put(&self, path: &str, ics: &str) {
        let s = self
            .send(
                "PUT",
                path,
                vec![("Content-Type", "text/calendar; charset=utf-8".into())],
                Some(ics.into()),
            )
            .await;
        assert!((200..300).contains(&s), "PUT {path}: {s}");
    }

    async fn reset(&self) {
        for (name, comp) in [("personal", "VEVENT"), ("family", "VEVENT"), ("tasks", "VTODO")] {
            self.send("DELETE", &format!("/alex/{name}/"), vec![], None).await;
            let body = format!(
                r#"<?xml version="1.0"?><C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:set><D:prop><D:displayname>{name}</D:displayname><C:supported-calendar-component-set><C:comp name="{comp}"/></C:supported-calendar-component-set></D:prop></D:set></C:mkcalendar>"#
            );
            let s = self
                .send("MKCALENDAR", &format!("/alex/{name}/"), vec![], Some(body))
                .await;
            assert_eq!(s, 201, "MKCALENDAR {name}");
        }
    }
}

/// Answers every request with 201, for the phone during a replay.
struct Accept;

#[async_trait]
impl Transport for Accept {
    async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 201,
            etag: None,
            body: String::new(),
        })
    }
}

/// Records what passes through.
struct Recorder {
    inner: Arc<dyn Transport>,
    log: Mutex<Vec<Exchange>>,
}

#[async_trait]
impl Transport for Recorder {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let res = self.inner.send(req.clone()).await?;
        self.log.lock().unwrap().push(Exchange::of(&req, &res));
        Ok(res)
    }
}

const DENTIST: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Phone//EN\r\nBEGIN:VEVENT\r\nUID:dentist@phone\r\nDTSTAMP:20261101T090000Z\r\nSUMMARY:Dentist\r\nLOCATION:Rua Augusta 10\r\nDTSTART;TZID=Europe/Lisbon:20261106T123000\r\nDTEND;TZID=Europe/Lisbon:20261106T133000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
const WEEKLY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Phone//EN\r\nBEGIN:VEVENT\r\nUID:dinner@phone\r\nDTSTAMP:20261101T090000Z\r\nSUMMARY:Family dinner\r\nDTSTART;TZID=Europe/Lisbon:20261020T200000\r\nDTEND;TZID=Europe/Lisbon:20261020T210000\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nEXDATE;TZID=Europe/Lisbon:20261110T200000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Reading, adding, a conflicting remove, removing, and discovery, against a real server
/// (live) or its recording (replay).
async fn scenario(transport: Arc<dyn Transport>, phone: Phone) {
    let base = phone.base.clone();
    phone.reset().await;
    phone.put("/alex/personal/dentist.ics", DENTIST).await;
    phone.put("/alex/family/dinner.ics", WEEKLY).await;

    let house = people(HOUSE);
    let source = CalDavSource::new(
        vec![
            config(
                &format!("{base}/alex/personal/"),
                People::One("alex".into()),
                Some("alex"),
            ),
            config(
                &format!("{base}/alex/family/"),
                People::One("all".into()),
                Some("family"),
            ),
        ],
        &house,
        Vec::new(),
        chrono_tz::Europe::Lisbon,
        transport.clone(),
        clock(calendar_contract::now()),
    )
    .with_ttl(Duration::zero());
    let range = calendar_contract::range();

    let l = source.list(range).await;
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    let dentist: Vec<_> = l.events.iter().filter(|e| e.title == "Dentist").collect();
    assert_eq!(dentist.len(), 1);
    assert_eq!(dentist[0].people, ["alex"]);
    assert_eq!(
        dentist[0].when,
        home_hq::calendar::When::Timed {
            start: utc(2026, 11, 6, 12, 30),
            end: utc(2026, 11, 6, 13, 30)
        }
    );
    let dinners: Vec<_> = l.events.iter().filter(|e| e.title == "Family dinner").collect();
    // Tuesdays from the 3rd (the range starts on the 4th, so the 10th is skipped and then 17th, 24th).
    let starts: Vec<_> = dinners
        .iter()
        .map(|e| e.when.instants(chrono_tz::Europe::Lisbon).0)
        .collect();
    assert_eq!(starts, [utc(2026, 11, 17, 20, 0), utc(2026, 11, 24, 20, 0)]);
    assert_eq!(dinners[0].people, house);
    assert!(source.can_write(), "Radicale grants the owner write");

    let added = source.add(haircut("rad1", &["alex", "sam"])).await.unwrap();
    assert_eq!(added.id, "rad1");
    let l = source.list(range).await;
    let e = l.events.iter().find(|e| e.id == "rad1").expect("added event is listed");
    assert_eq!((e.title.as_str(), e.local), ("Haircut", true));
    assert_eq!(e.people, ["alex", "sam"]);

    // A phone moves the event: the frame's remove must not delete the newer version.
    let moved = vevent_with_title(&haircut("rad1", &["alex", "sam"]), "Haircut (moved)");
    phone.put("/alex/family/rad1.ics", &moved).await;
    let err = source.remove("rad1").await.unwrap_err();
    assert_eq!(
        err,
        CalendarError::Backend("the event changed on the calendar server, try again".into())
    );
    let l = source.list(range).await;
    assert!(l.events.iter().any(|e| e.id == "rad1" && e.title == "Haircut (moved)"));

    // With the fresh ETag it goes.
    source.remove("rad1").await.unwrap();
    let l = source.list(range).await;
    assert!(!l.events.iter().any(|e| e.id == "rad1"));
    assert_eq!(source.remove("rad1").await, Err(CalendarError::NotFound));

    // Discovery: the account URL has two event calendars (the task list doesn't count), and a
    // task list is no calendar for events.
    let odd = CalDavSource::new(
        vec![
            config(&format!("{base}/alex/"), People::One("all".into()), None),
            config(&format!("{base}/alex/tasks/"), People::One("all".into()), None),
        ],
        &house,
        Vec::new(),
        chrono_tz::Europe::Lisbon,
        transport,
        clock(calendar_contract::now()),
    );
    let l = odd.list(range).await;
    assert_eq!(
        l.errors,
        [
            "Calendar 1: the URL is not a calendar but has 2 calendars under it: use one of their URLs",
            "Calendar 2: the calendar doesn't hold events"
        ]
    );
}

fn vevent_with_title(e: &NewEvent, title: &str) -> String {
    home_hq::calendar::caldav::vevent::serialize(
        &NewEvent {
            title: title.into(),
            ..e.clone()
        },
        utc(2026, 11, 5, 17, 45),
    )
}

#[tokio::test]
async fn replays_the_radicale_recording() {
    let text = std::fs::read_to_string(RECORDING).unwrap();
    let exchanges: Vec<Exchange> = serde_json::from_str(&text).unwrap();
    let replay = Arc::new(Replay::new(exchanges));
    scenario(
        replay.clone(),
        Phone {
            transport: Arc::new(Accept),
            base: caldav_fake::ORIGIN.into(),
        },
    )
    .await;
    assert_eq!(replay.unused(), 0, "every recorded exchange is used");
}

/// Against a real Radicale (see the top of this file). Re-records the fixture with `HQ_RECORD=1`.
#[tokio::test]
#[ignore = "needs a Radicale server: HQ_TEST_RADICALE=http://127.0.0.1:5232"]
async fn radicale_live() {
    let base = std::env::var("HQ_TEST_RADICALE").expect("HQ_TEST_RADICALE");
    home_hq::tls::install_crypto_provider();
    let http: Arc<dyn Transport> = Arc::new(home_hq::calendar::caldav::http::Http::new().unwrap());

    // The shared contract, on real collections.
    calendar_contract::run(|seed: Seed| {
        let http = http.clone();
        let base = base.clone();
        async move {
            let phone = Phone {
                transport: http.clone(),
                base: base.clone(),
            };
            let mut configs = Vec::new();
            for (i, c) in seed.calendars.iter().enumerate() {
                let path = format!("/alex/contract{i}/");
                phone.send("DELETE", &path, vec![], None).await;
                if !c.broken {
                    assert_eq!(phone.send("MKCALENDAR", &path, vec![], None).await, 201);
                    for (uid, obj) in caldav_fake::split_objects(&c.ics) {
                        phone.put(&format!("{path}{}.ics", uid.replace('@', "-")), &obj).await;
                    }
                }
                configs.push(config(&format!("{base}{path}"), People::Many(c.people.clone()), None));
            }
            let source = CalDavSource::new(configs, &seed.people, Vec::new(), seed.tz, http, clock(seed.now));
            Harness {
                source: Arc::new(source),
                secrets: vec![base.clone(), PASSWORD.into()],
            }
        }
    })
    .await;
    let phone = Phone {
        transport: http.clone(),
        base: base.clone(),
    };
    for i in 0..2 {
        phone.send("DELETE", &format!("/alex/contract{i}/"), vec![], None).await;
    }

    let recorder = Arc::new(Recorder {
        inner: http.clone(),
        log: Mutex::new(Vec::new()),
    });
    scenario(
        recorder.clone(),
        Phone {
            transport: http.clone(),
            base: base.clone(),
        },
    )
    .await;
    if std::env::var_os("HQ_RECORD").is_some() {
        let log = recorder.log.lock().unwrap().clone();
        let json = serde_json::to_string_pretty(&log).unwrap() + "\n";
        std::fs::create_dir_all("tests/fixtures/calendar/caldav").unwrap();
        std::fs::write(RECORDING, json).unwrap();
    }
}
