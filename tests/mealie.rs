//! The Mealie adapter's mapping, against the fake Mealie seeded from `tests/fixtures/mealie/`,
//! and the shopping routes with Mealie behind them (including Mealie being down).

#[path = "common/fake_mealie.rs"]
mod fake_mealie;

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{TimeZone, Utc};
use fake_mealie::{FakeMealie, TOKEN};
use home_hq::{
    AppState, router,
    settings::{EXAMPLE, Settings},
    shopping::{Item, ShoppingError, ShoppingList, UNAVAILABLE_ID, mealie::Mealie},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn settings_with(mealie: Value) -> Settings {
    let mut v: Value = serde_json::from_str(EXAMPLE).unwrap();
    v["shopping"]["mealie"] = mealie;
    Settings::parse(&v.to_string()).unwrap()
}

fn settings() -> Settings {
    settings_with(json!({ "list": "Groceries", "storeLists": { "market": "Market" } }))
}

async fn seeded() -> (FakeMealie, Mealie) {
    let fake = FakeMealie::from_fixtures().await;
    let m = Mealie::new(&fake.url, TOKEN, &settings()).unwrap();
    (fake, m)
}

async fn get(m: &Mealie, id: &str) -> Item {
    m.list()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("{id} is not listed"))
}

async fn by_name(m: &Mealie, name: &str) -> Item {
    m.list()
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
}

fn mealie_item(fake: &FakeMealie, list: &str, pred: impl Fn(&Value) -> bool) -> Value {
    fake.items_of(list).into_iter().find(pred).expect("no such Mealie item")
}

// ------------------------------------------------------------------ reading

#[tokio::test]
async fn maps_mealie_items_to_the_frontends_shape() {
    let (_fake, m) = seeded().await;
    let items = m.list().await.unwrap();
    // Two lists read (Groceries for the supermarket, Market for the market); Christmas is ignored.
    assert_eq!(items.len(), 14);

    let milk = by_name(&m, "Milk").await;
    assert_eq!(
        (
            milk.cat.as_str(),
            milk.qty,
            milk.store.as_str(),
            milk.note.as_str(),
            milk.done
        ),
        ("Dairy & Eggs", 2, "grocer", "", false)
    );
    assert_eq!(
        milk.id, "1e3a0000-0000-4000-8000-000000000001",
        "a Mealie item reports its Mealie id"
    );

    // Added from the frame earlier: the frame's id and note come back from extras.
    let loaf = get(&m, "mh3k9x2a1b").await;
    assert_eq!(
        (loaf.name.as_str(), loaf.cat.as_str(), loaf.note.as_str()),
        ("Sourdough loaf", "Bakery", "Sliced, please")
    );

    // An amount with a unit isn't a count: quantity 1, the amount at the front of the note.
    let chicken = by_name(&m, "Chicken thighs").await;
    assert_eq!(
        (chicken.qty, chicken.note.as_str(), chicken.cat.as_str()),
        (1, "500 g · skin on", "Meat & Fish")
    );

    // A label that isn't a category, and no label at all, both show under Other.
    assert_eq!(by_name(&m, "Smoked paprika").await.cat, "Other");
    assert_eq!(by_name(&m, "Birthday candles").await.cat, "Other");

    assert!(by_name(&m, "Coffee beans").await.done);
    let tomatoes = by_name(&m, "Tomatoes").await;
    assert_eq!(
        (tomatoes.store.as_str(), tomatoes.qty, tomatoes.cat.as_str()),
        ("market", 4, "Produce")
    );
}

#[tokio::test]
async fn changes_made_in_mealie_show_on_the_next_read() {
    let (fake, m) = seeded().await;
    let id = fake.add_in_mealie("Market", "Lemons", Some("Produce"));
    let lemons = get(&m, &id).await;
    assert_eq!(
        (lemons.name.as_str(), lemons.store.as_str(), lemons.cat.as_str()),
        ("Lemons", "market", "Produce")
    );

    fake.edit_in_mealie(&id, |i| i["checked"] = true.into());
    assert!(get(&m, &id).await.done);

    let milk = by_name(&m, "Milk").await.id;
    fake.edit_in_mealie(&milk, |i| i["quantity"] = 3.0.into());
    assert_eq!(get(&m, &milk).await.qty, 3);
}

#[tokio::test]
async fn store_sharing_the_main_list_is_kept_in_extras() {
    // Both stores share one list: the store is remembered per item.
    let fake = FakeMealie::from_fixtures().await;
    let m = Mealie::new(&fake.url, TOKEN, &settings_with(json!({ "list": "Groceries" }))).unwrap();
    assert_eq!(
        by_name(&m, "Milk").await.store,
        "grocer",
        "items added in Mealie go to the first store"
    );
    let milk = by_name(&m, "Milk").await;
    m.put(Item {
        store: "market".into(),
        ..milk.clone()
    })
    .await
    .unwrap();
    assert_eq!(get(&m, &milk.id).await.store, "market");
    let raw = mealie_item(&fake, "Groceries", |i| i["id"] == milk.id.as_str());
    assert_eq!(raw["extras"]["homeHqStore"], "market");
    assert!(
        !m.list().await.unwrap().iter().any(|i| i.name == "Tomatoes"),
        "a list the settings don't name isn't read"
    );
}

// ------------------------------------------------------------------ writing

#[tokio::test]
async fn adding_from_the_frame_creates_a_labelled_mealie_item() {
    let (fake, m) = seeded().await;
    let item = Item {
        id: "mh9z8y7x".into(),
        name: "Lemons".into(),
        cat: "Produce".into(),
        qty: 3,
        store: "market".into(),
        note: "unwaxed".into(),
        done: false,
    };
    m.put(item.clone()).await.unwrap();
    let raw = mealie_item(&fake, "Market", |i| i["note"] == "Lemons");
    assert_eq!(raw["quantity"], 3.0);
    assert_eq!(raw["label"]["name"], "Produce");
    assert_eq!(
        raw["extras"],
        json!({ "homeHqId": "mh9z8y7x", "homeHqStore": "market", "homeHqNote": "unwaxed" })
    );
    assert_eq!(raw["position"], 4, "new items go to the end of the list");
    assert_eq!(get(&m, "mh9z8y7x").await, item);
}

#[tokio::test]
async fn categories_without_a_label_get_one() {
    let (fake, m) = seeded().await;
    m.put(Item {
        id: "n1".into(),
        name: "Crackers".into(),
        cat: "Snacks".into(),
        qty: 1,
        store: "grocer".into(),
        note: String::new(),
        done: false,
    })
    .await
    .unwrap();
    assert!(fake.data.lock().unwrap().labels.iter().any(|l| l["name"] == "Snacks"));
    assert_eq!(get(&m, "n1").await.cat, "Snacks");
}

#[tokio::test]
async fn without_permission_to_create_labels_items_save_unlabelled() {
    let (fake, m) = seeded().await;
    fake.data.lock().unwrap().can_create_labels = false;
    let crackers = Item {
        id: "n1".into(),
        name: "Crackers".into(),
        cat: "Snacks".into(),
        qty: 1,
        store: "grocer".into(),
        note: String::new(),
        done: false,
    };
    m.put(crackers).await.unwrap();
    assert_eq!(get(&m, "n1").await.cat, "Other");
}

#[tokio::test]
async fn ticking_keeps_what_the_frame_doesnt_know_about() {
    let (fake, m) = seeded().await;
    let chicken = by_name(&m, "Chicken thighs").await;
    let before = mealie_item(&fake, "Groceries", |i| i["id"] == chicken.id.as_str());
    m.put(Item {
        done: true,
        ..chicken.clone()
    })
    .await
    .unwrap();
    let after = mealie_item(&fake, "Groceries", |i| i["id"] == chicken.id.as_str());
    assert_eq!(after["checked"], true);
    for k in ["foodId", "unitId", "labelId", "note", "position"] {
        assert_eq!(after[k], before[k], "{k} changed");
    }
    assert_eq!(after["quantity"].as_f64(), Some(500.0));
    // The frame's + and − don't turn "500 g" into a count.
    m.put(Item {
        qty: 2,
        ..get(&m, &chicken.id).await
    })
    .await
    .unwrap();
    assert_eq!(
        mealie_item(&fake, "Groceries", |i| i["id"] == chicken.id.as_str())["quantity"],
        500.0
    );
}

#[tokio::test]
async fn food_items_keep_their_note_in_mealie() {
    let (fake, m) = seeded().await;
    let cod = by_name(&m, "Cod").await;
    m.put(Item {
        note: "for Saturday".into(),
        ..cod.clone()
    })
    .await
    .unwrap();
    let raw = mealie_item(&fake, "Market", |i| i["id"] == cod.id.as_str());
    assert_eq!(raw["note"], "for Saturday");
    assert_eq!(raw["food"]["name"], "Cod");
}

#[tokio::test]
async fn a_save_with_nothing_changed_writes_nothing() {
    let (fake, m) = seeded().await;
    let milk = by_name(&m, "Milk").await;
    m.put(milk).await.unwrap();
    assert!(!fake.log().iter().any(|r| r.starts_with("PUT") || r.starts_with("POST")));
}

#[tokio::test]
async fn an_item_mealie_merged_on_create_is_updated_through_its_alias() {
    let (fake, m) = seeded().await;
    // Kitchen roll (×2, unticked, plain) is already on the list: Mealie merges a second one into it.
    let new = Item {
        id: "mhnew1".into(),
        name: "Kitchen roll".into(),
        cat: "Household".into(),
        qty: 1,
        store: "grocer".into(),
        note: String::new(),
        done: false,
    };
    m.put(new.clone()).await.unwrap();
    let rolls = fake
        .items_of("Groceries")
        .into_iter()
        .filter(|i| i["note"] == "Kitchen roll")
        .count();
    assert_eq!(rolls, 1);
    // The frame's next save of its item lands on the merged one instead of adding a third roll,
    // and its count (its own share) doesn't overwrite the merged total.
    m.put(Item { done: true, ..new }).await.unwrap();
    let merged = mealie_item(&fake, "Groceries", |i| i["note"] == "Kitchen roll");
    assert_eq!(
        (merged["checked"].clone(), merged["quantity"].clone()),
        (json!(true), json!(3.0))
    );
}

#[tokio::test]
async fn clear_deletes_in_mealie_and_undo_recreates_the_same_item() {
    let (fake, m) = seeded().await;
    let coffee = by_name(&m, "Coffee beans").await;
    let milk = by_name(&m, "Milk").await;
    let before = mealie_item(&fake, "Groceries", |i| i["id"] == coffee.id.as_str());
    m.clear(vec![coffee.id.clone(), milk.id.clone()]).await.unwrap();
    assert!(!fake.items_of("Groceries").iter().any(|i| i["id"] == coffee.id.as_str()));
    assert!(
        fake.items_of("Groceries").iter().any(|i| i["id"] == milk.id.as_str()),
        "an unticked item is never cleared"
    );

    m.restore(vec![coffee.id.clone()]).await.unwrap();
    let after = mealie_item(&fake, "Groceries", |i| i["id"] == coffee.id.as_str());
    assert_eq!(after["quantity"].as_f64(), before["quantity"].as_f64());
    for k in ["id", "foodId", "labelId", "checked", "position", "extras"] {
        assert_eq!(after[k], before[k], "{k} differs after Undo");
    }
    assert_eq!(get(&m, &coffee.id).await, coffee);
}

// ------------------------------------------------------------------ failures and config

#[tokio::test]
async fn mealie_down_is_unavailable() {
    let (fake, m) = seeded().await;
    fake.set_down(true);
    assert!(matches!(m.list().await, Err(ShoppingError::Unavailable(_))));
    let milk = Item {
        id: "x".into(),
        name: "Milk".into(),
        cat: "Other".into(),
        qty: 1,
        store: "grocer".into(),
        note: String::new(),
        done: false,
    };
    assert!(matches!(m.put(milk).await, Err(ShoppingError::Unavailable(_))));
}

#[tokio::test]
async fn unreachable_mealie_is_unavailable() {
    let m = Mealie::new("http://127.0.0.1:9", TOKEN, &settings()).unwrap();
    assert_eq!(
        m.list().await,
        Err(ShoppingError::Unavailable("Can't reach Mealie".into()))
    );
}

#[tokio::test]
async fn a_bad_token_or_missing_list_is_reported() {
    let fake = FakeMealie::from_fixtures().await;
    let m = Mealie::new(&fake.url, "wrong", &settings()).unwrap();
    assert_eq!(
        m.list().await,
        Err(ShoppingError::Rejected("Mealie refused the token (HTTP 401)".into()))
    );
    let m = Mealie::new(&fake.url, TOKEN, &settings_with(json!({ "list": "Weekly shop" }))).unwrap();
    assert_eq!(
        m.list().await,
        Err(ShoppingError::Rejected(
            "Mealie has no shopping list called \u{201c}Weekly shop\u{201d}".into()
        ))
    );
}

#[test]
fn settings_must_name_the_list_and_known_stores() {
    let s = Settings::parse(EXAMPLE).unwrap();
    assert!(Mealie::new("http://mealie", TOKEN, &s).is_err());
    let s = settings_with(json!({ "list": "Groceries", "storeLists": { "nowhere": "X" } }));
    assert!(Mealie::new("http://mealie", TOKEN, &s).is_err());
}

// ------------------------------------------------------------------ routes

async fn app(fake: &FakeMealie) -> Router {
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 17, 30, 0).unwrap();
    let s = settings();
    let m = Mealie::new(&fake.url, TOKEN, &s).unwrap();
    router(AppState::new(s.clone(), None, Arc::new(move || now)).with_shopping(Arc::new(m), &s))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn state() -> Request<Body> {
    Request::get("/api/state").body(Body::empty()).unwrap()
}

#[tokio::test]
async fn routes_read_and_write_mealie() {
    let fake = FakeMealie::from_fixtures().await;
    let app = app(&fake).await;
    let (status, s) = send(&app, state()).await;
    assert_eq!(status, StatusCode::OK);
    let groceries = s["groceries"].as_array().unwrap();
    assert_eq!(groceries.len(), 14);
    assert_eq!(
        groceries[0],
        json!({ "id": "1e3a0000-0000-4000-8000-000000000001", "name": "Milk", "cat": "Dairy & Eggs",
                "qty": 2, "store": "grocer", "note": "", "done": false })
    );
    assert!(s["notes"].is_array(), "the rest of the state is still there");

    let add = json!({ "id": "mhabc", "name": "Lemons", "cat": "Produce", "qty": 1, "store": "grocer", "note": "", "done": false });
    assert_eq!(
        send(&app, post("/api/groceries", add.clone())).await,
        (StatusCode::OK, json!({ "ok": true }))
    );
    let mut ticked = add.clone();
    ticked["done"] = true.into();
    send(&app, post("/api/groceries", ticked)).await;
    assert_eq!(
        send(&app, post("/api/groceries/clear", json!({ "ids": ["mhabc"] })))
            .await
            .0,
        StatusCode::OK
    );
    assert!(!fake.items_of("Groceries").iter().any(|i| i["note"] == "Lemons"));
    send(
        &app,
        post("/api/groceries/clear", json!({ "ids": ["mhabc"], "cleared": false })),
    )
    .await;
    let (_, s) = send(&app, state()).await;
    let back: Vec<&Value> = s["groceries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["id"] == "mhabc")
        .collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0]["done"], true);

    let (status, body) = send(&app, post("/api/groceries", json!({ "id": "x" }))).await;
    assert_eq!(
        (status, body),
        (StatusCode::BAD_REQUEST, json!({ "error": "id and name required" }))
    );
}

#[tokio::test]
async fn mealie_down_shows_an_error_row_and_writes_fail() {
    let fake = FakeMealie::from_fixtures().await;
    let app = app(&fake).await;
    send(&app, state()).await; // a good read first

    fake.set_down(true);
    let (status, s) = send(&app, state()).await;
    assert_eq!(status, StatusCode::OK, "the rest of the dashboard keeps working");
    let groceries = s["groceries"].as_array().unwrap();
    assert_eq!(groceries[0]["id"], UNAVAILABLE_ID);
    assert_eq!(groceries[0]["name"], "Can't load the shopping list");
    assert_eq!(groceries[0]["done"], false);
    assert_eq!(groceries[0]["store"], "grocer");
    assert_eq!(groceries[0]["cat"], "Produce", "in a category the frontend shows");
    assert_eq!(
        groceries[0]["note"],
        "Mealie isn't working (HTTP 503) · showing the list from 17:30 · changes won't save"
    );
    assert_eq!(groceries.len(), 15, "the error row, then the last list read");

    let add = json!({ "id": "mhabc", "name": "Lemons", "qty": 1 });
    assert_eq!(
        send(&app, post("/api/groceries", add)).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "Mealie isn't working (HTTP 503)" })
        )
    );
    let tick_row = json!({ "id": UNAVAILABLE_ID, "name": "Can't load the shopping list", "done": true });
    assert_eq!(
        send(&app, post("/api/groceries", tick_row)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let clear = json!({ "ids": ["1e3a0000-0000-4000-8000-000000000005"] });
    assert_eq!(
        send(&app, post("/api/groceries/clear", clear)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );

    fake.set_down(false);
    let (_, s) = send(&app, state()).await;
    assert!(
        !s["groceries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["id"] == UNAVAILABLE_ID)
    );
}

#[tokio::test]
async fn mealie_down_before_any_read_shows_only_the_error_row() {
    let fake = FakeMealie::from_fixtures().await;
    fake.set_down(true);
    let app = app(&fake).await;
    let (_, s) = send(&app, state()).await;
    let groceries = s["groceries"].as_array().unwrap();
    assert_eq!(groceries.len(), 1);
    assert_eq!(
        groceries[0]["note"],
        "Mealie isn't working (HTTP 503) · changes won't save"
    );
}
