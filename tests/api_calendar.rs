//! `/api/calendar` and `/api/events` with a configured `CalendarSource`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{DateTime, TimeZone, Utc};
use home_hq::calendar::fetch::{Fetch, Response};
use home_hq::calendar::ics::IcsSource;
use home_hq::calendar::{CalendarError, CalendarSource, Event, Listing, NewEvent, Range, When};
use home_hq::{
    AppState, router,
    settings::{EXAMPLE, Settings},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap()
}

fn app(source: Arc<dyn CalendarSource>) -> Router {
    let state = AppState::new(Settings::parse(EXAMPLE).unwrap(), None, Arc::new(now)).with_calendar(source);
    router(state)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::delete(uri).body(Body::empty()).unwrap()
}

struct Feed(&'static str);

#[async_trait]
impl Fetch for Feed {
    async fn get(&self, _url: &str) -> Result<Response, String> {
        Ok(Response {
            status: 200,
            body: self.0.into(),
        })
    }
}

const FEED: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a@test\r\nSUMMARY:Dentist\r\nDTSTART:20261106T123000Z\r\nDTEND:20261106T133000Z\r\nLOCATION:Rua Augusta\r\nDESCRIPTION:Travel: 20\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b@test\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261120\r\nDTEND;VALUE=DATE:20261127\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn ics() -> Arc<dyn CalendarSource> {
    let settings = Settings::parse(EXAMPLE).unwrap();
    let people: Vec<String> = settings.people.iter().map(|p| p.id.clone()).collect();
    let first = people[0].clone();
    let value = json!([{ "url": "https://calendar.example/secret/basic.ics", "people": [first] }]).to_string();
    Arc::new(
        IcsSource::from_env_value(
            Some(&value),
            &people,
            settings.tz().unwrap(),
            Arc::new(Feed(FEED)),
            Arc::new(now),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn calendar_serves_the_feeds_in_upstreams_shape() {
    let settings = Settings::parse(EXAMPLE).unwrap();
    let first = settings.people[0].id.clone();
    let (status, v) = send(&app(ics()), get("/api/calendar")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["errors"], json!([]));
    assert_eq!(v["checked"], now().timestamp_millis());
    assert_eq!(
        v["events"],
        json!([
            { "id": v["events"][0]["id"], "title": "Dentist", "people": [first],
              "start": "2026-11-06T12:30:00.000Z", "end": "2026-11-06T13:30:00.000Z",
              "loc": "Rua Augusta", "notes": "Travel: 20", "travel": 20 },
            { "id": v["events"][1]["id"], "title": "Holiday", "people": [first], "allDay": true,
              "start": "2026-11-20", "end": "2026-11-27" },
        ])
    );
}

#[tokio::test]
async fn a_read_only_calendar_refuses_adds_and_removes() {
    let app = app(ics());
    let new = json!({ "id": "x1", "title": "Haircut", "people": ["alex"], "start": 1, "end": 2 });
    let (status, v) = send(&app, post("/api/events", new)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(v, json!({ "error": "the calendar is read-only" }));
    let (status, _) = send(&app, delete("/api/events/x1")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn without_calendars_the_demo_calendar_stays() {
    let app = app(Arc::new(home_hq::calendar::NotConfigured));
    let (status, v) = send(&app, get("/api/calendar")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["events"].as_array().unwrap().iter().any(|e| e["id"] == "demo-e0"));
    let new = json!({ "id": "x1", "title": "Haircut", "people": ["alex"], "start": 1_000, "end": 2_000 });
    let (status, _) = send(&app, post("/api/events", new)).await;
    assert_eq!(status, StatusCode::OK);
}

/// A writable source, standing in for CalDAV.
#[derive(Default)]
struct Writable {
    events: Mutex<Vec<Event>>,
}

#[async_trait]
impl CalendarSource for Writable {
    fn configured(&self) -> bool {
        true
    }
    fn can_write(&self) -> bool {
        true
    }
    async fn list(&self, _range: Range) -> Listing {
        Listing {
            events: self.events.lock().unwrap().clone(),
            errors: vec![],
            checked: now(),
        }
    }
    async fn add(&self, new: NewEvent) -> Result<Event, CalendarError> {
        if new.title == "fail" {
            return Err(CalendarError::Backend("server said no".into()));
        }
        let ev = Event {
            id: new.id.clone(),
            key: new.id,
            title: new.title,
            people: new.people,
            when: When::Timed {
                start: new.start,
                end: new.end,
            },
            location: None,
            notes: None,
            travel: None,
            walk: None,
            local: true,
        };
        self.events.lock().unwrap().push(ev.clone());
        Ok(ev)
    }
    async fn remove(&self, id: &str) -> Result<(), CalendarError> {
        let mut events = self.events.lock().unwrap();
        let before = events.len();
        events.retain(|e| e.id != id);
        if events.len() == before {
            Err(CalendarError::NotFound)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn a_writable_calendar_adds_and_removes() {
    let app = app(Arc::new(Writable::default()));
    let start = now().timestamp_millis();
    let new = json!({ "id": "x1", "title": "Haircut", "people": ["alex"], "start": start, "end": start + 3_600_000 });
    let (status, v) = send(&app, post("/api/events", new)).await;
    assert_eq!((status, v), (StatusCode::OK, json!({ "ok": true })));
    let (_, v) = send(&app, get("/api/calendar")).await;
    assert_eq!(v["events"][0]["local"], true);
    assert_eq!(v["events"][0]["start"], "2026-11-05T17:30:00.000Z");

    let (status, _) = send(&app, delete("/api/events/x1")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, v) = send(&app, delete("/api/events/x1")).await;
    assert_eq!(
        (status, v),
        (StatusCode::NOT_FOUND, json!({ "error": "no such event" }))
    );

    let bad = json!({ "id": "x2", "title": "", "people": ["alex"], "start": start, "end": start });
    let (status, _) = send(&app, post("/api/events", bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let fails = json!({ "id": "x3", "title": "fail", "people": ["alex"], "start": start, "end": start + 1 });
    let (status, v) = send(&app, post("/api/events", fails)).await;
    assert_eq!(
        (status, v),
        (StatusCode::BAD_GATEWAY, json!({ "error": "server said no" }))
    );
}
