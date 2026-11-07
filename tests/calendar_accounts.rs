//! The calendar page's backend: connecting CalDAV accounts (Login Flow v2 or an app password),
//! ICS links, per-calendar choices, encrypted persistence, and `/api/calendars`. No network:
//! an in-memory CalDAV server answers.

#[path = "support/caldav_fake.rs"]
mod caldav_fake;
#[path = "support/calendar_contract.rs"]
mod calendar_contract;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use caldav_fake::{Collection, FakeDav};
use chrono::{TimeZone, Utc};
use home_hq::calendar::NewEvent;
use home_hq::calendar::accounts::{Config, Manager, Patch};
use home_hq::calendar::fetch::{Fetch, Response};
use home_hq::settings::Settings;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const AUTH: &str = "Basic YWxleDpzZWNyZXQtcHc="; // alex:secret-pw

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

fn settings() -> Settings {
    Settings::parse(
        r##"{ "timezone": "Europe/Lisbon", "people": [
            { "id": "alex", "name": "Alex", "color": "#2D5BFF" },
            { "id": "sam", "name": "Sam", "color": "#E4007C" } ] }"##,
    )
    .unwrap()
}

/// Alex's account: `/alex/personal/` ("Alex"), `/alex/family/` ("Family") and a task list.
fn server() -> Arc<FakeDav> {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection("/alex/personal/", Collection::events("Alex"));
    dav.add_collection("/alex/family/", Collection::events("Family"));
    dav.add_collection(
        "/alex/tasks/",
        Collection {
            components: vec!["VTODO"],
            ..Collection::events("Tasks")
        },
    );
    dav.seed("/alex/personal/", calendar_contract::ALEX_ICS);
    dav
}

fn tmp() -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "home-hq-accounts-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn manager(dav: Arc<FakeDav>, dir: Option<PathBuf>, key: Option<&str>, allowed: Option<&str>) -> Manager {
    let now = calendar_contract::now();
    Manager::new(Config {
        settings: settings(),
        env: None,
        fetch: Arc::new(OneFeed(calendar_contract::SAM_ICS.into())),
        transport: dav,
        clock: Arc::new(move || now),
        state_dir: dir,
        hq_key: key.map(String::from),
        allowed_servers: allowed.map(String::from),
    })
}

async fn connect(m: &Manager, dav: &FakeDav) -> Value {
    let login = m.start_login("https://dav.example").await.unwrap();
    assert_eq!(login["url"], "https://dav.example/index.php/login/v2/flow/f1");
    assert!(m.login_qr().await.unwrap().contains("<svg"));
    assert_eq!(m.poll_login().await.unwrap()["status"], "waiting");
    dav.approve();
    let done = m.poll_login().await.unwrap();
    assert_eq!(done["status"], "connected", "{done}");
    done["account"].clone()
}

fn haircut(who: &[&str]) -> NewEvent {
    NewEvent {
        id: "page1".into(),
        title: "Haircut".into(),
        people: who.iter().map(|s| s.to_string()).collect(),
        start: Utc.with_ymd_and_hms(2026, 11, 8, 10, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 11, 8, 11, 0, 0).unwrap(),
    }
}

#[tokio::test]
async fn connects_with_the_login_flow_and_lists_event_calendars() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    assert!(!m.source().configured(), "nothing yet: the demo calendar");
    let account = connect(&m, &dav).await;

    assert_eq!(account["name"], "dav.example");
    assert_eq!(account["username"], "alex");
    assert_eq!(account["via"], "login flow");
    let cals = account["calendars"].as_array().unwrap();
    let names: Vec<_> = cals.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Alex", "Family"], "the task list isn't offered");
    assert_eq!(cals[0]["people"], json!(["alex"]), "named after a person");
    assert_eq!(cals[1]["people"], json!(["all"]));
    assert_eq!(cals[0]["show"], true);
    assert_eq!(cals[0]["addEvents"], false, "the frame doesn't write until allowed");
    assert!(!account.to_string().contains("secret-pw"));

    // Discovery went through /.well-known/caldav with the new app password.
    let finds = dav.requests("PROPFIND");
    assert_eq!(finds[0].url, "https://dav.example/.well-known/caldav");
    assert!(finds.iter().all(|r| r.header("Authorization") == Some(AUTH)));

    let source = m.source();
    assert!(source.configured());
    assert!(!source.can_write());
    let l = source.list(calendar_contract::range()).await;
    assert!(l.errors.is_empty(), "{:?}", l.errors);
    let dentist = l.events.iter().find(|e| e.title == "Dentist").unwrap();
    assert_eq!(dentist.people, ["alex"]);
}

#[tokio::test]
async fn the_frame_writes_only_where_allowed() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    let account = connect(&m, &dav).await;
    let (a, family) = (
        account["id"].as_str().unwrap(),
        account["calendars"][1]["id"].as_str().unwrap(),
    );
    assert!(m.source().add(haircut(&["sam"])).await.is_err());
    m.update(
        a,
        family,
        Patch {
            add_events: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let source = m.source();
    assert!(source.can_write());
    source.add(haircut(&["sam"])).await.unwrap();
    assert!(dav.object("/alex/family/", "page1.ics").is_some());
    source.remove("page1").await.unwrap();
    assert!(dav.object("/alex/family/", "page1.ics").is_none());
}

#[tokio::test]
async fn read_only_calendars_cannot_be_made_writable() {
    let dav = Arc::new(FakeDav::new(Some(AUTH)));
    dav.add_collection(
        "/alex/shared/",
        Collection {
            writable: false,
            ..Collection::events("Holidays")
        },
    );
    let m = manager(dav.clone(), None, None, None);
    let account = connect(&m, &dav).await;
    let (a, c) = (
        account["id"].as_str().unwrap(),
        account["calendars"][0]["id"].as_str().unwrap(),
    );
    assert_eq!(account["calendars"][0]["serverWritable"], false);
    let err = m
        .update(
            a,
            c,
            Patch {
                add_events: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.status, 400);
    let err = m
        .update(
            a,
            c,
            Patch {
                people: Some(vec!["nobody".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.message, "unknown person nobody");
}

#[tokio::test]
async fn adds_with_an_app_password_and_rejects_wrong_ones() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    let err = m
        .add_with_password("https://dav.example/alex/", "alex", "wrong")
        .await
        .unwrap_err();
    assert_eq!(err.message, "the server refused that user name and app password");
    let account = m
        .add_with_password("https://dav.example/alex/", "alex", "secret-pw")
        .await
        .unwrap();
    assert_eq!(account["via"], "app password");
    assert_eq!(account["calendars"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn servers_are_checked() {
    let dav = server();
    let m = manager(dav.clone(), None, None, Some("https://dav.example"));
    for bad in ["ftp://dav.example", "https://alex:pw@dav.example", "https://"] {
        assert_eq!(m.start_login(bad).await.unwrap_err().status, 400, "{bad}");
    }
    assert_eq!(
        m.start_login("https://elsewhere.example").await.unwrap_err().status,
        403,
        "not in HQ_CALENDAR_SERVERS"
    );
    // A bare host means https.
    assert!(m.start_login("dav.example").await.is_ok());
}

#[tokio::test]
async fn servers_without_the_login_flow_point_to_app_passwords() {
    struct NoFlow;
    #[async_trait]
    impl home_hq::calendar::caldav::dav::Transport for NoFlow {
        async fn send(
            &self,
            _req: home_hq::calendar::caldav::dav::HttpRequest,
        ) -> Result<home_hq::calendar::caldav::dav::HttpResponse, String> {
            Ok(home_hq::calendar::caldav::dav::HttpResponse {
                status: 404,
                ..Default::default()
            })
        }
    }
    let now = calendar_contract::now();
    let m = Manager::new(Config {
        settings: settings(),
        env: None,
        fetch: Arc::new(OneFeed(String::new())),
        transport: Arc::new(NoFlow),
        clock: Arc::new(move || now),
        state_dir: None,
        hq_key: None,
        allowed_servers: None,
    });
    let err = m.start_login("https://radicale.example").await.unwrap_err();
    assert!(err.message.contains("app password"), "{}", err.message);
}

#[tokio::test]
async fn ics_links_are_added_with_their_people_and_names() {
    let m = manager(server(), None, None, None);
    let err = m.add_ics("https://calendar.example/", "", &[]).await.unwrap_err();
    assert_eq!(err.status, 400, "a server, not a link");
    let a = m
        .add_ics(
            "webcal://calendar.example/secret-token/basic.ics",
            "Sam's",
            &["sam".into()],
        )
        .await
        .unwrap();
    assert!(!a.to_string().contains("secret-token"));
    assert_eq!(a["calendars"][0]["people"], json!(["sam"]));
    let l = m.source().list(calendar_contract::range()).await;
    assert!(l.events.iter().any(|e| e.title == "Gym" && e.people == ["sam"]));
}

#[tokio::test]
async fn refresh_keeps_the_choices_and_finds_new_calendars() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    let account = connect(&m, &dav).await;
    let (a, family) = (
        account["id"].as_str().unwrap().to_string(),
        account["calendars"][1]["id"].as_str().unwrap().to_string(),
    );
    m.update(
        &a,
        &family,
        Patch {
            people: Some(vec!["sam".into()]),
            show: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    dav.add_collection("/alex/work/", Collection::events("Work"));
    let after = m.refresh(&a).await.unwrap();
    let cals = after["calendars"].as_array().unwrap();
    assert_eq!(cals.len(), 3);
    let fam = cals.iter().find(|c| c["name"] == "Family").unwrap();
    assert_eq!(fam["id"], family.as_str());
    assert_eq!(fam["people"], json!(["sam"]));
    assert_eq!(fam["show"], false);
}

#[tokio::test]
async fn accounts_persist_encrypted_with_the_dashboard_key() {
    let dav = server();
    let dir = tmp();
    let m = manager(dav.clone(), Some(dir.clone()), Some("hq-key"), None);
    assert_eq!(m.view().await["persisted"], true);
    connect(&m, &dav).await;
    let raw = std::fs::read(dir.join("calendars.bin")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("secret-pw"));
    assert!(!String::from_utf8_lossy(&raw).contains("dav.example"));

    // A restart loads them again.
    let again = manager(dav.clone(), Some(dir.clone()), Some("hq-key"), None);
    assert_eq!(again.view().await["accounts"].as_array().unwrap().len(), 1);
    assert!(again.source().configured());

    // Another key can't read them: they're set aside, not lost.
    let other = manager(dav.clone(), Some(dir.clone()), Some("other-key"), None);
    assert!(other.view().await["accounts"].as_array().unwrap().is_empty());
    assert!(dir.join("calendars.unreadable").exists());

    // Without a key nothing is written.
    let dir2 = tmp();
    let open = manager(dav.clone(), Some(dir2.clone()), None, None);
    assert_eq!(open.view().await["persisted"], false);
    connect(&open, &dav).await;
    assert!(!dir2.join("calendars.bin").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[tokio::test]
async fn removing_an_account_forgets_it() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    let account = connect(&m, &dav).await;
    m.remove(account["id"].as_str().unwrap()).await.unwrap();
    assert!(!m.source().configured());
    assert_eq!(m.remove("nope").await.unwrap_err().status, 404);
}

#[tokio::test]
async fn an_expired_login_says_so() {
    let dav = server();
    let m = manager(dav.clone(), None, None, None);
    m.start_login("https://dav.example").await.unwrap();
    *dav.login.lock().unwrap() = None; // the server forgot it
    assert_eq!(m.poll_login().await.unwrap()["status"], "expired");
    assert_eq!(m.view().await["outcome"]["status"], "expired");
    assert_eq!(m.poll_login().await.unwrap()["status"], "none");
}

// ------------------------------------------------------------------ the API

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let res = app
        .clone()
        .oneshot(
            req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, ct, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap()
}

#[tokio::test]
async fn the_api_connects_configures_and_never_returns_secrets() {
    let dav = server();
    let m = Arc::new(manager(dav.clone(), None, None, None));
    let now = calendar_contract::now();
    let app = home_hq::router(
        home_hq::AppState::new(settings(), None, Arc::new(move || now)).with_calendar_accounts(m.clone()),
    );

    let (s, _, b) = call(&app, "GET", "/api/calendars", None).await;
    assert_eq!(s, StatusCode::OK);
    let v = json_of(&b);
    assert_eq!(v["accounts"], json!([]));
    assert_eq!(v["people"][0]["name"], "Alex");
    assert!(v["persistNote"].as_str().unwrap().contains("HQ_STATE_DIR"));

    let (s, _, b) = call(
        &app,
        "POST",
        "/api/calendars/login",
        Some(json!({ "server": "https://dav.example" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let (s, ct, _) = call(&app, "GET", "/api/calendars/login/qr", None).await;
    assert_eq!((s, ct.as_str()), (StatusCode::OK, "image/svg+xml"));
    dav.approve();
    let (_, _, b) = call(&app, "POST", "/api/calendars/login/poll", None).await;
    let account = json_of(&b)["account"].clone();
    let (a, family) = (
        account["id"].as_str().unwrap(),
        account["calendars"][1]["id"].as_str().unwrap(),
    );

    let (s, _, b) = call(
        &app,
        "PATCH",
        &format!("/api/calendars/{a}/{family}"),
        Some(json!({ "addEvents": true, "people": "all" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    let (_, _, b) = call(&app, "GET", "/api/state", None).await;
    assert_eq!(json_of(&b)["capabilities"]["addEvents"], true);

    let (_, _, b) = call(
        &app,
        "POST",
        "/api/calendars/ics",
        Some(json!({ "url": "https://ics.example/private-token/basic.ics", "name": "Sam", "people": ["sam"] })),
    )
    .await;
    assert!(!String::from_utf8_lossy(&b).contains("private-token"));
    let (_, _, b) = call(&app, "GET", "/api/calendars", None).await;
    let all = String::from_utf8_lossy(&b).to_string();
    assert!(!all.contains("secret-pw") && !all.contains("private-token"), "{all}");

    // The frame's calendar now reads both, and the event lands in Family.
    let (s, _, b) = call(
        &app,
        "POST",
        "/api/events",
        Some(json!({ "id": "api9", "title": "Haircut", "people": ["sam"],
                     "start": 1794132000000i64, "end": 1794135600000i64 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    assert!(dav.object("/alex/family/", "api9.ics").is_some());

    let (s, _, _) = call(&app, "DELETE", &format!("/api/calendars/{a}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, b) = call(&app, "GET", "/api/calendars", None).await;
    assert_eq!(
        json_of(&b)["accounts"].as_array().unwrap().len(),
        1,
        "the ICS link stays"
    );
}

#[tokio::test]
async fn without_a_manager_the_api_says_unavailable() {
    let now = calendar_contract::now();
    let app = home_hq::router(home_hq::AppState::new(settings(), None, Arc::new(move || now)));
    let (s, _, _) = call(&app, "GET", "/api/calendars", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
