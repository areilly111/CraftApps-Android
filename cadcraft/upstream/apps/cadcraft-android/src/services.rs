//! Android file services for CADCraft: the app's synchronous pick
//! hooks, served by the suite's one file method — the shared file bridge.
//!
//! CADCraft's `Services` hooks are synchronous (`Fn() -> Option<String>`
//! for open, `Fn(&str) -> Option<String>` for save and APPLOAD): the
//! desktop answers them with a blocking `rfd` dialog. On Android each hook
//! writes a bridge request, the Activity (polling on the main thread)
//! launches the system SAF picker, and the hook waits for the bridge's
//! answer on the calling thread — the system picker is in front meanwhile,
//! so the paused frame behind it is never seen (the DesignCraft /
//! SoundCraft arrangement). These hooks own the bridge response file end
//! to end, so the shell must NOT call `saf::poll_bridge()`: it would take
//! the responses and drop them for want of a pending request.
//!
//! CADCraft's `Services` has no folder hook (unlike SoundCraft's): there is
//! no `pick_folder` to serve, and no folder pick is faked in. Answers are
//! plain filesystem paths, because the engine reads and writes with
//! `std::fs`:
//! - **Open** — the Activity copies the pick into the bridge inbox, so the
//!   answer already is a real path. The pick is unfiltered: DWG, DXF, DWT
//!   and SVGZ have no registered MIME type, so a filtered pick would hide
//!   them; the engine rejects anything it cannot read.
//! - **Save** — the bridge answers with a `content://` URI; it is resolved
//!   to a real path when it names a file on the primary external-storage
//!   volume (the common case, and the one "All files access" covers). A
//!   save URI that cannot be resolved falls back — logged — to the suite
//!   media folder under the chosen name, so work is never lost.
//! - **APPLOAD (LISP)** — the bridge has no start folder, so the hook's
//!   `dir` argument is ignored and the pick starts wherever the system
//!   picker opens; the answer is filtered to `.lsp`, as the desktop's
//!   filter does. The picked file is the bridge inbox copy, so APPLOAD
//!   reads the copy, not the original.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cadcraft_ui_egui::Services;
use craftapps_shared::picker_bridge::{PickerKind, PickerResponse, poll_response, request_picker};
use craftapps_shared::saf;
use craftapps_shared::storage::StorageManager;

/// How long a picker may stay open before the pick is treated as abandoned.
/// Generous: opening can mean browsing a big library.
const PICK_TIMEOUT: Duration = Duration::from_secs(600);

/// How often a waiting hook polls the bridge for the answer.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// Build the [`Services`] the UI crate needs, backed by the file bridge.
/// `media_dir` is the suite's shared media root (`/sdcard/CraftApps/Media`);
/// saves land under its `CadCraft` folder when a pick cannot be mapped
/// back to a filesystem path.
pub fn build_services(storage: Option<Arc<StorageManager>>, media_dir: PathBuf) -> Services {
    Services {
        pick_open: Some(Box::new(move || pick_open())),
        pick_save: Some(Box::new(move |name: &str| pick_save(name, &storage, &media_dir))),
        // APPLOAD: AutoLISP routines only; the bridge offers no start
        // folder, so `dir` is ignored.
        pick_lisp: Some(Box::new(move |_dir: &str| pick_lisp())),
        no_file_system: false,
    }
}

/// Pick an existing drawing (or any file the engine can read). The bridge
/// answer's paths are real files (the Activity copied the pick into the
/// bridge inbox); the hook returns one path, so a multi-pick is not offered.
fn pick_open() -> Option<String> {
    let response = await_response(PickerKind::Open, Vec::new(), String::new(), false)?;
    if response.cancelled {
        return None;
    }
    response.paths.into_iter().next()
}

/// Pick where to write a file (a drawing save, an export…). `name` is the
/// name the app suggests; it reaches the picker as the suggested title.
fn pick_save(name: &str, storage: &Option<Arc<StorageManager>>, media_dir: &Path) -> Option<String> {
    let ext = name.rsplit('.').next().unwrap_or("");
    let mimes = match saf::mime_for_extension(ext) {
        Some(mime) => vec![mime.to_string()],
        None => Vec::new(),
    };
    let response = await_response(PickerKind::Save, mimes, name.to_string(), false)?;
    if response.cancelled {
        return None;
    }
    let picked_name = response.name.clone().unwrap_or_else(|| name.to_string());
    if let Some(uri) = response.uri.as_deref() {
        if let Some(storage) = storage {
            storage.record_uri_name(uri, &picked_name);
        }
        if let Some(path) = external_storage_path(uri) {
            log::info!("save target resolved: {path}");
            return Some(path);
        }
        log::warn!("save URI {uri} is not a primary-volume path; using the fallback folder");
    } else {
        log::warn!("save picker answered without a URI; using the fallback folder");
    }
    // Fallback: the suite media folder under the chosen name, so the save
    // still lands somewhere the user can reach.
    let fallback = media_dir.join("CadCraft").join(sanitize_name(&picked_name));
    if let Some(dir) = fallback.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!("couldn't create the fallback save folder {}: {e}", dir.display());
            return None;
        }
    }
    Some(fallback.to_string_lossy().into_owned())
}

/// Pick an AutoLISP file for APPLOAD. The desktop filters to `.lsp`; the
/// bridge pick is unfiltered (no registered MIME type), so the extension
/// decides here, and a non-LISP pick reads as cancelled.
fn pick_lisp() -> Option<String> {
    let response = await_response(PickerKind::Open, Vec::new(), String::new(), false)?;
    if response.cancelled {
        return None;
    }
    response.paths.into_iter().find(|p| p.to_ascii_lowercase().ends_with(".lsp"))
}

// ---------------------------------------------------------------------------
// Bridge plumbing.

/// Write a bridge request and poll until its answer (or the timeout).
fn await_response(
    kind: PickerKind,
    mimes: Vec<String>,
    name: String,
    multiple: bool,
) -> Option<PickerResponse> {
    let id = match request_picker(kind, mimes, name, multiple) {
        Ok(id) => id,
        Err(e) => {
            log::error!("picker request failed: {e}");
            return None;
        }
    };
    let deadline = Instant::now() + PICK_TIMEOUT;
    loop {
        if let Some(response) = poll_response() {
            if response.id == id {
                return Some(response);
            }
            // A stale answer to an earlier request: discard and keep waiting.
            log::warn!("discarding stale picker response (id {})", response.id);
        }
        if Instant::now() >= deadline {
            log::warn!("picker request {id} timed out");
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// `content://com.android.externalstorage.documents/...` -> a filesystem
/// path on the primary volume, or `None` for any other provider/volume.
/// Same mapping the FilmCraft, EffectCraft, DesignCraft and SoundCraft ports use.
///
/// The document id is `primary:<relative/path>`, URL-encoded, carried
/// either as the `document/<id>` or `tree/<id>` segment.
fn external_storage_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("content://com.android.externalstorage.documents/")?;
    let doc_id = rest
        .strip_prefix("document/")
        .or_else(|| rest.strip_prefix("tree/"))
        .unwrap_or(rest);
    let doc_id = percent_decode(doc_id);
    let relative = doc_id.strip_prefix("primary:")?;
    if relative.is_empty() {
        Some("/sdcard".to_string())
    } else {
        Some(format!("/sdcard/{relative}"))
    }
}

/// Minimal percent-decoding for a URI document id.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A display name safe to use as a plain file name.
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}
