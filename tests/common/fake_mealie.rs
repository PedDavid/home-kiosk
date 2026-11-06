//! An in-process fake of the parts of Mealie's API the shopping adapter uses, seeded from the
//! recorded-shape responses in `tests/fixtures/mealie/`. It keeps state, so writes show up in
//! later reads, and it copies the Mealie behaviour the adapter relies on: bearer-token auth,
//! client-supplied item ids on create, merging a new plain item into an unticked one with the
//! same note, labels resolved from `labelId` (or the food's label), and full-replace `PUT`s.
//!
//! `POST /__fake/down` and `/__fake/up` switch every `/api` route to 503 and back (no auth), so a
//! running backend can be shown "Mealie is down".

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path as UrlPath, RawQuery, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde_json::{Map, Value, json};

pub const TOKEN: &str = "fake-mealie-token";
const GROUP: &str = "4d5c6b7a-0000-4000-8000-0000000000aa";
const HOUSEHOLD: &str = "4d5c6b7a-0000-4000-8000-0000000000bb";

#[derive(Default)]
pub struct Data {
    pub down: bool,
    /// Lists without their items.
    pub lists: Vec<Value>,
    /// Items in Mealie's `ShoppingListItemOut` shape.
    pub items: Vec<Value>,
    pub labels: Vec<Value>,
    /// Foods and units seen in the seed, so a PUT naming one gets the full object back.
    pub foods: Vec<Value>,
    pub units: Vec<Value>,
    /// Whether the token may create labels.
    pub can_create_labels: bool,
    pub next_id: u64,
    /// "METHOD path" of every authenticated request, in order.
    pub log: Vec<String>,
}

#[derive(Clone)]
pub struct FakeMealie {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

impl FakeMealie {
    /// Lists with these names, no items and no labels.
    pub async fn empty(lists: &[&str]) -> Self {
        let mut d = Data {
            can_create_labels: true,
            ..Data::default()
        };
        for (i, name) in lists.iter().enumerate() {
            d.lists.push(json!({
                "name": name, "extras": {}, "groupId": GROUP, "householdId": HOUSEHOLD,
                "id": format!("aaaaaaaa-0000-4000-8000-{:012x}", i + 1),
                "recipeReferences": [],
            }));
        }
        Self::serve(d, None).await
    }

    /// Seeded from `tests/fixtures/mealie/` (lists.json, labels.json, list-*.json).
    pub async fn from_fixtures() -> Self {
        Self::from_fixtures_on(None).await
    }

    pub async fn from_fixtures_on(addr: Option<SocketAddr>) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mealie");
        let read = |f: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap() };
        let mut d = Data {
            can_create_labels: true,
            ..Data::default()
        };
        d.lists = read("lists.json")["items"].as_array().unwrap().clone();
        d.labels = read("labels.json")["items"].as_array().unwrap().clone();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.starts_with("list-") {
                let l = read(&name);
                d.items.extend(l["listItems"].as_array().unwrap().iter().cloned());
            }
        }
        for i in &d.items {
            if i["food"].is_object() && !d.foods.iter().any(|f| f["id"] == i["food"]["id"]) {
                d.foods.push(i["food"].clone());
            }
            if i["unit"].is_object() && !d.units.iter().any(|u| u["id"] == i["unit"]["id"]) {
                d.units.push(i["unit"].clone());
            }
        }
        Self::serve(d, addr).await
    }

    async fn serve(d: Data, addr: Option<SocketAddr>) -> Self {
        let data = Arc::new(Mutex::new(d));
        let api = Router::new()
            .route("/api/households/shopping/lists", get(lists))
            .route("/api/households/shopping/lists/{id}", get(one_list))
            .route("/api/households/shopping/items", post(create_one).delete(delete_many))
            .route("/api/households/shopping/items/create-bulk", post(create_bulk))
            .route("/api/households/shopping/items/{id}", put(update_one))
            .route("/api/groups/labels", get(labels).post(create_label))
            .layer(middleware::from_fn_with_state(data.clone(), gate));
        let app = Router::new()
            .route(
                "/__fake/down",
                post(|State(d): State<Shared>| async move {
                    d.lock().unwrap().down = true;
                    "down"
                }),
            )
            .route(
                "/__fake/up",
                post(|State(d): State<Shared>| async move {
                    d.lock().unwrap().down = false;
                    "up"
                }),
            )
            .merge(api)
            .with_state(data.clone());
        let listener = tokio::net::TcpListener::bind(addr.unwrap_or(([127, 0, 0, 1], 0).into()))
            .await
            .unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeMealie { url, data }
    }

    pub fn set_down(&self, down: bool) {
        self.data.lock().unwrap().down = down;
    }

    pub fn list_id(&self, name: &str) -> String {
        let d = self.data.lock().unwrap();
        d.lists.iter().find(|l| l["name"] == name).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// The items of a list, as Mealie stores them.
    pub fn items_of(&self, list: &str) -> Vec<Value> {
        let id = self.list_id(list);
        let d = self.data.lock().unwrap();
        d.items
            .iter()
            .filter(|i| i["shoppingListId"] == id.as_str())
            .cloned()
            .collect()
    }

    /// Add an item the way Mealie's own UI would (a plain, note-only item).
    pub fn add_in_mealie(&self, list: &str, note: &str, label: Option<&str>) -> String {
        let list_id = self.list_id(list);
        let mut d = self.data.lock().unwrap();
        let label_id = label.map(|n| d.labels.iter().find(|l| l["name"] == n).unwrap()["id"].clone());
        let body = json!({ "shoppingListId": list_id, "note": note, "quantity": 1.0, "labelId": label_id });
        let out = d.build(&body, None);
        let id = out["id"].as_str().unwrap().to_string();
        d.items.push(out);
        id
    }

    /// Change an item the way Mealie's UI would.
    pub fn edit_in_mealie(&self, id: &str, f: impl FnOnce(&mut Value)) {
        let mut d = self.data.lock().unwrap();
        let i = d.items.iter_mut().find(|i| i["id"] == id).unwrap();
        f(i);
    }

    pub fn log(&self) -> Vec<String> {
        self.data.lock().unwrap().log.clone()
    }
}

type Shared = Arc<Mutex<Data>>;

async fn gate(State(d): State<Shared>, headers: HeaderMap, req: Request, next: Next) -> Response {
    {
        let mut d = d.lock().unwrap();
        if d.down {
            return (StatusCode::SERVICE_UNAVAILABLE, "upstream unavailable").into_response();
        }
        let ok = headers.get("authorization").and_then(|h| h.to_str().ok()) == Some(format!("Bearer {TOKEN}").as_str());
        if !ok {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "detail": "Not authenticated" }))).into_response();
        }
        let q = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        d.log.push(format!("{} {}{}", req.method(), req.uri().path(), q));
    }
    next.run(req).await
}

fn page(items: Vec<Value>) -> Value {
    json!({ "page": 1, "perPage": -1, "total": items.len(), "totalPages": 1, "items": items, "next": null, "previous": null })
}

async fn lists(State(d): State<Shared>) -> Json<Value> {
    Json(page(d.lock().unwrap().lists.clone()))
}

async fn one_list(State(d): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    let d = d.lock().unwrap();
    let Some(l) = d.lists.iter().find(|l| l["id"] == id.as_str()) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "detail": "Not found" }))).into_response();
    };
    let mut l = l.clone();
    let mut items: Vec<Value> = d
        .items
        .iter()
        .filter(|i| i["shoppingListId"] == id.as_str())
        .cloned()
        .collect();
    items.sort_by_key(|i| i["position"].as_i64().unwrap_or(0));
    l["listItems"] = items.into();
    l["labelSettings"] = json!([]);
    Json(l).into_response()
}

impl Data {
    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("bbbbbbbb-0000-4000-8000-{:012x}", self.next_id)
    }

    /// An item in Mealie's output shape from a create or update body.
    fn build(&mut self, b: &Value, id: Option<&str>) -> Value {
        let id = id
            .map(String::from)
            .or_else(|| b["id"].as_str().map(String::from))
            .unwrap_or_else(|| self.new_id());
        let food = self
            .foods
            .iter()
            .find(|f| !b["foodId"].is_null() && f["id"] == b["foodId"])
            .cloned();
        let unit = self
            .units
            .iter()
            .find(|u| !b["unitId"].is_null() && u["id"] == b["unitId"])
            .cloned();
        let label = self
            .labels
            .iter()
            .find(|l| !b["labelId"].is_null() && l["id"] == b["labelId"])
            .cloned()
            .or_else(|| {
                food.as_ref()
                    .and_then(|f| f["label"].as_object().map(|o| Value::Object(o.clone())))
            });
        let extras: Map<String, Value> = b["extras"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_str()
                                .map(Value::from)
                                .unwrap_or_else(|| Value::from(v.to_string())),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "quantity": b["quantity"].as_f64().unwrap_or(1.0),
            "unit": unit, "food": food, "referencedRecipe": null,
            "note": b["note"].as_str().unwrap_or(""), "display": b["note"].as_str().unwrap_or(""),
            "shoppingListId": b["shoppingListId"], "checked": b["checked"].as_bool().unwrap_or(false),
            "position": b["position"].as_i64().unwrap_or(0),
            "foodId": food.as_ref().map(|f| f["id"].clone()), "unitId": unit.as_ref().map(|u| u["id"].clone()),
            "labelId": label.as_ref().map(|l| l["id"].clone()),
            "extras": extras, "id": id, "groupId": GROUP, "householdId": HOUSEHOLD,
            "label": label, "recipeReferences": b["recipeReferences"].as_array().cloned().unwrap_or_default(),
            "createdAt": "2026-11-06T09:00:00Z", "updatedAt": "2026-11-06T09:00:00Z",
        })
    }

    fn create(&mut self, bodies: &[Value]) -> Result<Value, StatusCode> {
        let mut created = Vec::new();
        let mut updated = Vec::new();
        for b in bodies {
            if !self.lists.iter().any(|l| l["id"] == b["shoppingListId"]) {
                return Err(StatusCode::NOT_FOUND);
            }
            let checked = b["checked"].as_bool().unwrap_or(false);
            let plain = b["foodId"].is_null();
            // Mealie merges a new unticked plain item into an unticked one with the same note.
            if !checked
                && plain
                && let Some(existing) = self.items.iter_mut().find(|i| {
                    i["shoppingListId"] == b["shoppingListId"]
                        && i["checked"] == false
                        && i["foodId"].is_null()
                        && i["note"] == b["note"]
                })
            {
                let q = existing["quantity"].as_f64().unwrap_or(0.0) + b["quantity"].as_f64().unwrap_or(1.0);
                existing["quantity"] = q.into();
                updated.push(existing.clone());
                continue;
            }
            let out = self.build(b, None);
            if self.items.iter().any(|i| i["id"] == out["id"]) {
                return Err(StatusCode::CONFLICT);
            }
            self.items.push(out.clone());
            created.push(out);
        }
        Ok(json!({ "createdItems": created, "updatedItems": updated, "deletedItems": [] }))
    }
}

async fn create_one(State(d): State<Shared>, Json(b): Json<Value>) -> Response {
    match d.lock().unwrap().create(&[b]) {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(s) => s.into_response(),
    }
}

async fn create_bulk(State(d): State<Shared>, Json(b): Json<Vec<Value>>) -> Response {
    match d.lock().unwrap().create(&b) {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(s) => s.into_response(),
    }
}

async fn update_one(State(d): State<Shared>, UrlPath(id): UrlPath<String>, Json(b): Json<Value>) -> Response {
    let mut d = d.lock().unwrap();
    if b["shoppingListId"].is_null() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "detail": "shoppingListId required" })),
        )
            .into_response();
    }
    let Some(pos) = d.items.iter().position(|i| i["id"] == id.as_str()) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "detail": "Not found" }))).into_response();
    };
    let created_at = d.items[pos]["createdAt"].clone();
    let mut out = d.build(&b, Some(&id));
    out["createdAt"] = created_at;
    d.items[pos] = out.clone();
    Json(json!({ "createdItems": [], "updatedItems": [out], "deletedItems": [] })).into_response()
}

async fn delete_many(State(d): State<Shared>, RawQuery(q): RawQuery) -> Json<Value> {
    let ids: Vec<String> = q
        .unwrap_or_default()
        .split('&')
        .filter_map(|kv| kv.strip_prefix("ids=").map(String::from))
        .collect();
    d.lock()
        .unwrap()
        .items
        .retain(|i| !ids.iter().any(|id| i["id"] == id.as_str()));
    Json(json!({ "message": "", "error": false }))
}

async fn labels(State(d): State<Shared>) -> Json<Value> {
    Json(page(d.lock().unwrap().labels.clone()))
}

async fn create_label(State(d): State<Shared>, Json(b): Json<Value>) -> Response {
    let mut d = d.lock().unwrap();
    if !d.can_create_labels {
        return (StatusCode::FORBIDDEN, Json(json!({ "detail": "Forbidden" }))).into_response();
    }
    let id = d.new_id();
    let l = json!({ "name": b["name"], "color": b["color"].as_str().unwrap_or("#959595"), "groupId": GROUP, "id": id });
    d.labels.push(l.clone());
    Json(l).into_response()
}
