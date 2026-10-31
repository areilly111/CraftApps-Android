//! Android shell integration for `pdfcraft-ui-egui`.
//!
//! The Android port (`pdfcraft-android` crate) owns the JNI bridge, the
//! Storage Access Framework pickers and the shared storage layout. This
//! module is the narrow contract between the portable UI and that shell:
//!
//! - [`set_output_base`] — the directory the shell wants output-folder picks
//!   (`Ask::FolderOut`) to fall back to. The shell sets it once at startup
//!   from its `StorageManager`; a user-picked folder (see
//!   [`set_output_dir_for`]) wins over it for its purpose.
//! - [`output_dir`] — resolve a per-purpose output directory: a user-picked
//!   folder first, else under the base, creating it.
//! - [`PickRequest`], [`PickKind`] and [`set_pick_handler`] are re-exported
//!   from the Android `rfd` shim so the shell can implement picks without a
//!   direct dependency on the shim crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

pub use crate::rfd::{PickKind, PickRequest, set_pick_handler};

/// Base directory for output-folder picks, installed by the shell at startup
/// (e.g. `/sdcard/CraftApps/Media/PdfCraft`).
static OUTPUT_BASE: OnceLock<PathBuf> = OnceLock::new();

/// Per-purpose output directories the user picked with the SAF folder picker
/// (see [`set_output_dir_for`]). A picked folder wins over the base below, so
/// later outputs for the same purpose land in the picked folder without
/// asking again.
static OUTPUT_DIR_OVERRIDES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn output_dir_overrides() -> &'static Mutex<HashMap<String, PathBuf>> {
    OUTPUT_DIR_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Install the base directory for [`output_dir`]. Called once by the shell.
pub fn set_output_base(dir: PathBuf) {
    let _ = OUTPUT_BASE.set(dir);
}

/// Remember a user-picked output directory for `purpose`, replacing the
/// deterministic directory [`output_dir`] would otherwise return. Called
/// when an output-folder pick succeeds on Android.
pub fn set_output_dir_for(purpose: &str, dir: PathBuf) {
    let safe = safe_purpose(purpose);
    output_dir_overrides()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(safe, dir);
}

/// Keep a purpose string filesystem-safe (also the override-map key).
fn safe_purpose(purpose: &str) -> String {
    purpose
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Resolve (and create) the output directory for `purpose`: a user-picked
/// directory first (see [`set_output_dir_for`]), else under the base set by
/// [`set_output_base`]. Falls back to the system temp dir when the shell
/// never called [`set_output_base`] (tests).
pub fn output_dir(purpose: &str) -> PathBuf {
    let safe = safe_purpose(purpose);
    if let Some(dir) = output_dir_overrides()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&safe)
        .cloned()
    {
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    let base = OUTPUT_BASE
        .get()
        .cloned()
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(if safe.is_empty() { "output".to_string() } else { safe });
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The output base as a lossy string, for status messages.
pub fn output_base_display() -> String {
    OUTPUT_BASE
        .get()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| std::env::temp_dir().display().to_string())
}

/// True on Android: whether `path` is a SAF `content://` URI rather than a
/// filesystem path.
pub fn is_content_uri(path: &Path) -> bool {
    path.to_string_lossy().starts_with("content://")
}
