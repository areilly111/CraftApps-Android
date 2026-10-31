//! Android file services for SoundCraft: the app's three synchronous pick
//! hooks, served by the suite's one file method — the shared file bridge.
//!
//! SoundCraft's `Services` hooks are synchronous (`Fn(&str, …) ->
//! Option<String>`): the desktop answers them with a blocking `rfd` dialog.
//! On Android each hook writes a bridge request, the Activity (polling on
//! the main thread) launches the system SAF picker, and the hook waits for
//! the bridge's answer on the calling thread — the system picker is in
//! front meanwhile, so the paused frame behind it is never seen (the
//! DesignCraft arrangement). These hooks own the bridge response file end
//! to end, so the shell must NOT call `saf::poll_bridge()`: it would take
//! the responses and drop them for want of a pending request.
//!
//! Answers are plain filesystem paths, because the engine reads and writes
//! with `std::fs`:
//! - **Open** — the Activity copies the pick into the bridge inbox, so the
//!   answer already is a real path.
//! - **Save / Folder** — the bridge answers with a `content://` URI; it is
//!   resolved to a real path when it names a file on the primary
//!   external-storage volume (the common case, and the one "All files
//!   access" covers). A save URI that cannot be resolved falls back —
//!   logged — to the suite media folder under the chosen name, so work is
//!   never lost; a folder URI that cannot be resolved makes the pick read
//!   as cancelled, since a folder the engine cannot address is useless.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use craftapps_shared::picker_bridge::{PickerKind, PickerResponse, poll_response, request_picker};
use craftapps_shared::saf;
use craftapps_shared::storage::StorageManager;
use soundcraft_ui_egui::Services;

/// How long a picker may stay open before the pick is treated as abandoned.
/// Generous: importing can mean browsing a big library.
const PICK_TIMEOUT: Duration = Duration::from_secs(600);

/// How often a waiting hook polls the bridge for the answer.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

/// Audio extensions the desktop offers for imports (its `pick_open`
/// default filter), reused so the Android picker offers the same kinds.
const AUDIO_EXTS: &[&str] = &[
    "wav", "wave", "bwf", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "m4a", "m4b",
    "aac", "mp4", "caf", "mkv", "webm",
];

/// Build the [`Services`] the UI crate needs, backed by the file bridge.
/// `media_dir` is the suite's shared media root (`/sdcard/CraftApps/Media`);
/// saves land under its `SoundCraft` folder when a pick cannot be mapped
/// back to a filesystem path.
pub fn build_services(storage: Option<Arc<StorageManager>>, media_dir: PathBuf) -> Services {
    let save_storage = storage.clone();
    Services {
        pick_open: Some(Box::new(move |purpose: &str, exts: &[&str]| {
            pick_open(purpose, exts).map(std::path::PathBuf::from)
        })),
        pick_save: Some(Box::new(move |purpose: &str, default: &std::path::Path| {
            pick_save(purpose, default, &save_storage, &media_dir)
        })),
        pick_folder: Some(Box::new(move |start: &std::path::Path| pick_folder(start, &storage))),
    }
}

/// Pick an existing file. The bridge answer's paths are real files (the
/// Activity copied the pick into the bridge inbox); the hook returns one
/// path, so a multi-pick is not offered.
fn pick_open(purpose: &str, exts: &[&str]) -> Option<String> {
    // Extensions to offer: the caller's when it passes any (the sampler
    // does), else the desktop's per-purpose defaults.
    let effective: &[&str] = if !exts.is_empty() {
        exts
    } else {
        match purpose {
            "session.open" => &["scraft"],
            "file.import_midi" => &["mid", "midi", "smf"],
            _ => AUDIO_EXTS,
        }
    };
    let response = await_response(PickerKind::Open, mimes_for_extensions(effective), String::new(), false)?;
    if response.cancelled {
        return None;
    }
    let mut paths = response.paths.into_iter();
    if purpose == "session.open" {
        // `.scraft` has no registered MIME type, so the pick was
        // unfiltered; the extension decides (the EffectCraft project
        // pattern).
        return paths.find(|p| p.to_ascii_lowercase().ends_with(".scraft"));
    }
    paths.next()
}

/// Pick where to write a file (a session save, a bounce…). `default` is
/// the path the app suggests; only its file name reaches the picker.
fn pick_save(purpose: &str, default: &Path, storage: &Option<Arc<StorageManager>>, media_dir: &Path) -> Option<PathBuf> {
    let _ = purpose; // The picker is the same for every save purpose.
    let base = default
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| default.to_string_lossy().into_owned());
    let ext = base.rsplit('.').next().unwrap_or("");
    let mimes = match saf::mime_for_extension(ext) {
        Some(mime) => vec![mime.to_string()],
        None => Vec::new(),
    };
    let response = await_response(PickerKind::Save, mimes, base.clone(), false)?;
    if response.cancelled {
        return None;
    }
    let picked_name = response.name.clone().unwrap_or_else(|| base.clone());
    if let Some(uri) = response.uri.as_deref() {
        if let Some(storage) = storage {
            storage.record_uri_name(uri, &picked_name);
        }
        if let Some(path) = external_storage_path(uri) {
            log::info!("save target resolved: {path}");
            return Some(PathBuf::from(path));
        }
        log::warn!("save URI {uri} is not a primary-volume path; using the fallback folder");
    } else {
        log::warn!("save picker answered without a URI; using the fallback folder");
    }
    // Fallback: the suite media folder under the chosen name, so the save
    // still lands somewhere the user can reach.
    let fallback = media_dir.join("SoundCraft").join(sanitize_name(&picked_name));
    if let Some(dir) = fallback.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::error!("couldn't create the fallback save folder {}: {e}", dir.display());
            return None;
        }
    }
    Some(fallback)
}

/// Pick a folder. The tree URI is remembered as the app's SAF tree grant
/// and resolved to a real path for the engine.
fn pick_folder(_start: &Path, storage: &Option<Arc<StorageManager>>) -> Option<PathBuf> {
    let response = await_response(PickerKind::Folder, Vec::new(), String::new(), false)?;
    if response.cancelled {
        return None;
    }
    let Some(uri) = response.uri.as_deref() else {
        log::error!("folder picker answered without a URI");
        return None;
    };
    if let Some(storage) = storage {
        if let Err(e) = storage.set_sd_tree_uri(uri) {
            log::warn!("couldn't remember the picked folder grant: {e}");
        }
    }
    match external_storage_path(uri) {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            log::error!("picked folder {uri} is not a primary-volume path; the pick is unusable");
            None
        }
    }
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

/// Extensions → MIME types for the open picker, all or nothing: when any
/// extension has no known type the pick is left unfiltered, because a
/// partial `EXTRA_MIME_TYPES` list would hide the files whose types are
/// missing (the importer itself rejects anything it cannot read).
fn mimes_for_extensions(exts: &[&str]) -> Vec<String> {
    let mut mimes = Vec::with_capacity(exts.len());
    for ext in exts {
        match saf::mime_for_extension(ext) {
            Some(mime) => mimes.push(mime.to_string()),
            None => return Vec::new(),
        }
    }
    mimes
}

/// `content://com.android.externalstorage.documents/...` -> a filesystem
/// path on the primary volume, or `None` for any other provider/volume.
/// Same mapping the FilmCraft, EffectCraft and DesignCraft ports use.
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
