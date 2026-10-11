//! In-app file browser backing `Services::open_async` on Android.
//!
//! The browser is rooted at the suite's shared media location
//! (`/sdcard/ArtCraft/Media`, or the chosen SD-card folder) through
//! [`StorageManager`]; entry ids are filesystem paths on tablet storage and
//! content-URI strings on the SD card (the same convention
//! [`StorageManager`] itself uses). When shared storage isn't granted, it
//! falls back to plain filesystem browsing of the app's files dir (the v1
//! behaviour).
//!
//! DesignCraft's `Services` has no async `file_dialog` service (that's a
//! PhotoCraft-ism); instead the port leaves `pick_open` unset and registers
//! `open_async`, so `pick_and_open` takes the web path: the chosen file
//! arrives through [`designcraft_ui_egui::Inbox`]. "Browse other locations…"
//! hands the pending reply to the Storage Access Framework
//! ([`craftapps_shared::saf`]): the browser closes without answering and the
//! SAF result handler answers later through the [`DialogReply`] adapter
//! below, which maps the answer onto the inbox the same way.

use craftapps_shared::saf::{self, DialogAnswer, DialogReply};
use craftapps_shared::storage::{MediaEntry, StorageManager};
use designcraft_ui_egui::{ImportRequest, ImportedFile, Inbox};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The [`DialogReply`] adapter: maps the shared SAF answers onto DesignCraft's
/// inbox (`ImportedFile`), i.e. this ui-egui's reply `send()`.
struct InboxReply {
    request: ImportRequest,
    inbox: Inbox,
    storage: Arc<StorageManager>,
}

impl DialogReply for InboxReply {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>) {
        let files: Vec<(String, Vec<u8>)> = match answer {
            Some(DialogAnswer::Contents(name, bytes)) => vec![(name, bytes)],
            Some(DialogAnswer::Paths(paths)) => paths
                .iter()
                .filter_map(|p| match read_id(&self.storage, p) {
                    Ok(file) => Some(file),
                    Err(e) => {
                        log::warn!("couldn't read {p}: {e}");
                        None
                    }
                })
                .collect(),
            // Cancelled, or a save answer (the Open browser never asks for one).
            _ => Vec::new(),
        };
        let Ok(mut inbox) = self.inbox.lock() else { return };
        for (name, bytes) in files {
            inbox.push(ImportedFile { request: self.request.clone(), name, bytes });
        }
    }
}

struct Pending {
    request: ImportRequest,
    inbox: Inbox,
}

struct Entry {
    name: String,
    is_dir: bool,
    /// Filesystem path (tablet / fallback) or content URI (SD card).
    id: String,
}

/// In-app Open/Place browser. Save flows can't use it: DesignCraft's
/// `pick_save` is synchronous, so saves go straight to the media folder (see
/// `services.rs`).
pub struct FileBrowser {
    storage: Arc<StorageManager>,
    /// Plain-fs fallback root when shared storage isn't granted.
    fallback_dir: Option<PathBuf>,
    pending: Option<Pending>,
    /// Directory stack; empty = browser root.
    trail: Vec<Entry>,
    /// Single-click selection for a single-open.
    highlighted: Option<String>,
    error: Option<String>,
}

impl FileBrowser {
    pub fn new(storage: Arc<StorageManager>, fallback_dir: Option<PathBuf>) -> Self {
        FileBrowser { storage, fallback_dir, pending: None, trail: Vec::new(), highlighted: None, error: None }
    }

    /// Queue an open dialog from the `open_async` service closure (the UI thread).
    pub fn open_dialog(&mut self, request: ImportRequest, inbox: Inbox) {
        self.trail.clear();
        self.highlighted = None;
        self.error = None;
        self.pending = Some(Pending { request, inbox });
    }

    /// Id of the directory the browser is showing, if any (`None` = root).
    fn current_id(&self) -> Option<&str> {
        self.trail.last().map(|e| e.id.as_str())
    }

    fn root_label(&self) -> String {
        if self.fallback_dir.is_some() {
            "Internal files".to_string()
        } else {
            self.storage.location_label()
        }
    }

    fn title(&self) -> &'static str {
        match self.pending.as_ref().map(|p| p.request.purpose.as_str()) {
            Some("place") => "Place",
            _ => "Open",
        }
    }

    /// Extensions the current dialog filters by, from the request's purpose.
    fn filter(&self) -> Option<&'static [&'static str]> {
        match self.pending.as_ref().map(|p| p.request.purpose.as_str()) {
            Some("place") => Some(&[
                "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "psd", "svg", "pdf", "txt", "docx", "rtf",
                "md", "xlsx",
            ]),
            // open: documents
            _ => Some(&["designcraft", "idml"]),
        }
    }

    /// Entries of the current directory, directories first. `filter` keeps
    /// only files whose extension is listed.
    fn entries(&self, filter: Option<&[&str]>) -> Result<Vec<Entry>, String> {
        if let Some(root) = &self.fallback_dir {
            // Plain filesystem fallback (shared storage not granted).
            let mut dir = root.clone();
            for e in &self.trail {
                dir = dir.join(&e.name);
            }
            return list_dir(&dir, filter);
        }
        let mut entries: Vec<Entry> = self
            .storage
            .list(self.current_id())
            .map(|v| v.into_iter().map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.id }).collect())?;
        if let Some(want) = filter {
            entries.retain(|e| {
                e.is_dir
                    || {
                        let ext = e.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                        want.iter().any(|w| *w == ext)
                    }
            });
        }
        Ok(entries)
    }

    /// Draw the pending dialog, if any. Called by the shell after the app frame.
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let title = self.title().to_string();
        let filter = self.filter();
        let mut done = false;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let at_root = self.trail.is_empty();
                    if ui.add_enabled(!at_root, egui::Button::new("⬆ Up")).clicked() {
                        self.trail.pop();
                        self.highlighted = None;
                    }
                    let here: String = if at_root {
                        self.root_label()
                    } else {
                        let mut s = self.root_label();
                        for e in &self.trail {
                            s.push('/');
                            s.push_str(&e.name);
                        }
                        s
                    };
                    ui.label(here);
                });
                ui.separator();
                match self.entries(filter) {
                    Ok(entries) => {
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for entry in entries {
                                if entry.is_dir {
                                    if ui.button(format!("📁 {}", entry.name)).clicked() {
                                        self.trail.push(entry);
                                        self.highlighted = None;
                                    }
                                } else {
                                    let hl = self.highlighted.as_ref() == Some(&entry.id);
                                    if ui.selectable_label(hl, &entry.name).clicked() {
                                        self.highlighted = Some(entry.id.clone());
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
                if let Some(e) = self.error.as_ref() {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                    let openable = self.highlighted.is_some();
                    if ui.add_enabled(openable, egui::Button::new("Open")).clicked()
                        && let Some(id) = self.highlighted.clone()
                    {
                        match self.open_id(&id) {
                            Ok(()) => done = true,
                            Err(e) => self.error = Some(e),
                        }
                    }
                    if ui.button("Browse other locations…").clicked() {
                        self.browse_elsewhere(filter.unwrap_or(&[]));
                        done = true;
                    }
                });
            });
        if done {
            // Answered (or cancelled / handed to SAF): the window closes.
            self.pending = None;
            self.trail.clear();
            self.highlighted = None;
        }
    }

    /// Read the chosen id and push it into the inbox with the request's context.
    fn open_id(&mut self, id: &str) -> Result<(), String> {
        let pending = self.pending.as_ref().ok_or("no dialog is open")?;
        let (name, bytes) = read_id(&self.storage, id)?;
        match pending.inbox.lock() {
            Ok(mut inbox) => {
                inbox.push(ImportedFile { request: pending.request.clone(), name, bytes });
                Ok(())
            }
            Err(_) => Err("the import queue is unavailable".to_string()),
        }
    }

    /// "Browse other locations…": hand a [`DialogReply`] adapter to the SAF
    /// picker and close the browser without answering; the SAF result answers
    /// it later, delivering through the inbox.
    fn browse_elsewhere(&mut self, extensions: &[&str]) {
        let Some(pending) = self.pending.take() else { return };
        let mimes: Vec<String> = extensions.iter().filter_map(|e| saf::mime_for_extension(e)).map(str::to_string).collect();
        let reply: Box<dyn DialogReply> =
            Box::new(InboxReply { request: pending.request, inbox: pending.inbox, storage: self.storage.clone() });
        match saf::launch_open_document(false, &mimes) {
            Ok(()) => saf::set_pending_open(reply, false, self.storage.clone()),
            Err(e) => {
                log::error!("couldn't open the system file picker: {e}");
                saf::cancel_pending();
                reply.send(None);
            }
        }
        self.trail.clear();
        self.highlighted = None;
    }
}

/// Read an entry id: content URIs through SAF, plain paths from the filesystem.
fn read_id(storage: &StorageManager, id: &str) -> Result<(String, Vec<u8>), String> {
    let entry = MediaEntry { name: String::new(), is_dir: false, id: id.to_string() };
    let (mut name, bytes) = storage.read(&entry)?;
    if name.is_empty() {
        name = storage.name_for_id(id);
    }
    Ok((name, bytes))
}

struct FsEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Directories first, then files, case-insensitive by name. `filter` (open
/// dialog) keeps only files whose extension is listed.
fn list_dir(dir: &Path, filter: Option<&[&str]>) -> Result<Vec<Entry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
    let mut entries: Vec<FsEntry> = Vec::new();
    for item in read {
        let item = item.map_err(|e| format!("couldn't read a directory entry: {e}"))?;
        let path = item.path();
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir
            && let Some(want) = filter
        {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !want.iter().any(|w| *w == ext) {
                continue;
            }
        }
        let name = item.file_name().to_string_lossy().into_owned();
        entries.push(FsEntry { name, path, is_dir });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });
    Ok(entries
        .into_iter()
        .map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.path.to_string_lossy().into_owned() })
        .collect())
}
