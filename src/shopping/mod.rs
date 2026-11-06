//! The shopping list port (`ShoppingList`) and what the API layer needs around it.
//!
//! The frontend talks upstream's shapes: `/api/state` → `groceries` is a list of
//! `{ id, name, cat, qty, store, note, done }`, `POST /api/groceries` upserts one item (add, tick,
//! untick, quantity, store, note) and `POST /api/groceries/clear` clears ticked items
//! (`cleared: false` restores them, for Undo). [`Groceries`] turns those requests into port calls,
//! and keeps the last list it read, so the shopping tab can show an error *and* the last known
//! list when the backing store is down, instead of an empty list.
//!
//! Adapters: [`mealie::Mealie`] (the real one) and [`memory::MemoryShoppingList`] (a reference
//! implementation the contract suite also runs against). When no adapter is configured the API
//! keeps the demo list in `demo.rs`.

pub mod mealie;
pub mod memory;

use std::{future::Future, pin::Pin, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::settings::Settings;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One shopping item, in the frontend's shape and field order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The frontend's id. Items the frontend adds carry a client-generated id; items added
    /// elsewhere get one from the adapter. It stays the same for the item's whole life.
    pub id: String,
    pub name: String,
    /// One of the settings' categories, or "Other".
    pub cat: String,
    /// 1..=99.
    pub qty: u32,
    /// One of the settings' store ids.
    pub store: String,
    pub note: String,
    pub done: bool,
}

/// Why a shopping call failed. Both are shown to the user ("Couldn't save · <message>").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShoppingError {
    /// The backing store is unreachable or failing (network, timeout, 5xx). Retrying may work.
    Unavailable(String),
    /// The backing store answered, but refused or didn't understand (auth, a missing list, 4xx).
    Rejected(String),
}

impl ShoppingError {
    pub fn message(&self) -> &str {
        match self {
            ShoppingError::Unavailable(m) | ShoppingError::Rejected(m) => m,
        }
    }
}

impl std::fmt::Display for ShoppingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// The household shopping list. Object-safe, so the app holds an `Arc<dyn ShoppingList>`.
///
/// Behaviour every adapter must have (see `tests/shopping_contract.rs`):
/// - `put` is an upsert keyed by `Item::id`: an unknown id adds the item, a known one updates it
///   (quantity, store, note, category, name, ticked or not). The id never changes.
/// - `clear` removes the given items that are ticked; unticked or unknown ids are left alone.
/// - `restore` brings cleared items back exactly as they were (ticked), for Undo. Ids that
///   weren't cleared, or that are already back, are ignored.
pub trait ShoppingList: Send + Sync {
    /// Whether this adapter has what it needs to work (feeds `/api/state` capability flags).
    fn configured(&self) -> bool {
        true
    }
    fn list(&self) -> BoxFuture<'_, Result<Vec<Item>, ShoppingError>>;
    fn put(&self, item: Item) -> BoxFuture<'_, Result<(), ShoppingError>>;
    fn clear(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>>;
    fn restore(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>>;
}

/// The id of the row that stands in for the list while the store can't be read.
pub const UNAVAILABLE_ID: &str = "hq-shopping-unavailable";

/// A failed request, as the API layer renders it.
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
}

impl From<ShoppingError> for Failure {
    fn from(e: ShoppingError) -> Self {
        let status = match e {
            ShoppingError::Unavailable(_) => 503,
            ShoppingError::Rejected(_) => 502,
        };
        Failure {
            status,
            message: e.message().to_string(),
        }
    }
}

/// The API's view of the shopping list: request parsing, upstream's validation, and the
/// "can't reach it" state.
pub struct Groceries {
    port: Arc<dyn ShoppingList>,
    settings: Settings,
    last: Mutex<Option<(DateTime<Utc>, Vec<Item>)>>,
}

impl Groceries {
    pub fn new(port: Arc<dyn ShoppingList>, settings: Settings) -> Self {
        Groceries {
            port,
            settings,
            last: Mutex::new(None),
        }
    }

    pub fn configured(&self) -> bool {
        self.port.configured()
    }

    /// `/api/state` → `groceries`. When the list can't be read, the last list read (if any) plus
    /// one row saying so, at the top of the first store's list, so the tab never looks empty.
    pub async fn state(&self, now: DateTime<Utc>) -> Value {
        match self.port.list().await {
            Ok(items) => {
                *self.last.lock().await = Some((now, items.clone()));
                json!(items)
            }
            Err(e) => {
                tracing::warn!(error = %e, "shopping list unavailable");
                let last = self.last.lock().await.clone();
                let mut rows = vec![self.unavailable_row(&e, last.as_ref().map(|(at, _)| *at))];
                rows.extend(last.map(|(_, items)| items).unwrap_or_default());
                json!(rows)
            }
        }
    }

    fn unavailable_row(&self, e: &ShoppingError, since: Option<DateTime<Utc>>) -> Item {
        let tz = self.settings.tz().unwrap_or(chrono_tz::UTC);
        let note = match since {
            Some(at) => format!(
                "{} · showing the list from {} · changes won't save",
                e.message(),
                at.with_timezone(&tz).format("%H:%M")
            ),
            None => format!("{} · changes won't save", e.message()),
        };
        let cat = self
            .settings
            .categories()
            .into_iter()
            .next()
            .unwrap_or_else(|| "Other".into());
        Item {
            id: UNAVAILABLE_ID.into(),
            name: "Can't load the shopping list".into(),
            cat,
            qty: 1,
            store: self.settings.stores()[0].id.clone(),
            note,
            done: false,
        }
    }

    /// `POST /api/groceries`.
    pub async fn put(&self, body: &Value) -> Result<(), Failure> {
        let item = parse_item(body, &self.settings).map_err(|m| Failure {
            status: 400,
            message: m.into(),
        })?;
        if item.id == UNAVAILABLE_ID {
            return Err(Failure {
                status: 503,
                message: "The shopping list can't be reached".into(),
            });
        }
        self.port.put(item).await.map_err(Failure::from)
    }

    /// `POST /api/groceries/clear`.
    pub async fn clear(&self, mut ids: Vec<String>, restore: bool) -> Result<(), Failure> {
        ids.retain(|id| id != UNAVAILABLE_ID);
        if ids.is_empty() {
            return Ok(());
        }
        let r = if restore {
            self.port.restore(ids).await
        } else {
            self.port.clear(ids).await
        };
        r.map_err(Failure::from)
    }
}

/// Upstream's validation for `POST /api/groceries`: id and name required, qty clamped to 1..=99,
/// an unknown store becomes the first store, an empty category becomes "Other".
pub fn parse_item(b: &Value, settings: &Settings) -> Result<Item, &'static str> {
    let text = |v: &Value, max: usize| -> String { v.as_str().unwrap_or("").trim().chars().take(max).collect() };
    let id = text(&b["id"], 64);
    let name = text(&b["name"], 80);
    if id.is_empty() || name.is_empty() {
        return Err("id and name required");
    }
    let stores = settings.stores();
    let store = b["store"]
        .as_str()
        .filter(|s| stores.iter().any(|x| x.id == *s))
        .unwrap_or(&stores[0].id)
        .to_string();
    let qty = b["qty"]
        .as_i64()
        .or_else(|| b["qty"].as_f64().map(|f| f as i64))
        .or_else(|| b["qty"].as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(1)
        .clamp(1, 99) as u32;
    let cat = Some(text(&b["cat"], 40))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "Other".into());
    Ok(Item {
        id,
        name,
        cat,
        qty,
        store,
        note: text(&b["note"], 120),
        done: b["done"].as_bool().unwrap_or(false),
    })
}

/// The shopping adapter the environment asks for: Mealie when `MEALIE_URL` is set.
/// `None` keeps the demo list.
pub fn from_env(settings: &Settings) -> anyhow::Result<Option<Arc<dyn ShoppingList>>> {
    let url = std::env::var("MEALIE_URL").ok().filter(|u| !u.trim().is_empty());
    let token = std::env::var("MEALIE_TOKEN").ok().filter(|t| !t.trim().is_empty());
    match url {
        None => {
            if settings.shopping.mealie.is_some() {
                tracing::warn!("settings name a Mealie list but MEALIE_URL is not set: using the demo shopping list");
            }
            Ok(None)
        }
        Some(url) => {
            let token = token.ok_or_else(|| anyhow::anyhow!("MEALIE_URL is set but MEALIE_TOKEN is not"))?;
            let m = mealie::Mealie::new(&url, &token, settings)?;
            tracing::info!(url = %url, lists = ?m.list_names(), "shopping list from Mealie");
            Ok(Some(Arc::new(m)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EXAMPLE;

    #[test]
    fn parse_item_applies_upstreams_rules() {
        let s = Settings::parse(EXAMPLE).unwrap();
        let i = parse_item(
            &json!({ "id": " a1 ", "name": "Milk", "qty": "200", "store": "nowhere" }),
            &s,
        )
        .unwrap();
        assert_eq!(i.id, "a1");
        assert_eq!(i.qty, 99);
        assert_eq!(i.store, "grocer");
        assert_eq!(i.cat, "Other");
        assert!(!i.done);
        assert!(parse_item(&json!({ "id": "a" }), &s).is_err());
        assert_eq!(
            parse_item(&json!({ "id": "a", "name": "x", "qty": 0 }), &s)
                .unwrap()
                .qty,
            1
        );
    }
}
