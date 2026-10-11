//! Android platform services for DesignCraft.
//!
//! Mirrors `apps/designcraft/src/main.rs` (`services()`), minus everything
//! desktop-only: rfd dialogs (the egui browser in `file_browser.rs` answers
//! opens instead), the control server, logging and the app icon. The port
//! leaves `pick_open` unset so opens take the web path — `open_async` opens
//! the in-app browser and the chosen file arrives through the inbox — while
//! `pick_save` reserves the suggested name in the current media location
//! (DesignCraft's save API is synchronous, so there is no save dialog on
//! Android; see the note there).

use crate::file_browser::FileBrowser;
use craftapps_shared::saf;
use craftapps_shared::storage::{MediaLocation, StorageManager};
use designcraft_ui_egui::{ImportRequest, Inbox, Services};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Last-resort panic guard for import/export: a panic becomes an error the UI
/// shows, so open documents stay in place (same contract as the desktop's
/// `guard` module).
fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("{what}: escaped panic, reported as an error");
        Err(format!("{what} failed with an internal error (logged); your open documents are unchanged"))
    })
}

/// A save name is a bare file name: no slashes, not empty.
fn clean_name(name: &str) -> Option<String> {
    let clean = name.trim();
    (!clean.is_empty() && !clean.contains(['/', '\\'])).then(|| clean.to_string())
}

/// Where `pick_save` puts the document: the current media location, or the
/// internal fallback dir when shared storage isn't granted.
///
/// DesignCraft's save API is synchronous — the in-app browser can't answer it
/// — so saves and exports go straight to the media folder with the suggested
/// name (the Open browser shows the same folder, and Preferences › Storage
/// says where it is). Engine document saves use `std::fs` directly, so a
/// `.designcraft` save on the SD card (a content URI, not a path) is declined
/// with an explanation instead of failing deep in the engine; exports reach
/// `services.write`, which handles content URIs through SAF.
fn save_target(storage: &StorageManager, fallback: Option<&Path>, name: &str) -> Option<String> {
    let clean = clean_name(name)?;
    let is_document = clean.to_ascii_lowercase().ends_with(".designcraft");
    match storage.media_location() {
        MediaLocation::SdCard if is_document => {
            log::warn!("Save As to the SD card isn't supported (the engine needs a file path); switch to tablet storage in Preferences › Storage.");
            None
        }
        MediaLocation::SdCard => storage.reserve(None, &clean).ok(),
        MediaLocation::Tablet => match fallback {
            Some(root) => Some(root.join(&clean).to_string_lossy().into_owned()),
            None => storage.reserve(None, &clean).ok(),
        },
    }
}

/// Storage-aware read: `content://` URIs through SAF, plain paths from the
/// filesystem (mirrors the desktop's `std::fs::read`).
fn read_id(path: &str) -> Result<Vec<u8>, String> {
    if path.starts_with("content://") {
        saf::saf_read_uri(path).map(|(_, bytes)| bytes)
    } else {
        std::fs::read(path).map_err(|e| format!("couldn't read {path}: {e}"))
    }
}

pub fn android_services(
    browser: Arc<Mutex<FileBrowser>>,
    inbox: Inbox,
    storage: Arc<StorageManager>,
    // Plain-fs fallback root when shared storage isn't granted (the v1 layout).
    fallback: Option<PathBuf>,
) -> Services {
    let storage_save = storage.clone();
    let storage_write = storage.clone();
    let browser_open = browser.clone();
    let inbox_open = inbox.clone();
    Services {
        // Unset on purpose: `pick_and_open` then takes the web path and calls
        // `open_async`, which opens the in-app browser.
        pick_open: None,
        pick_save: Some(Box::new(move |name: &str| save_target(&storage_save, fallback.as_deref(), name))),
        read: Some(Box::new(move |path: &str| guard("Read", || read_id(path)))),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            // Routes `content://` URIs (SD-card saves) through SAF; plain
            // paths go through the atomic file write.
            guard("Write", || storage_write.write_id(path, bytes))
        })),
        open_async: Some(Box::new(move |request: ImportRequest| {
            match browser_open.lock() {
                Ok(mut b) => b.open_dialog(request, inbox_open.clone()),
                Err(_) => log::error!("the file browser is unavailable"),
            }
        })),
        download: None,
        inbox: Some(inbox),
        ..Default::default()
    }
}
