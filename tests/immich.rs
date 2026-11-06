use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, Request, StatusCode},
    routing::{get, post, put},
};
use home_hq::{
    AppState,
    immich::Immich,
    router,
    settings::{EXAMPLE, Settings},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct Mock {
    searches: Arc<AtomicUsize>,
    favorite: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
}
async fn search(State(mock): State<Mock>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    assert_eq!(headers["x-api-key"], "test-key");
    assert_eq!(body["isFavorite"], true);
    assert_eq!(body["type"], "IMAGE");
    assert_eq!(body["withExif"], true);
    mock.searches.fetch_add(1, Ordering::SeqCst);
    if mock.fail.load(Ordering::SeqCst) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"secret upstream detail"})),
        );
    }
    let items = if body["page"] == 1 && mock.favorite.load(Ordering::SeqCst) {
        json!([{"id":"asset-1","type":"IMAGE","isFavorite":true,"localDateTime":"2020-06-12T12:00:00","exifInfo":{"exifImageHeight":2000,"exifImageWidth":1000,"city":"Lisbon"}}, {"id":"video-1","type":"VIDEO","isFavorite":true}, {"id":"other-1","type":"IMAGE","isFavorite":false}])
    } else {
        json!([])
    };
    (
        StatusCode::OK,
        Json(json!({"assets":{"items":items,"nextPage":if body["page"]==1 {json!("2")} else {Value::Null}}})),
    )
}
async fn thumbnail(
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ([(&'static str, &'static str); 1], Vec<u8>) {
    assert_eq!(headers["x-api-key"], "test-key");
    let bytes = match q["size"].as_str() {
        "thumbnail" => b"small".to_vec(),
        "preview" => b"large".to_vec(),
        _ => panic!("wrong size"),
    };
    ([("content-type", "image/jpeg")], bytes)
}
async fn update(State(mock): State<Mock>, headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(headers["x-api-key"], "test-key");
    mock.favorite
        .store(body["isFavorite"].as_bool().unwrap(), Ordering::SeqCst);
    Json(json!({"id":"asset-1"}))
}
async fn fixture() -> (Router, Mock, tokio::task::JoinHandle<()>) {
    let mock = Mock::default();
    mock.favorite.store(true, Ordering::SeqCst);
    let upstream = Router::new()
        .route("/api/search/metadata", post(search))
        .route("/api/assets/asset-1/thumbnail", get(thumbnail))
        .route("/api/assets/asset-1", put(update))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let mut state = AppState::new(Settings::parse(EXAMPLE).unwrap(), None, Arc::new(chrono::Utc::now));
    state.immich = Some(Arc::new(Immich::new(&url, "test-key").unwrap()));
    (router(state), mock, task)
}
async fn request(app: &Router, method: &str, path: &str, body: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = res.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes().to_vec(),
    )
}
#[tokio::test]
async fn favorites_are_paginated_cached_proxied_and_updated() {
    let (app, mock, task) = fixture().await;
    let (status, _, bytes) = request(&app, "GET", "/api/state", "").await;
    assert_eq!(status, StatusCode::OK);
    let state: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        state["photos"],
        json!([{"id":"asset-1","o":"p","date":"2020-06-12","place":"Lisbon","from":null,"fav":true,"tag":"2020-06-12"}])
    );
    for (path, expected) in [("/api/photos/asset-1?s=1", b"small"), ("/api/photos/asset-1", b"large")] {
        let (status, headers, bytes) = request(&app, "GET", path, "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "image/jpeg");
        assert_eq!(bytes, expected);
        assert!(!headers.contains_key("x-api-key"));
    }
    assert_eq!(mock.searches.load(Ordering::SeqCst), 2);
    assert_eq!(
        request(&app, "GET", "/api/photos/unknown", "").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "PATCH", "/api/photos/asset-1", r#"{"fav":false}"#)
            .await
            .0,
        StatusCode::OK
    );
    let (_, _, bytes) = request(&app, "GET", "/api/state", "").await;
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["photos"], json!([]));
    assert!(!mock.favorite.load(Ordering::SeqCst));
    task.abort();
}
#[tokio::test]
async fn upstream_failure_is_sanitized_and_never_returns_demo_photos() {
    let (app, mock, task) = fixture().await;
    mock.fail.store(true, Ordering::SeqCst);
    let (status, _, bytes) = request(&app, "GET", "/api/state", "").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!({"error":"photo source unavailable"})
    );
    assert_eq!(
        request(&app, "PATCH", "/api/photos/asset-1", r#"{"fav":"no"}"#).await.0,
        StatusCode::BAD_REQUEST
    );
    task.abort();
}
#[test]
fn rejects_unsafe_configuration() {
    for url in [
        "file:///tmp/immich",
        "https://user:pass@example.org/api",
        "https://example.org/api?token=x",
        "https://example.org/api#fragment",
    ] {
        assert!(Immich::new(url, "key").is_err());
    }
    assert!(Immich::new("https://example.org/api", "").is_err());
}
