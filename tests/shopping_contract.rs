//! The `ShoppingList` contract: behaviour every adapter must have, run against each of them.
//! Mealie runs against the in-process fake in `common/fake_mealie.rs` (no real network).
//!
//! To add an adapter: write a `make_*` function that returns a fresh, empty list for the example
//! settings (stores `grocer` and `market`), and add a `contract!` line for it.

#[path = "common/fake_mealie.rs"]
mod fake_mealie;

use std::sync::Arc;

use home_hq::{
    settings::{EXAMPLE, Settings},
    shopping::{Item, ShoppingList, mealie::Mealie, memory::MemoryShoppingList},
};
use serde_json::{Value, json};

pub fn settings() -> Settings {
    let mut v: Value = serde_json::from_str(EXAMPLE).unwrap();
    v["shopping"]["mealie"] = json!({ "list": "Groceries", "storeLists": { "market": "Market" } });
    Settings::parse(&v.to_string()).unwrap()
}

async fn make_memory() -> Arc<dyn ShoppingList> {
    Arc::new(MemoryShoppingList::new())
}

async fn make_mealie() -> Arc<dyn ShoppingList> {
    let fake = fake_mealie::FakeMealie::empty(&["Groceries", "Market", "Something else"]).await;
    Arc::new(Mealie::new(&fake.url, fake_mealie::TOKEN, &settings()).unwrap())
}

fn item(id: &str, name: &str) -> Item {
    Item {
        id: id.into(),
        name: name.into(),
        cat: "Produce".into(),
        qty: 1,
        store: "grocer".into(),
        note: String::new(),
        done: false,
    }
}

async fn listed(l: &dyn ShoppingList) -> Vec<Item> {
    let mut items = l.list().await.unwrap();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
}

async fn one(l: &dyn ShoppingList, id: &str) -> Option<Item> {
    let mut found: Vec<Item> = l.list().await.unwrap().into_iter().filter(|i| i.id == id).collect();
    assert!(found.len() <= 1, "{id} is listed {} times", found.len());
    found.pop()
}

pub mod cases {
    use super::*;

    pub async fn reports_configured(l: Arc<dyn ShoppingList>) {
        assert!(l.configured());
    }

    pub async fn starts_empty(l: Arc<dyn ShoppingList>) {
        assert_eq!(listed(&*l).await, vec![]);
    }

    pub async fn added_items_keep_their_id_and_fields(l: Arc<dyn ShoppingList>) {
        let lemons = Item {
            qty: 3,
            note: "unwaxed".into(),
            ..item("c1", "Lemons")
        };
        let milk = Item {
            cat: "Dairy & Eggs".into(),
            store: "market".into(),
            ..item("c2", "Milk")
        };
        l.put(lemons.clone()).await.unwrap();
        l.put(milk.clone()).await.unwrap();
        assert_eq!(listed(&*l).await, vec![lemons, milk]);
    }

    pub async fn put_updates_in_place(l: Arc<dyn ShoppingList>) {
        l.put(item("c1", "Lemons")).await.unwrap();
        let changed = Item {
            qty: 5,
            note: "for the cake".into(),
            ..item("c1", "Lemons")
        };
        l.put(changed.clone()).await.unwrap();
        assert_eq!(listed(&*l).await, vec![changed]);
    }

    pub async fn ticks_and_unticks(l: Arc<dyn ShoppingList>) {
        l.put(item("c1", "Lemons")).await.unwrap();
        l.put(Item {
            done: true,
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        assert!(one(&*l, "c1").await.unwrap().done);
        l.put(item("c1", "Lemons")).await.unwrap();
        assert!(!one(&*l, "c1").await.unwrap().done);
    }

    pub async fn moves_between_stores(l: Arc<dyn ShoppingList>) {
        l.put(Item {
            qty: 2,
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        l.put(Item {
            qty: 2,
            store: "market".into(),
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        let moved = one(&*l, "c1").await.unwrap();
        assert_eq!((moved.store.as_str(), moved.qty), ("market", 2));
        l.put(Item {
            qty: 2,
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        assert_eq!(one(&*l, "c1").await.unwrap().store, "grocer");
    }

    pub async fn clear_removes_only_ticked_items(l: Arc<dyn ShoppingList>) {
        l.put(Item {
            done: true,
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        l.put(item("c2", "Bread")).await.unwrap();
        l.put(Item {
            done: true,
            store: "market".into(),
            ..item("c3", "Cod")
        })
        .await
        .unwrap();
        l.clear(vec!["c1".into(), "c2".into(), "c3".into(), "nope".into()])
            .await
            .unwrap();
        assert_eq!(listed(&*l).await, vec![item("c2", "Bread")]);
    }

    pub async fn restore_brings_cleared_items_back(l: Arc<dyn ShoppingList>) {
        let lemons = Item {
            done: true,
            qty: 4,
            note: "unwaxed".into(),
            ..item("c1", "Lemons")
        };
        let cod = Item {
            done: true,
            store: "market".into(),
            cat: "Meat & Fish".into(),
            ..item("c3", "Cod")
        };
        l.put(lemons.clone()).await.unwrap();
        l.put(cod.clone()).await.unwrap();
        l.clear(vec!["c1".into(), "c3".into()]).await.unwrap();
        assert_eq!(listed(&*l).await, vec![]);
        l.restore(vec!["c1".into(), "c3".into(), "never-cleared".into()])
            .await
            .unwrap();
        assert_eq!(listed(&*l).await, vec![lemons.clone(), cod.clone()]);
        // A second Undo is a no-op, not a duplicate.
        l.restore(vec!["c1".into()]).await.unwrap();
        assert_eq!(listed(&*l).await, vec![lemons, cod]);
    }

    pub async fn restored_items_can_be_changed_again(l: Arc<dyn ShoppingList>) {
        l.put(Item {
            done: true,
            ..item("c1", "Lemons")
        })
        .await
        .unwrap();
        l.clear(vec!["c1".into()]).await.unwrap();
        l.restore(vec!["c1".into()]).await.unwrap();
        l.put(item("c1", "Lemons")).await.unwrap();
        assert_eq!(listed(&*l).await, vec![item("c1", "Lemons")]);
    }

    pub async fn unknown_ids_are_ignored(l: Arc<dyn ShoppingList>) {
        l.put(item("c1", "Lemons")).await.unwrap();
        l.clear(vec!["nope".into()]).await.unwrap();
        l.restore(vec!["nope".into()]).await.unwrap();
        assert_eq!(listed(&*l).await, vec![item("c1", "Lemons")]);
    }
}

macro_rules! contract {
    ($adapter:ident, $make:path) => {
        mod $adapter {
            use super::*;
            contract!(@cases $make; reports_configured, starts_empty, added_items_keep_their_id_and_fields,
                put_updates_in_place, ticks_and_unticks, moves_between_stores,
                clear_removes_only_ticked_items, restore_brings_cleared_items_back,
                restored_items_can_be_changed_again, unknown_ids_are_ignored);
        }
    };
    (@cases $make:path; $($case:ident),*) => {
        $(
            #[tokio::test]
            async fn $case() {
                cases::$case($make().await).await;
            }
        )*
    };
}

contract!(memory, make_memory);
contract!(mealie, make_mealie);
