//! File browsing for the PdfCraft Android port.
//!
//! PdfCraft's UI asks for files through `rfd::AsyncFileDialog`. On Android
//! `rfd` is swapped for the in-tree shim (`crates/rfd-shim`), which hands
//! each pick to the handler installed here. The handler launches the matching
//! Storage Access Framework picker and delivers the result:
//!
//! - open (single): `ACTION_OPEN_DOCUMENT` → the document's bytes are staged
//!   into the cache and the pick resolves to a real path;
//! - open (multiple): same, multi-select; the SAF layer already stages;
//! - folder: `ACTION_OPEN_DOCUMENT_TREE` → a primary-volume tree maps to
//!   its real `/sdcard/...` path (readable with `std::fs`, so pinning and
//!   Combine folder adds stay live); any other tree is enumerated and
//!   every file staged into the cache, preserving relative paths, and
//!   the pick resolves to the staging directory;
//! - save: `ACTION_CREATE_DOCUMENT` → the pick resolves to the returned
//!   `content://` URI, which `write_atomically` writes through the SAF writer.
//!
//! Output-folder picks (`Ask::FolderOut`) go through the same SAF folder
//! bridge as `PickKind::Folder` (see `launch_pick`): the picked tree maps
//! to its real path on the primary volume and becomes that purpose's
//! output directory; cancel falls back to the deterministic output base
//! (see `services`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craftapps_shared::saf;
use craftapps_shared::storage::StorageManager;
use pdfcraft_ui_egui::android::{PickKind, PickRequest};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Install the shim's pick handler. Call once from `android_main` after the
/// JNI bridge and storage are up.
pub fn install_pick_handler(storage: Arc<StorageManager>) {
    let inbox = storage.saf_inbox_dir().to_path_buf();
    pdfcraft_ui_egui::android::set_pick_handler(move |req: PickRequest| {
        let (tx, rx) = std::sync::mpsc::channel();
        let result = launch_pick(&req, &storage, &inbox, tx);
        if let Err(e) = result {
            log::error!("file picker failed to launch: {e}");
        }
        rx
    });
}

/// Map the shim's filter extensions to MIME types for
/// `ACTION_OPEN_DOCUMENT`. Unmapped extensions fall back to `*/*`.
fn mime_types(req: &PickRequest) -> Vec<String> {
    let mut mimes: Vec<String> = req
        .filters
        .iter()
        .flat_map(|(_, exts)| exts.iter())
        .filter_map(|e| saf::mime_for_extension(e).map(str::to_string))
        .collect();
    mimes.sort();
    mimes.dedup();
    if mimes.is_empty() {
        mimes.push("*/*".to_string());
    }
    mimes
}

/// MIME type for a save pick, from the suggested file name's extension.
fn save_mime(req: &PickRequest) -> String {
    let ext = req
        .file_name
        .as_deref()
        .and_then(|n| Path::new(n).extension())
        .and_then(|e| e.to_str())
        .unwrap_or("");
    saf::mime_for_extension(ext).unwrap_or("application/octet-stream").to_string()
}

fn launch_pick(
    req: &PickRequest,
    storage: &Arc<StorageManager>,
    inbox: &Path,
    tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
) -> Result<(), String> {
    match req.kind {
        PickKind::File | PickKind::Files => {
            let multiple = matches!(req.kind, PickKind::Files);
            let reply = open_reply(tx, inbox.to_path_buf());
            saf::set_pending_open(reply, multiple, storage.clone());
            saf::launch_open_document(multiple, &mime_types(req))?;
        }
        PickKind::Folder => {
            let reply = tree_reply(tx, inbox.to_path_buf());
            saf::set_pending_tree(Some(reply), storage.clone());
            saf::launch_open_document_tree_bridge()?;
        }
        PickKind::Save => {
            let name = req.file_name.clone().unwrap_or_else(|| "document.pdf".to_string());
            let reply = save_reply(tx);
            saf::set_pending_create(reply, name.clone(), storage.clone());
            saf::launch_create_document(&name, &save_mime(req))?;
        }
    }
    Ok(())
}

/// Adapter: single/multi open → real paths. `Contents` (single open) is
/// staged into the inbox; `Paths` (multi) is already staged by the SAF layer.
fn open_reply(
    tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
    inbox: PathBuf,
) -> Box<dyn saf::DialogReply> {
    struct Reply {
        tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
        inbox: PathBuf,
    }
    impl saf::DialogReply for Reply {
        fn send(self: Box<Self>, answer: Option<saf::DialogAnswer>) {
            let paths = match answer {
                Some(saf::DialogAnswer::Paths(paths)) => {
                    paths.into_iter().map(PathBuf::from).collect()
                }
                Some(saf::DialogAnswer::Contents(name, bytes)) => {
                    let safe_name = sanitize_file_name(&name);
                    let path = self.inbox.join(safe_name);
                    match std::fs::write(&path, &bytes) {
                        Ok(()) => vec![path],
                        Err(e) => {
                            log::error!("couldn't stage picked file: {e}");
                            Vec::new()
                        }
                    }
                }
                _ => Vec::new(),
            };
            let _ = self.tx.send(paths);
        }
    }
    let _ = std::fs::create_dir_all(&inbox);
    Box::new(Reply { tx, inbox })
}

/// Adapter: folder pick → enumerate the SAF tree, stage every file into a
/// fresh cache directory (relative paths preserved), resolve to that dir.
fn tree_reply(
    tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
    inbox: PathBuf,
) -> Box<dyn saf::DialogReply> {
    struct Reply {
        tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
        inbox: PathBuf,
    }
    impl saf::DialogReply for Reply {
        fn send(self: Box<Self>, answer: Option<saf::DialogAnswer>) {
            let staged = match answer {
                Some(saf::DialogAnswer::Folder(uri)) => {
                    // A primary-volume tree maps to a real path the engine
                    // can read with `std::fs` (pinning, Combine folder adds):
                    // prefer it when it is readable, so the folder stays
                    // live instead of a one-off cache staging. Anything else
                    // (SD card, another provider, no All-files access) falls
                    // back to staging, as before.
                    // Edition 2021: no let-chains; filter+map keeps the
                    // same semantics (primary path only when readable).
                    match external_storage_path(&uri)
                        .filter(|path| std::fs::read_dir(path).is_ok())
                        .map(PathBuf::from)
                    {
                        Some(path) => Some(path),
                        None => stage_tree(&uri, &self.inbox).unwrap_or_else(|e| {
                            log::error!("couldn't stage picked folder: {e}");
                            None
                        }),
                    }
                }
                _ => None,
            };
            let _ = self.tx.send(staged.into_iter().collect());
        }
    }
    Box::new(Reply { tx, inbox })
}

/// Adapter: save pick → the `content://` URI, as a path-like string.
/// `editing::write_atomically` writes it through the SAF writer.
fn save_reply(tx: std::sync::mpsc::Sender<Vec<PathBuf>>) -> Box<dyn saf::DialogReply> {
    struct Reply {
        tx: std::sync::mpsc::Sender<Vec<PathBuf>>,
    }
    impl saf::DialogReply for Reply {
        fn send(self: Box<Self>, answer: Option<saf::DialogAnswer>) {
            let paths = match answer {
                Some(saf::DialogAnswer::SaveTo(uri)) => vec![PathBuf::from(uri)],
                _ => Vec::new(),
            };
            let _ = self.tx.send(paths);
        }
    }
    Box::new(Reply { tx })
}

/// Recursively enumerate `tree_uri` and stage every file under
/// `inbox/tree-<n>/`, preserving relative paths. Returns the staging dir.
fn stage_tree(tree_uri: &str, inbox: &Path) -> Result<Option<PathBuf>, String> {
    let seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dest = inbox.join(format!("tree-{seq}"));
    let mut staged_any = false;
    stage_tree_into(tree_uri, None, &dest, &mut staged_any)?;
    if staged_any {
        Ok(Some(dest))
    } else {
        Ok(None)
    }
}

fn stage_tree_into(
    tree_uri: &str,
    parent_uri: Option<&str>,
    dest_dir: &Path,
    staged_any: &mut bool,
) -> Result<(), String> {
    for entry in saf::saf_tree_children(tree_uri, parent_uri)? {
        if entry.is_dir {
            let sub = dest_dir.join(sanitize_file_name(&entry.name));
            stage_tree_into(tree_uri, Some(&entry.uri), &sub, staged_any)?;
        } else {
            let (name, bytes) = saf::saf_read_uri(&entry.uri)?;
            let _ = std::fs::create_dir_all(dest_dir);
            let display = if name.is_empty() { entry.name.clone() } else { name };
            let path = dest_dir.join(sanitize_file_name(&display));
            std::fs::write(&path, &bytes).map_err(|e| format!("couldn't stage {}: {e}", path.display()))?;
            *staged_any = true;
        }
    }
    Ok(())
}

/// Strip path separators from a staged file name so tree entries can't
/// escape the staging directory.
fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    if base.is_empty() {
        "file".to_string()
    } else {
        base
    }
}

/// `content://com.android.externalstorage.documents/...` → a filesystem
/// path on the primary volume, or `None` for any other provider/volume.
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
                    b'A'..=b'F' => Some(b - b'a' + 10),
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
