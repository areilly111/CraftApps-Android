//! The storage manager ([`StorageHost`], `storage.*` commands, Settings ▸ Disk ▸ Browser
//! Storage): the origin's usage and quota (`navigator.storage.estimate()`), persistent storage
//! (`navigator.storage.persist()`), what the store holds by kind ([`crate::store::FileKind`])
//! and the disk cache ([`crate::diskcache`]); clearing each.
//!
//! The browser answers asynchronously: [`StorageHost::info`] returns the latest estimate and
//! asks for a new one at most once a second.

use std::cell::RefCell;

use effectcraft_engine::storage::StorageHost;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::store::FileKind;

#[wasm_bindgen(module = "/js/host.js")]
extern "C" {
    #[wasm_bindgen(js_name = storePersist)]
    fn store_persist() -> js_sys::Promise;
}

#[derive(Default)]
struct Estimate {
    usage: u64,
    quota: u64,
    persisted: bool,
    /// When the last refresh started (ms).
    asked: f64,
    busy: bool,
}

thread_local! {
    static EST: RefCell<Estimate> = RefCell::new(Estimate::default());
}

/// Refresh the estimate in the background (at most once a second).
pub fn refresh() {
    let now = js_sys::Date::now();
    let go = EST.with(|e| {
        let mut e = e.borrow_mut();
        if e.busy || now - e.asked < 1000.0 {
            return false;
        }
        e.busy = true;
        e.asked = now;
        true
    });
    if !go {
        return;
    }
    wasm_bindgen_futures::spawn_local(async {
        let v = JsFuture::from(crate::persist::store_estimate()).await.ok();
        EST.with(|e| {
            let mut e = e.borrow_mut();
            e.busy = false;
            if let Some(v) = v {
                let get = |k: &str| js_sys::Reflect::get(&v, &k.into()).ok();
                e.usage = get("usage").and_then(|x| x.as_f64()).unwrap_or(0.0) as u64;
                e.quota = get("quota").and_then(|x| x.as_f64()).unwrap_or(0.0) as u64;
                e.persisted = get("persisted").and_then(|x| x.as_bool()).unwrap_or(false);
            }
        });
    });
}

/// The origin's quota in bytes, when known.
pub fn quota() -> Option<u64> {
    EST.with(|e| Some(e.borrow().quota).filter(|q| *q > 0))
}

/// The web app's [`StorageHost`].
pub struct WebStorage;

impl StorageHost for WebStorage {
    fn info(&self) -> Value {
        refresh();
        let (usage, quota, persisted) = EST.with(|e| {
            let e = e.borrow();
            (e.usage, e.quota, e.persisted)
        });
        json!({
            "backend": crate::persist::backend(),
            "usage": usage,
            "quota": quota,
            "persisted": persisted,
            "files": crate::files::STORE.usage(),
            "diskCache": crate::diskcache::stats(),
            "error": crate::persist::last_error(),
        })
    }

    fn request_persist(&self) -> Value {
        wasm_bindgen_futures::spawn_local(async {
            let granted = JsFuture::from(store_persist()).await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
            EST.with(|e| e.borrow_mut().persisted = granted);
            crate::repaint();
        });
        json!({"requested": true, "persisted": EST.with(|e| e.borrow().persisted)})
    }

    fn clear(&self, what: &str) -> Result<Value, String> {
        let kinds: Vec<FileKind> = match what {
            "diskCache" => vec![],
            "all" => vec![FileKind::Media, FileKind::Project, FileKind::AutoSave],
            k => vec![FileKind::from_name(k).ok_or_else(|| format!("unknown storage kind {k}"))?],
        };
        let (mut entries, mut bytes) = (0u64, 0u64);
        if matches!(what, "diskCache" | "all") {
            let (n, b) = crate::diskcache::clear();
            entries += n;
            bytes += b;
        }
        for k in kinds {
            let (n, b) = crate::files::STORE.clear_kind(k);
            entries += n;
            bytes += b;
        }
        // The estimate catches up on the next refresh.
        EST.with(|e| e.borrow_mut().asked = 0.0);
        Ok(json!({"cleared": what, "entries": entries, "bytes": bytes}))
    }
}
