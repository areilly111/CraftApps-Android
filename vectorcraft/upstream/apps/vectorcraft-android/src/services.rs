//! Android platform services for the VectorCraft port.
//!
//! Mirrors `apps/vectorcraft/src/main.rs` (`services()`), minus everything
//! desktop-only: rfd dialogs (the egui browser in `file_browser.rs` answers
//! through `Services::start_pick` instead), the system clipboard, the control
//! server and automation, printing, and revealing files in a file manager.
//! Reads and writes go through the same paths the app uses everywhere
//! (`Services::read` / `write` / `write_shared`), with `content://` URIs
//! (system picker saves) routed through SAF.

use crate::file_browser::FileBrowser;
use craftapps_shared::saf;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use vectorcraft_ui_egui::Services;
use vectorcraft_ui_egui::picks::{PickAnswer, PickRequest};

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename),
/// exactly like the desktop's `fileio::write_atomic`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    vectorcraft_format::write_atomic(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read a path: `content://` URIs (the system picker's choices) come back
/// through SAF; plain paths are read from the filesystem.
fn read_path(path: &str) -> Result<Vec<u8>, String> {
    if path.starts_with("content://") {
        saf::saf_read_uri(path).map(|(_, bytes)| bytes)
    } else {
        std::fs::read(path).map_err(|e| format!("couldn't read {path}: {e}"))
    }
}

/// Write a path: `content://` URIs (system picker saves) go out through SAF;
/// plain paths go through the atomic file write.
fn write_path(path: &str, bytes: &[u8]) -> Result<(), String> {
    if path.starts_with("content://") {
        saf::saf_write_uri(path, bytes)
    } else {
        write_atomic(Path::new(path), bytes)
    }
}

pub fn android_services(browser: Arc<Mutex<FileBrowser>>, cache_dir: PathBuf) -> Services {
    let browser_pick = browser.clone();
    let cache_pick = cache_dir.clone();
    Services {
        read: Some(Box::new(|p: &str| read_path(p))),
        write: Some(Box::new(|p: &str, bytes: &[u8]| write_path(p, bytes))),
        // Background Save and Export write from worker threads (the same
        // routing as `write`; the JNI bridge attaches threads as needed).
        write_shared: Some(Arc::new(|p: &str, bytes: &[u8]| write_path(p, bytes))),
        // The inline dialogs can't block the UI thread: the real file dialogs
        // go through `start_pick` below (the app reruns whatever asked once
        // the browser answers). These exist so `picks::can` sees the kinds as
        // available; called with no entry running they answer "cancelled".
        pick_open: Some(Box::new(|_: &vectorcraft_ui_egui::FilePick| None)),
        pick_save: Some(Box::new(|_: &vectorcraft_ui_egui::FilePick| None)),
        pick_folder: Some(Box::new(|| None)),
        pick_open_multi: Some(Box::new(Vec::new)),
        // The app's file dialogs, answered by the in-app browser: what asked
        // runs again with the picked paths (the desktop's off-thread rfd path).
        start_pick: Some(Box::new(move |request: PickRequest| {
            let (tx, rx) = std::sync::mpsc::channel::<PickAnswer>();
            match browser_pick.lock() {
                Ok(mut b) => {
                    b.push(request, tx, cache_pick.clone());
                    Some(rx)
                }
                Err(_) => None,
            }
        })),
        // Help → Discord / website / GitHub, the Discord button, About links.
        open_url: Some(Box::new(|url: &str| {
            let _ = craftapps_shared::jni_bridge::open_url(url);
        })),
        ..Default::default()
    }
}
