//! `ShoppingList` backed by Mealie's shopping lists (Mealie v2+/v3 API, `/api/households/...`).
//!
//! Mapping:
//! - **Lists and stores.** `shopping.mealie.list` in the settings names the Mealie list. A store can
//!   have a list of its own (`shopping.mealie.storeLists: { "<store id>": "<list name>" }`); every
//!   other store shares the main list, and which of those stores an item belongs to is kept in the
//!   item's `extras` (`homeHqStore`). Items added in Mealie land in the list's first store.
//!   Moving an item to a store with its own list moves it to that Mealie list.
//! - **Categories** are Mealie labels, matched by name (case-insensitive). An item's own label
//!   wins, then its food's label. Labels that aren't one of the settings' categories show under
//!   "Other" (the frontend hides categories it doesn't know). A category with no label yet gets one
//!   created; if the token may not create labels, the item is saved without one.
//! - **Names, notes and quantities.** A food item shows the food's name and Mealie's note as the
//!   note. A plain (note-only) item shows Mealie's note as the name, and our note lives in
//!   `extras.homeHqNote`. Whole quantities without a unit are the quantity; anything else (a unit,
//!   a fraction) shows as quantity 1 with the amount at the front of the note ("500 g"), and the
//!   frame's + and − leave it alone.
//! - **Ids.** The frontend's client-generated id is stored in `extras.homeHqId` and reported back
//!   as the item's id; items created in Mealie report their Mealie id.
//! - **Clear and Undo.** Clearing deletes the ticked items in Mealie (as Mealie's own "delete
//!   checked" does) and keeps a copy here for an hour. Undo re-creates them from that copy with
//!   the same Mealie id, food, unit, label, extras and recipe references, still ticked.
//!
//! Every write reads the lists first and changes only the fields the frame changed, so anything
//! the frontend doesn't know about (foods, units, recipe references, positions) is kept.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{BoxFuture, Item, ShoppingError, ShoppingList};
use crate::settings::Settings;

/// `shopping.mealie` in the settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MealieSettings {
    /// The Mealie shopping list's name.
    #[serde(default)]
    pub list: Option<String>,
    /// Store id -> name of a Mealie list of its own.
    #[serde(default)]
    pub store_lists: BTreeMap<String, String>,
}

const EXTRA_ID: &str = "homeHqId";
const EXTRA_STORE: &str = "homeHqStore";
const EXTRA_NOTE: &str = "homeHqNote";

/// How long cleared items can be brought back with Undo.
const UNDO_FOR: Duration = Duration::from_secs(3600);
const LABELS_FOR: Duration = Duration::from_secs(300);

pub struct Mealie {
    http: reqwest::Client,
    base: String,
    auth: String,
    /// Store id -> list name, for every store, in the settings' store order.
    store_lists: Vec<(String, String)>,
    categories: Vec<String>,
    list_ids: Mutex<HashMap<String, String>>,
    labels: Mutex<Option<(Instant, Vec<Label>)>>,
    /// Frontend id -> Mealie id, for items Mealie merged into another on create.
    aliases: Mutex<HashMap<String, String>>,
    /// Frontend id -> (when, the item as Mealie had it), for Undo.
    cleared: Mutex<HashMap<String, (Instant, Value)>>,
    /// One write at a time, so two quick saves of a new item don't create it twice.
    writes: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
struct Label {
    id: String,
    name: String,
}

/// A Mealie item and where it lives.
struct Found {
    list: String,
    raw: Value,
}

impl Mealie {
    pub fn new(url: &str, token: &str, settings: &Settings) -> anyhow::Result<Self> {
        let cfg = settings.shopping.mealie.clone().unwrap_or_default();
        let main = cfg
            .list
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .ok_or_else(|| anyhow::anyhow!("MEALIE_URL is set but the settings name no list (shopping.mealie.list)"))?;
        let stores = settings.stores();
        for id in cfg.store_lists.keys() {
            if !stores.iter().any(|s| &s.id == id) {
                anyhow::bail!("shopping.mealie.storeLists names an unknown store {id:?}");
            }
        }
        let store_lists = stores
            .iter()
            .map(|s| {
                let list = cfg
                    .store_lists
                    .get(&s.id)
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| main.clone());
                (s.id.clone(), list)
            })
            .collect();
        crate::tls::install_crypto_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .user_agent(concat!("home-hq/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Mealie {
            http,
            base: url.trim().trim_end_matches('/').to_string(),
            auth: format!("Bearer {}", token.trim()),
            store_lists,
            categories: settings.categories(),
            list_ids: Mutex::default(),
            labels: Mutex::default(),
            aliases: Mutex::default(),
            cleared: Mutex::default(),
            writes: tokio::sync::Mutex::new(()),
        })
    }

    /// The Mealie lists this reads, in store order.
    pub fn list_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, l) in &self.store_lists {
            if !out.contains(l) {
                out.push(l.clone());
            }
        }
        out
    }

    fn list_for_store(&self, store: &str) -> &str {
        self.store_lists
            .iter()
            .find(|(s, _)| s == store)
            .or(self.store_lists.first())
            .map(|(_, l)| l.as_str())
            .unwrap_or("")
    }

    fn stores_of_list(&self, list: &str) -> Vec<&str> {
        self.store_lists
            .iter()
            .filter(|(_, l)| l == list)
            .map(|(s, _)| s.as_str())
            .collect()
    }

    // ------------------------------------------------------------ HTTP

    async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value, ShoppingError> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base, path))
            .header(reqwest::header::AUTHORIZATION, &self.auth)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|e| {
            tracing::warn!(error = %e, path, "Mealie request failed");
            ShoppingError::Unavailable("Can't reach Mealie".into())
        })?;
        let status = res.status();
        let text = res
            .text()
            .await
            .map_err(|_| ShoppingError::Unavailable("Can't reach Mealie".into()))?;
        if !status.is_success() {
            tracing::warn!(%status, path, body = %text.chars().take(300).collect::<String>(), "Mealie refused a request");
            return Err(status_error(status));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| ShoppingError::Rejected("Mealie sent something unexpected".into()))
    }

    async fn list_id(&self, name: &str, refresh: bool) -> Result<String, ShoppingError> {
        let key = name.to_lowercase();
        if !refresh && let Some(id) = self.list_ids.lock().unwrap().get(&key) {
            return Ok(id.clone());
        }
        let page = self
            .call(Method::GET, "/api/households/shopping/lists?perPage=-1", None)
            .await?;
        let mut ids = HashMap::new();
        for l in page["items"].as_array().into_iter().flatten() {
            if let (Some(n), Some(id)) = (l["name"].as_str(), l["id"].as_str()) {
                ids.entry(n.trim().to_lowercase()).or_insert_with(|| id.to_string());
            }
        }
        let found = ids.get(&key).cloned();
        *self.list_ids.lock().unwrap() = ids;
        found.ok_or_else(|| {
            ShoppingError::Rejected(format!("Mealie has no shopping list called \u{201c}{name}\u{201d}"))
        })
    }

    /// The items of one Mealie list (resolving its id again if the list was replaced).
    async fn list_items(&self, name: &str) -> Result<(String, Vec<Value>), ShoppingError> {
        let mut refresh = false;
        loop {
            let id = self.list_id(name, refresh).await?;
            match self
                .call(Method::GET, &format!("/api/households/shopping/lists/{id}"), None)
                .await
            {
                Ok(l) => {
                    let mut items = l["listItems"].as_array().cloned().unwrap_or_default();
                    items.sort_by_key(|i| i["position"].as_i64().unwrap_or(0));
                    return Ok((id, items));
                }
                Err(ShoppingError::Rejected(_)) if !refresh => refresh = true,
                Err(e) => return Err(e),
            }
        }
    }

    /// Every item of every list this reads.
    async fn snapshot(&self) -> Result<Vec<Found>, ShoppingError> {
        let mut out = Vec::new();
        for name in self.list_names() {
            let (_, items) = self.list_items(&name).await?;
            out.extend(items.into_iter().map(|raw| Found {
                list: name.clone(),
                raw,
            }));
        }
        Ok(out)
    }

    async fn labels(&self, refresh: bool) -> Result<Vec<Label>, ShoppingError> {
        if !refresh
            && let Some((at, l)) = &*self.labels.lock().unwrap()
            && at.elapsed() < LABELS_FOR
        {
            return Ok(l.clone());
        }
        let page = self.call(Method::GET, "/api/groups/labels?perPage=-1", None).await?;
        let labels: Vec<Label> = page["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                Some(Label {
                    id: l["id"].as_str()?.to_string(),
                    name: l["name"].as_str()?.to_string(),
                })
            })
            .collect();
        *self.labels.lock().unwrap() = Some((Instant::now(), labels.clone()));
        Ok(labels)
    }

    /// The label id for a category, creating the label if Mealie has none by that name.
    /// "Other" and categories that can't get a label are saved unlabelled.
    async fn label_for(&self, cat: &str) -> Result<Option<String>, ShoppingError> {
        let find = |labels: &[Label]| {
            labels
                .iter()
                .find(|l| l.name.trim().eq_ignore_ascii_case(cat.trim()))
                .map(|l| l.id.clone())
        };
        if let Some(id) = find(&self.labels(false).await?) {
            return Ok(Some(id));
        }
        if let Some(id) = find(&self.labels(true).await?) {
            return Ok(Some(id));
        }
        if cat.eq_ignore_ascii_case("Other") || !self.categories.iter().any(|c| c == cat) {
            return Ok(None);
        }
        match self
            .call(Method::POST, "/api/groups/labels", Some(&json!({ "name": cat })))
            .await
        {
            Ok(l) => {
                *self.labels.lock().unwrap() = None;
                Ok(l["id"].as_str().map(String::from))
            }
            Err(ShoppingError::Rejected(m)) => {
                tracing::warn!(category = cat, error = %m, "couldn't create a Mealie label; saving without one");
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------ mapping

    fn category(&self, raw: &Value) -> String {
        let label = raw["label"]["name"]
            .as_str()
            .or_else(|| raw["food"]["label"]["name"].as_str())
            .unwrap_or("");
        self.categories
            .iter()
            .find(|c| c.trim().eq_ignore_ascii_case(label.trim()))
            .cloned()
            .unwrap_or_else(|| "Other".into())
    }

    fn view(&self, f: &Found) -> Option<Item> {
        let raw = &f.raw;
        let extras = &raw["extras"];
        let mealie_id = raw["id"].as_str()?;
        let id = extras[EXTRA_ID]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(mealie_id)
            .to_string();
        let food = raw["food"]["name"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let mealie_note = raw["note"].as_str().unwrap_or("").trim();
        let (name, note) = match food {
            Some(food) => (food.to_string(), mealie_note.to_string()),
            None => (
                mealie_note.to_string(),
                extras[EXTRA_NOTE].as_str().unwrap_or("").trim().to_string(),
            ),
        };
        let name = if name.is_empty() {
            raw["display"].as_str().unwrap_or("").trim().to_string()
        } else {
            name
        };
        if name.is_empty() {
            return None;
        }
        let (qty, note) = match countable(raw) {
            Some(q) => (q, note),
            None => {
                let amount = amount(raw);
                (
                    1,
                    [amount, note]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
            }
        };
        let stores = self.stores_of_list(&f.list);
        let store = extras[EXTRA_STORE]
            .as_str()
            .filter(|s| stores.contains(s))
            .or(stores.first().copied())
            .unwrap_or("")
            .to_string();
        Some(Item {
            id,
            name: name.chars().take(80).collect(),
            cat: self.category(raw),
            qty,
            store,
            note: note.chars().take(120).collect(),
            done: raw["checked"].as_bool().unwrap_or(false),
        })
    }

    fn find<'a>(&self, all: &'a [Found], id: &str) -> Option<&'a Found> {
        self.find_direct(all, id).or_else(|| self.find_alias(all, id))
    }

    fn find_direct<'a>(&self, all: &'a [Found], id: &str) -> Option<&'a Found> {
        all.iter()
            .find(|f| f.raw["extras"][EXTRA_ID].as_str() == Some(id))
            .or_else(|| all.iter().find(|f| f.raw["id"].as_str() == Some(id)))
    }

    /// An item Mealie merged ours into when we created it.
    fn find_alias<'a>(&self, all: &'a [Found], id: &str) -> Option<&'a Found> {
        let alias = self.aliases.lock().unwrap().get(id).cloned()?;
        all.iter().find(|f| f.raw["id"].as_str() == Some(alias.as_str()))
    }

    // ------------------------------------------------------------ writes

    async fn put_item(&self, item: Item) -> Result<(), ShoppingError> {
        let _one = self.writes.lock().await;
        let all = self.snapshot().await?;
        if let Some(found) = self.find_direct(&all, &item.id) {
            return self.update(found, &item, true).await;
        }
        match self.find_alias(&all, &item.id) {
            // The frame's count was its own share of the merged item: keep Mealie's total.
            Some(found) => self.update(found, &item, false).await,
            None => self.create(&all, &item).await,
        }
    }

    async fn create(&self, all: &[Found], item: &Item) -> Result<(), ShoppingError> {
        let list = self.list_for_store(&item.store).to_string();
        let list_id = self.list_id(&list, false).await?;
        let position = all
            .iter()
            .filter(|f| f.list == list)
            .filter_map(|f| f.raw["position"].as_i64())
            .max()
            .map_or(0, |p| p + 1);
        let mut extras = Map::new();
        extras.insert(EXTRA_ID.into(), item.id.clone().into());
        extras.insert(EXTRA_STORE.into(), item.store.clone().into());
        if !item.note.is_empty() {
            extras.insert(EXTRA_NOTE.into(), item.note.clone().into());
        }
        let body = json!({
            "shoppingListId": list_id,
            "note": item.name,
            "quantity": item.qty,
            "checked": item.done,
            "position": position,
            "labelId": self.label_for(&item.cat).await?,
            "extras": extras,
        });
        let r = self
            .call(Method::POST, "/api/households/shopping/items", Some(&body))
            .await?;
        // Mealie merges an item into an unticked one with the same name: remember where it went.
        let created = r["createdItems"][0]["id"]
            .as_str()
            .or_else(|| r["updatedItems"][0]["id"].as_str());
        if let Some(mid) = created {
            self.aliases.lock().unwrap().insert(item.id.clone(), mid.to_string());
        }
        Ok(())
    }

    async fn update(&self, found: &Found, item: &Item, own_count: bool) -> Result<(), ShoppingError> {
        let Some(view) = self.view(found) else {
            return Ok(());
        };
        let mut raw = found.raw.clone();
        let Some(mealie_id) = raw["id"].as_str().map(String::from) else {
            return Ok(());
        };
        let mut changed = false;
        let o = raw.as_object_mut().expect("Mealie items are objects");
        let mut extras: Map<String, Value> = o.get("extras").and_then(Value::as_object).cloned().unwrap_or_default();
        let has_food = o
            .get("food")
            .is_some_and(|f| f["name"].as_str().is_some_and(|n| !n.trim().is_empty()));

        if item.name != view.name {
            // A renamed food item becomes a plain one: the food's name can't change from here.
            o.insert("foodId".into(), Value::Null);
            o.insert("food".into(), Value::Null);
            o.insert("isFood".into(), false.into());
            o.insert("note".into(), item.name.clone().into());
            if !item.note.is_empty() || extras.contains_key(EXTRA_NOTE) {
                extras.insert(EXTRA_NOTE.into(), item.note.clone().into());
            }
            changed = true;
        } else if item.note != view.note {
            if has_food {
                o.insert("note".into(), item.note.clone().into());
            } else {
                extras.insert(EXTRA_NOTE.into(), item.note.clone().into());
            }
            changed = true;
        }
        if own_count && item.qty != view.qty {
            if countable(&found.raw).is_some() {
                o.insert("quantity".into(), item.qty.into());
                changed = true;
            } else {
                tracing::debug!(id = %item.id, "not changing the quantity of an item with a unit");
            }
        }
        if item.done != view.done {
            o.insert("checked".into(), item.done.into());
            changed = true;
        }
        if item.cat != view.cat {
            o.insert("labelId".into(), self.label_for(&item.cat).await?.into());
            o.remove("label");
            changed = true;
        }
        if item.store != view.store {
            let list = self.list_for_store(&item.store).to_string();
            if list != found.list {
                o.insert("shoppingListId".into(), self.list_id(&list, false).await?.into());
            }
            extras.insert(EXTRA_STORE.into(), item.store.clone().into());
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        o.insert("extras".into(), extras.into());
        self.call(
            Method::PUT,
            &format!("/api/households/shopping/items/{mealie_id}"),
            Some(&raw),
        )
        .await
        .map(drop)
    }

    async fn clear_items(&self, ids: Vec<String>) -> Result<(), ShoppingError> {
        let _one = self.writes.lock().await;
        let all = self.snapshot().await?;
        let mut gone: Vec<(String, Value)> = Vec::new();
        for id in &ids {
            if let Some(f) = self.find(&all, id)
                && f.raw["checked"].as_bool() == Some(true)
                && !gone.iter().any(|(_, r)| r["id"] == f.raw["id"])
            {
                gone.push((id.clone(), f.raw.clone()));
            }
        }
        if gone.is_empty() {
            return Ok(());
        }
        let query: Vec<String> = gone
            .iter()
            .filter_map(|(_, r)| r["id"].as_str().map(|id| format!("ids={id}")))
            .collect();
        self.call(
            Method::DELETE,
            &format!("/api/households/shopping/items?{}", query.join("&")),
            None,
        )
        .await?;
        let mut cleared = self.cleared.lock().unwrap();
        cleared.retain(|_, (at, _)| at.elapsed() < UNDO_FOR);
        let now = Instant::now();
        for (id, raw) in gone {
            cleared.insert(id, (now, raw));
        }
        Ok(())
    }

    async fn restore_items(&self, ids: Vec<String>) -> Result<(), ShoppingError> {
        let _one = self.writes.lock().await;
        let stash: Vec<(String, Value)> = {
            let mut cleared = self.cleared.lock().unwrap();
            cleared.retain(|_, (at, _)| at.elapsed() < UNDO_FOR);
            ids.iter()
                .filter_map(|id| cleared.get(id).map(|(_, raw)| (id.clone(), raw.clone())))
                .collect()
        };
        if stash.is_empty() {
            return Ok(());
        }
        let all = self.snapshot().await?;
        let back: Vec<Value> = stash
            .iter()
            .filter(|(id, raw)| self.find(&all, id).is_none() && !all.iter().any(|f| f.raw["id"] == raw["id"]))
            .map(|(_, raw)| recreate(raw))
            .collect();
        if !back.is_empty() {
            self.call(
                Method::POST,
                "/api/households/shopping/items/create-bulk",
                Some(&Value::Array(back)),
            )
            .await?;
        }
        let mut cleared = self.cleared.lock().unwrap();
        for (id, _) in &stash {
            cleared.remove(id);
        }
        Ok(())
    }
}

impl ShoppingList for Mealie {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Item>, ShoppingError>> {
        Box::pin(async move { Ok(self.snapshot().await?.iter().filter_map(|f| self.view(f)).collect()) })
    }

    fn put(&self, item: Item) -> BoxFuture<'_, Result<(), ShoppingError>> {
        Box::pin(self.put_item(item))
    }

    fn clear(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>> {
        Box::pin(self.clear_items(ids))
    }

    fn restore(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>> {
        Box::pin(self.restore_items(ids))
    }
}

fn status_error(status: StatusCode) -> ShoppingError {
    match status.as_u16() {
        401 | 403 => ShoppingError::Rejected(format!("Mealie refused the token (HTTP {})", status.as_u16())),
        s if s >= 500 => ShoppingError::Unavailable(format!("Mealie isn't working (HTTP {s})")),
        s => ShoppingError::Rejected(format!("Mealie refused the request (HTTP {s})")),
    }
}

/// The quantity, when it's a plain count (no unit, a whole number).
fn countable(raw: &Value) -> Option<u32> {
    let has_unit = !raw["unitId"].is_null() || !raw["unit"].is_null();
    let q = raw["quantity"].as_f64().unwrap_or(1.0);
    (!has_unit && q.fract() == 0.0).then(|| q.clamp(1.0, 99.0) as u32)
}

/// "500 g", "1.5 kg", "0.5": what a non-countable item's amount reads as.
fn amount(raw: &Value) -> String {
    let q = raw["quantity"].as_f64().unwrap_or(0.0);
    let q = if q > 0.0 {
        let s = format!("{q:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        String::new()
    };
    let unit = &raw["unit"];
    let unit = if unit["useAbbreviation"].as_bool() == Some(true) {
        unit["abbreviation"]
            .as_str()
            .filter(|a| !a.is_empty())
            .or(unit["name"].as_str())
    } else {
        unit["name"].as_str()
    }
    .unwrap_or("");
    [q.as_str(), unit]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A create request that brings a deleted item back as it was, under the same id.
fn recreate(raw: &Value) -> Value {
    let refs: Vec<Value> = raw["recipeReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            json!({
                "recipeId": r["recipeId"], "recipeQuantity": r["recipeQuantity"],
                "recipeScale": r["recipeScale"], "recipeNote": r["recipeNote"],
            })
        })
        .collect();
    json!({
        "id": raw["id"],
        "shoppingListId": raw["shoppingListId"],
        "checked": raw["checked"],
        "position": raw["position"],
        "quantity": raw["quantity"],
        "note": raw["note"],
        "isFood": raw["isFood"],
        "foodId": raw["foodId"],
        "unitId": raw["unitId"],
        "labelId": raw["labelId"],
        "extras": raw["extras"],
        "recipeReferences": refs,
    })
}
