//! Android [`Services`] for VectorCraft: SAF-backed async picks, an in-app
//! media browser, content-URI aware file I/O, and the system browser opener.
//!
//! File dialogs go through [`Services::start_pick`]: open/save/folder ask on
//! the in-app media browser first (fast, offline, rooted at the suite's
//! shared media tree), with a "System files…" escape hatch to the Storage
//! Access Framework. The picks machinery (`picks::poll`) picks the answer up
//! on the next frame; the UI thread is never blocked.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use craftapps_shared::{jni_bridge, saf};
use vectorcraft_ui_egui::picks::PickRequest;
use vectorcraft_ui_egui::{FilePick, Services};

use crate::file_browser::{BrowserMode, BrowserRequest, FileBrowser};

/// Filename characters the inbox staging strips.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Reply adapter: turns a SAF [`saf::DialogAnswer`] into the picked paths the
/// picks machinery expects, staging single-file contents into the inbox dir
/// so the engine always sees real filesystem paths.
struct ChannelReply {
    tx: Sender<Vec<String>>,
    inbox: PathBuf,
}

impl ChannelReply {
    fn stage(&self, name: &str, bytes: &[u8]) -> Option<String> {
        let name = safe_name(name);
        let name = if name.is_empty() { "untitled".to_string() } else { name };
        // Avoid collisions: suffix a counter.
        for i in 0..1000u32 {
            let candidate = if i == 0 {
                self.inbox.join(&name)
            } else {
                let stem = Path::new(&name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let ext = Path::new(&name)
                    .extension()
                    .map(|s| format!(".{}", s.to_string_lossy()))
                    .unwrap_or_default();
                self.inbox.join(format!("{stem}-{i}{ext}"))
            };
            if !candidate.exists() {
                if std::fs::create_dir_all(&self.inbox).is_err() {
                    return None;
                }
                return std::fs::write(&candidate, bytes)
                    .ok()
                    .map(|_| candidate.to_string_lossy().to_string());
            }
        }
        None
    }
}

impl saf::DialogReply for ChannelReply {
    fn send(self: Box<Self>, answer: Option<saf::DialogAnswer>) {
        let paths = match answer {
            Some(saf::DialogAnswer::Contents(name, bytes)) => {
                self.stage(&name, &bytes).into_iter().collect()
            }
            Some(saf::DialogAnswer::Paths(paths)) => paths,
            Some(saf::DialogAnswer::SaveTo(uri)) => vec![uri],
            Some(saf::DialogAnswer::Folder(uri)) => vec![uri],
            None => Vec::new(),
        };
        let _ = self.tx.send(paths);
    }
}

/// Shared Android services plus the handle the shell needs to draw the
/// in-app browser each frame.
pub struct AndroidServices {
    pub browser: Arc<Mutex<FileBrowser>>,
    storage: Option<Arc<craftapps_shared::storage::StorageManager>>,
    browser_root: PathBuf,
    inbox: PathBuf,
}

impl AndroidServices {
    pub fn new(
        storage: Option<Arc<craftapps_shared::storage::StorageManager>>,
        browser_root: PathBuf,
        inbox: PathBuf,
    ) -> Self {
        Self {
            browser: Arc::new(Mutex::new(FileBrowser::new(browser_root.clone()))),
            storage,
            browser_root,
            inbox,
        }
    }

    /// Extension list for a pick, lowercase without dots.
    fn extensions(pick: &FilePick) -> Vec<String> {
        let mut out = Vec::new();
        for (_, exts) in &pick.filters {
            for e in exts {
                let e = e.trim_start_matches('.').to_lowercase();
                if !out.contains(&e) {
                    out.push(e);
                }
            }
        }
        out
    }

    /// MIME types for a SAF open, from the pick's extensions.
    fn mimes(extensions: &[String]) -> Vec<String> {
        let mut out: Vec<String> = extensions
            .iter()
            .filter_map(|e| saf::mime_for_extension(e))
            .map(|s| s.to_string())
            .collect();
        if out.is_empty() {
            out.push("*/*".to_string());
        }
        out
    }

    /// Launch the SAF open picker for `multiple`; the answer goes to `tx`.
    fn saf_open(&self, tx: Sender<Vec<String>>, multiple: bool, extensions: &[String]) {
        let Some(storage) = self.storage.clone() else {
            log::warn!("SAF open without shared storage; reporting cancelled");
            let _ = tx.send(Vec::new());
            return;
        };
        let reply: Box<dyn saf::DialogReply> = Box::new(ChannelReply { tx, inbox: self.inbox.clone() });
        saf::set_pending_open(reply, multiple, storage);
        if let Err(e) = saf::launch_open_document(multiple, &Self::mimes(extensions)) {
            log::error!("SAF open picker failed to launch: {e}");
            saf::cancel_pending();
        }
    }

    /// Launch the SAF create-document picker; the URI goes to `tx`.
    fn saf_create(&self, tx: Sender<Vec<String>>, name: &str, extensions: &[String]) {
        let Some(storage) = self.storage.clone() else {
            log::warn!("SAF create without shared storage; reporting cancelled");
            let _ = tx.send(Vec::new());
            return;
        };
        let mime = extensions
            .first()
            .and_then(|e| saf::mime_for_extension(e))
            .unwrap_or("application/octet-stream");
        let reply: Box<dyn saf::DialogReply> = Box::new(ChannelReply { tx, inbox: self.inbox.clone() });
        saf::set_pending_create(reply, name.to_string(), storage);
        if let Err(e) = saf::launch_create_document(name, mime) {
            log::error!("SAF create picker failed to launch: {e}");
            saf::cancel_pending();
        }
    }

    /// Build the [`Services`] for [`VectorcraftApp::new`].
    pub fn services(&self) -> Services {
        let browser = self.browser.clone();
        let self_for_saf = self.saf_helper();

        Services {
            // Async picks: the in-app media browser first, SAF as fallback.
            // `can()` also gates on the sync hooks below being `Some`.
            start_pick: Some(Box::new(move |request: PickRequest| {
                let (tx, rx) = channel::<Vec<String>>();
                let helper = self_for_saf.clone();
                match request {
                    PickRequest::Open(pick) => {
                        helper.open_browser(
                            &browser,
                            tx,
                            BrowserMode::Open,
                            &pick,
                            "Open",
                        );
                    }
                    PickRequest::OpenMany => {
                        helper.open_browser(
                            &browser,
                            tx,
                            BrowserMode::OpenMany,
                            &FilePick::default(),
                            "Place Files",
                        );
                    }
                    PickRequest::Save(pick) => {
                        helper.open_browser(
                            &browser,
                            tx,
                            BrowserMode::Save,
                            &pick,
                            "Save",
                        );
                    }
                    PickRequest::Folder => {
                        // Folders need real paths (SAF tree URIs are unusable
                        // as paths), so this stays in-app.
                        helper.open_browser(
                            &browser,
                            tx,
                            BrowserMode::Folder,
                            &FilePick::default(),
                            "Choose Folder",
                        );
                    }
                }
                Some(rx)
            })),

            // Sync hooks exist so `picks::can()` enables the dialogs; the
            // entry-based flows above do the real work. Outside an entry the
            // picks machinery warns and treats `None` as cancelled.
            pick_open: Some(Box::new(move |_pick: &FilePick| {
                log::warn!("pick_open outside a command entry; cancelled");
                None
            })),
            pick_open_multi: Some(Box::new(|| {
                log::warn!("pick_open_multi outside a command entry; cancelled");
                Vec::new()
            })),
            pick_save: Some(Box::new(move |_pick: &FilePick| {
                log::warn!("pick_save outside a command entry; cancelled");
                None
            })),
            pick_folder: Some(Box::new(|| {
                log::warn!("pick_folder outside a command entry; cancelled");
                None
            })),

            // Content-URI aware file I/O.
            read: Some(Box::new(move |p: &str| read_path(p))),
            write: Some(Box::new(move |p: &str, data: &[u8]| write_path(p, data))),
            exists: Some(Box::new(move |p: &str| {
                if p.starts_with("content://") {
                    // Can't stat a URI cheaply; the SAF layer staged what it
                    // handed us, so treat URIs as existing.
                    true
                } else {
                    Path::new(p).exists()
                }
            })),
            write_shared: Some(std::sync::Arc::new(move |p: &str, data: &[u8]| {
                write_path(p, data)
            })),
            stream_shared: Some(std::sync::Arc::new(move |p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
                let mut buf = Vec::new();
                fill(&mut buf).map_err(|e| e.to_string())?;
                write_path(p, &buf)
            })),

            open_url: Some(Box::new(|url: &str| {
                if let Err(e) = jni_bridge::open_url(url) {
                    log::warn!("open_url failed: {e}");
                }
            })),

            ..Default::default()
        }
    }

    fn saf_helper(&self) -> SafHelper {
        SafHelper {
            browser_root: self.browser_root.clone(),
            storage: self.storage.clone(),
            inbox: self.inbox.clone(),
        }
    }

    /// Called by the shell when the browser asks for the system picker.
    pub fn on_system_picker(&self, tx: Sender<Vec<String>>, request: &BrowserRequest) {
        match request.mode {
            BrowserMode::Save => {
                let name = if request.suggested_name.is_empty() {
                    "untitled".to_string()
                } else {
                    request.suggested_name.clone()
                };
                self.saf_create(tx, &name, &request.extensions);
            }
            BrowserMode::OpenMany => self.saf_open(tx, true, &request.extensions),
            _ => self.saf_open(tx, false, &request.extensions),
        }
    }
}

/// Small cloneable helper so the `start_pick` closure stays readable.
#[derive(Clone)]
struct SafHelper {
    browser_root: PathBuf,
    storage: Option<Arc<craftapps_shared::storage::StorageManager>>,
    inbox: PathBuf,
}

impl SafHelper {
    fn open_browser(
        &self,
        browser: &Arc<Mutex<FileBrowser>>,
        tx: Sender<Vec<String>>,
        mode: BrowserMode,
        pick: &FilePick,
        title: &str,
    ) {
        let extensions = AndroidServices::extensions(pick);
        let suggested = if matches!(mode, BrowserMode::Save) && !pick.name.is_empty() {
            pick.name.clone()
        } else {
            String::new()
        };
        if let Ok(mut b) = browser.lock() {
            // Fresh root each time (media dir may have changed).
            *b = FileBrowser::new(self.browser_root.clone());
            b.open(
                BrowserRequest {
                    mode,
                    extensions,
                    suggested_name: suggested,
                    title: title.to_string(),
                },
                tx,
            );
        } else {
            let _ = tx.send(Vec::new());
        }
    }
}

/// Read bytes from a filesystem path or a `content://` URI.
fn read_path(path: &str) -> Result<Vec<u8>, String> {
    if path.starts_with("content://") {
        saf::saf_read_uri(path).map(|(_, bytes)| bytes)
    } else {
        std::fs::read(path).map_err(|e| e.to_string())
    }
}

/// Write bytes to a filesystem path or a `content://` URI.
fn write_path(path: &str, data: &[u8]) -> Result<(), String> {
    if path.starts_with("content://") {
        return saf::saf_write_uri(path, data).map_err(|e| e.to_string());
    }
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    // Atomic-ish: write temp then rename, so a crash never leaves a half file.
    let tmp = p.with_extension("tmp-write");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}
