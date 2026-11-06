//! An in-memory `ShoppingList`: the reference the contract suite is checked against, and a
//! stand-in for tests that need a working list without Mealie.

use std::sync::Mutex;

use super::{BoxFuture, Item, ShoppingError, ShoppingList};

#[derive(Default)]
pub struct MemoryShoppingList {
    inner: Mutex<Lists>,
}

#[derive(Default)]
struct Lists {
    items: Vec<Item>,
    cleared: Vec<Item>,
}

impl MemoryShoppingList {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ShoppingList for MemoryShoppingList {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Item>, ShoppingError>> {
        let items = self.inner.lock().unwrap().items.clone();
        Box::pin(async move { Ok(items) })
    }

    fn put(&self, item: Item) -> BoxFuture<'_, Result<(), ShoppingError>> {
        let mut l = self.inner.lock().unwrap();
        l.cleared.retain(|c| c.id != item.id);
        match l.items.iter_mut().find(|i| i.id == item.id) {
            Some(existing) => *existing = item,
            None => l.items.push(item),
        }
        Box::pin(async { Ok(()) })
    }

    fn clear(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>> {
        let mut l = self.inner.lock().unwrap();
        let (gone, keep): (Vec<Item>, Vec<Item>) = std::mem::take(&mut l.items)
            .into_iter()
            .partition(|i| i.done && ids.contains(&i.id));
        l.items = keep;
        l.cleared.retain(|c| !gone.iter().any(|g| g.id == c.id));
        l.cleared.extend(gone);
        Box::pin(async { Ok(()) })
    }

    fn restore(&self, ids: Vec<String>) -> BoxFuture<'_, Result<(), ShoppingError>> {
        let mut l = self.inner.lock().unwrap();
        let (back, keep): (Vec<Item>, Vec<Item>) = std::mem::take(&mut l.cleared)
            .into_iter()
            .partition(|c| ids.contains(&c.id));
        l.cleared = keep;
        for item in back {
            if !l.items.iter().any(|i| i.id == item.id) {
                l.items.push(item);
            }
        }
        Box::pin(async { Ok(()) })
    }
}
