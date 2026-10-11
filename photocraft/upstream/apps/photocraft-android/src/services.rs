//! Android platform services: in-app file dialogs, filesystem, codecs.
//!
//! Mirrors `apps/photocraft/src/services.rs` (`native()`), minus everything
//! desktop-only: rfd dialogs (the egui browser in `file_browser.rs` answers
//! instead), the clipboard, Photoshop shortcut import, display profiles, the
//! control server and automation. Crash recovery, prefs and the preset store
//! work exactly like the desktop's, rooted at the Android config dir.

use crate::file_browser::{FileBrowser, GalleryFlag, gallery_mime};
use crate::jni_bridge;
use crate::storage::StorageManager;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{Recoverable, Services};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub struct AndroidDirs {
    pub config: PathBuf,
}

fn prefs_file(dirs: &AndroidDirs) -> PathBuf {
    dirs.config.join("preferences.json")
}

/// Last-resort panic guard for import/export: a panic becomes an error the UI
/// shows, so open documents stay in place (same contract as the desktop's
/// `crash_guard::guard`).
fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        log::error!("{what}: escaped panic, reported as an error");
        Err(format!("{what} failed with an internal error (logged); your open documents are unchanged"))
    })
}

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename,
/// directory fsync), exactly like the desktop's `write_atomic`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// After a successful save, insert a Gallery copy when the Save dialog asked
/// for one (consumes the flag).
fn after_write(gallery: &GalleryFlag, path: &str, bytes: &[u8]) {
    let take = match gallery.lock() {
        Ok(mut g) => {
            let take = g.as_ref().is_some_and(|want| want.to_string_lossy() == path);
            if take { g.take() } else { None }
        }
        Err(_) => None,
    };
    if take.is_some() {
        let path = Path::new(path);
        if let Some(mime) = gallery_mime(path) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image.png".to_string());
            match jni_bridge::gallery_insert(&name, mime, bytes) {
                Ok(uri) => log::info!("gallery copy: {uri}"),
                Err(e) => log::warn!("gallery copy failed: {e}"),
            }
        }
    }
}

fn export_options(settings: &photocraft_ui_egui::ExportSettings) -> photocraft_io::ExportOptions {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(q) = settings.jpeg_quality {
        opts.encode.jpeg_quality = q;
    }
    opts.encode.webp_lossless = settings.webp_lossless;
    if let Some(q) = settings.webp_quality {
        opts.encode.webp_quality = q;
    }
    opts.tiff_layers = settings.tiff_layers;
    opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
    opts
}

/// A save on a worker thread (`Services::save_file`): encode, then write
/// atomically, like the desktop's. `path` may be a `content://` URI when the
/// user saved through the system picker: the format is sniffed from the
/// recorded display name and the bytes go out through SAF.
fn save_file(
    gallery: &GalleryFlag,
    storage: &StorageManager,
    doc: &photocraft_doc::Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
) -> Result<Vec<String>, String> {
    ctl.progress(0.0, "Encoding");
    let opts = export_options(settings);
    let name_for_ext =
        if path.starts_with("content://") { storage.name_for_id(path) } else { path.to_string() };
    let r =
        guard("Save", || photocraft_io::export(doc, &name_for_ext, &opts).map_err(|e| e.to_string()))?;
    ctl.commit()?;
    ctl.progress(0.0, "Writing");
    storage.write_id(path, &r.bytes)?;
    after_write(gallery, path, &r.bytes);
    Ok(r.warnings)
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

/// Crash recovery into `<config>/Recovery` (same semantics as the desktop's).
fn recovery_services(dir: PathBuf) -> Services {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(Some(RecoveryStore::new(dir))));
    let (s1, s2, s4) = (store.clone(), store.clone(), store.clone());
    Services {
        autosave: Some(Box::new(move |doc: &Arc<photocraft_doc::Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
        })),
        autosave_results: Some(Box::new(move || with_store(&s4, |s| s.take_completed()).unwrap_or_default())),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            photocraft_format::list_recovery(&recover_dir)
                .into_iter()
                .map(|e| Recoverable {
                    key: e.info.key.clone(),
                    name: e.info.document_name.clone(),
                    path: e.info.original_path.clone(),
                    load: Box::new(move || photocraft_format::recover(&e).map_err(|e| e.to_string())),
                })
                .collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

pub fn android_services(
    dirs: &AndroidDirs,
    browser: Arc<Mutex<FileBrowser>>,
    gallery: GalleryFlag,
    storage: Arc<StorageManager>,
) -> Services {
    let gallery_write = gallery.clone();
    let gallery_save = gallery.clone();
    let storage_save = storage.clone();
    let storage_write = storage.clone();
    let prefs = prefs_file(dirs);
    let prefs_load = prefs.clone();
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8]| {
            guard("Open", || photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))
        })),
        export: Some(Box::new(|doc: &photocraft_doc::Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let opts = export_options(settings);
            guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        save_file: Some(Arc::new(move |doc, path, settings, ctl| {
            save_file(&gallery_save, &storage_save, doc, path, settings, ctl)
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| {
            match browser.lock() {
                Ok(mut b) => b.push(request, reply),
                Err(_) => reply.send(None),
            }
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            // Routes `content://` URIs (system picker saves) through SAF;
            // plain paths go through the atomic file write, as before.
            storage_write.write_id(path, bytes)?;
            after_write(&gallery_write, path, bytes);
            Ok(())
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        open_url: Some(Box::new(|url: &str| jni_bridge::open_url(url))),
        load_prefs: Some(Box::new(move || std::fs::read_to_string(&prefs_load).ok())),
        save_prefs: Some(Box::new(move |text: &str| write_atomic(&prefs, text.as_bytes()))),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        ..recovery_services(dirs.config.join("Recovery"))
    }
}
