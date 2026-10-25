//! Android services for the PdfCraft port.
//! Persistent data lives under getFilesDir(); external documents use SAF.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craftapps_shared::storage::{StorageAppInfo, StorageManager};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const APP_INFO: StorageAppInfo = StorageAppInfo { app_name: "PdfCraft", settings_folder: "PdfCraft" };

/// Media subdirectory for this app's documents.
const MEDIA_SUBDIR: &str = "PdfCraft";

/// Build the shared [`StorageManager`]. `cache_dir` comes from the JNI bridge
/// (the app's cache dir).
pub fn init_storage(cache_dir: &Path) -> Result<Arc<StorageManager>, String> {
    // Persistent app-private storage: startup must not require all-files access.
    // Open/save outside this directory goes through the Android system picker.
    let private_root = PathBuf::from(craftapps_shared::jni_bridge::files_dir()?);
    let storage = Arc::new(StorageManager::new_at_root(cache_dir, &APP_INFO, &private_root)?);
    storage.ensure_dirs()?;
    // Tell the portable UI where output-folder picks resolve
    // (the app-private `CraftApps/Media/PdfCraft` directory).
    let out_base = app_media_dir(&storage);
    pdfcraft_ui_egui::android::set_output_base(out_base);
    Ok(storage)
}

/// This app's media directory: `<CraftApps>/Media/PdfCraft`.
pub fn app_media_dir(storage: &StorageManager) -> PathBuf {
    // Per-app subdir keeps the suite's apps from colliding in shared storage.
    let dir = storage.tablet_media_dir().join(MEDIA_SUBDIR);
    let _ = std::fs::create_dir_all(&dir);
    dir
}
