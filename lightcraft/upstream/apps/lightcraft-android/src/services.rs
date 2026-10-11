//! Android platform services for LightCraft.
//!
//! Mirrors `apps/lightcraft/src/main.rs` (`services()`), minus everything
//! desktop-only: rfd dialogs (the async [`Picker`][lightcraft_ui_egui::pick::Picker]
//! below is answered by the in-app browser in [`crate::file_browser`] plus the
//! system SAF pickers), the synchronous in-dialog pickers (their buttons stay
//! disabled — the UI already handles `None` there), open-with/reveal (desktop
//! only) and the log file (Android logs to logcat). Crash-safe atomic writes
//! come from the engine, exactly like the desktop's.

use crate::file_browser::FileBrowser;
use craftapps_shared::{jni_bridge, saf, storage::StorageManager};
use lightcraft_ui_egui::Services;
use std::sync::{Arc, Mutex};

/// Route a write at `path`: `content://` URIs (system picker saves) go out
/// through SAF; plain paths go through the engine's atomic file write, as on
/// the desktop.
fn write_at(path: &str, bytes: &[u8]) -> Result<(), String> {
    if path.starts_with("content://") {
        saf::saf_write_uri(path, bytes)
    } else {
        lightcraft_engine::export::write_file(path, bytes)
    }
}

pub fn android_services(
    browser: Arc<Mutex<FileBrowser>>,
    storage: Arc<StorageManager>,
) -> Services {
    let _ = storage;
    let write_shared = move |path: &str, bytes: &[u8]| write_at(path, bytes);
    Services {
        // Commands' file dialogs run off the command and answer later: the
        // in-app browser (drawn by the shell) answers through the channel, and
        // `pick::poll` re-runs the command with the answer (`poll` repaints
        // every 250 ms while a pick is pending, so no explicit repaint
        // plumbing is needed; SAF returns resume the activity anyway).
        // "Browse other locations…" / "Save elsewhere…" hand the reply to the
        // SAF pickers.
        picker: Some(Box::new(move |req: lightcraft_ui_egui::pick::PickRequest| {
            let (tx, rx) = std::sync::mpsc::channel();
            match browser.lock() {
                Ok(mut b) => {
                    b.push(req, tx);
                    Ok(rx)
                }
                Err(_) => Err("the file dialog is busy".to_string()),
            }
        })),
        // The synchronous in-dialog pickers have no Android equivalent (a
        // system picker can't answer synchronously): with `picker` set, the
        // commands use it, and the dialog buttons check `is_some()` and stay
        // disabled. `pick_folder` likewise: the library lives at a fixed
        // `<config>/Library` on this port, so Settings › General › Open
        // Library… is unavailable (the UI says so).
        pick_files: None,
        pick_denoise_model: None,
        pick_model_file: None,
        pick_preset_files: None,
        pick_tracklog: None,
        pick_display_profile: None,
        save_preset_file: None,
        pick_keyword_list: None,
        save_keyword_list: None,
        pick_curve_preset_files: None,
        save_curve_preset_file: None,
        pick_folder: None,
        pick_lightroom_catalog: None,
        write: Some(Box::new(write_at)),
        // Thread-safe writer: with it, UI-started exports run in the background.
        write_shared: Some(Arc::new(write_shared)),
        // PNG encoder (the host links an image encoder; the UI crate stays codec-free).
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(
                &lightcraft_codecs::EncodeImage::rgba8(img),
                &lightcraft_codecs::EncodeMeta::default(),
            )
            .unwrap_or_default()
        })),
        // Desktop only: no external editor, no file-manager reveal on Android.
        reveal: None,
        open_with: None,
        // Android logs to logcat (android_logger); there is no log file, so
        // Help ▸ Open Log Folder stays disabled.
        log_file: None,
        open_url: Some(Box::new(|url: &str| {
            if !url.starts_with("https://") {
                return Err("only https links are opened".into());
            }
            jni_bridge::open_url(url)
        })),
        // The library is a folder on disk: backed up with the user's other files.
        backup_library: None,
        restore_library: None,
        ..Default::default()
    }
}
