//! In-app file browser backing `Services::file_dialog` on Android.
//!
//! The browser is rooted at the suite's shared media location
//! (`/sdcard/ArtCraft/Media`, or the chosen SD-card folder) through
//! [`StorageManager`]; entry ids are filesystem paths on tablet storage and
//! content-URI strings on the SD card (the same convention
//! [`StorageManager`] itself uses). When shared storage isn't granted, it
//! falls back to plain filesystem browsing of the app's files dir (the v1
//! behaviour).
//!
//! "Browse other locations…" (Open) and "Save elsewhere…" (Save) hand the
//! pending reply to the Storage Access Framework ([`crate::saf`]): the browser
//! closes without answering and the SAF result handler answers later. The
//! answer goes back through [`FileDialogReply`], exactly like the desktop's
//! rfd path, so the app logic is unchanged.

use crate::saf;
use crate::storage::{MediaEntry, StorageManager};
use eframe::egui;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Set by the Save dialog when "Add a copy to the Gallery" is ticked; the
/// `write` service consumes it after a successful write (see `services.rs`).
pub type GalleryFlag = Arc<Mutex<Option<PathBuf>>>;

struct Pending {
    request: FileDialogRequest,
    reply: Option<FileDialogReply>,
}

struct Entry {
    name: String,
    is_dir: bool,
    /// Filesystem path (tablet / fallback) or content URI (SD card).
    id: String,
}

pub struct FileBrowser {
    storage: Arc<StorageManager>,
    /// Plain-fs fallback root when shared storage isn't granted.
    fallback_dir: Option<PathBuf>,
    pending: Option<Pending>,
    /// Directory stack; empty = browser root.
    trail: Vec<Entry>,
    /// Selected ids for a multi-open.
    selected: Vec<String>,
    /// Single-click selection for a single-open.
    highlighted: Option<String>,
    save_name: String,
    gallery_copy: bool,
    gallery: GalleryFlag,
    error: Option<String>,
}

impl FileBrowser {
    pub fn new(storage: Arc<StorageManager>, fallback_dir: Option<PathBuf>, gallery: GalleryFlag) -> Self {
        FileBrowser {
            storage,
            fallback_dir,
            pending: None,
            trail: Vec::new(),
            selected: Vec::new(),
            highlighted: None,
            save_name: String::new(),
            gallery_copy: false,
            gallery,
            error: None,
        }
    }

    /// Queue a dialog request from the `file_dialog` service closure (any thread).
    pub fn push(&mut self, request: FileDialogRequest, reply: FileDialogReply) {
        if let FileDialogRequest::Save { suggested } = &request {
            self.save_name = Path::new(suggested)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| suggested.clone());
            self.gallery_copy = false;
        }
        self.trail.clear();
        self.selected.clear();
        self.highlighted = None;
        self.error = None;
        self.pending = Some(Pending { request, reply: Some(reply) });
    }

    /// Take the pending reply without answering (handing it to SAF).
    fn take_reply(&mut self) -> Option<FileDialogReply> {
        self.pending.take().and_then(|mut p| p.reply.take())
    }

    fn answer(&mut self, answer: Option<FileDialogAnswer>) {
        if let Some(mut pending) = self.pending.take()
            && let Some(reply) = pending.reply.take()
        {
            reply.send(answer);
        }
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

    /// Entries of the current directory, directories first. `filter` (open dialog)
    /// keeps only files whose extension is listed.
    fn entries(&self, filter: Option<&[String]>) -> Result<Vec<Entry>, String> {
        if let Some(root) = &self.fallback_dir {
            // Plain filesystem fallback (shared storage not granted).
            let mut dir = root.clone();
            for e in &self.trail {
                dir = dir.join(&e.name);
            }
            return list_dir(&dir, filter).map(|v| {
                v.into_iter()
                    .map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.path.to_string_lossy().into_owned() })
                    .collect()
            });
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
                        want.iter().any(|w| w.to_ascii_lowercase() == ext)
                    }
            });
        }
        Ok(entries)
    }

    /// Draw the pending dialog, if any. Called by the shell after the app frame.
    pub fn show(&mut self, ctx: &egui::Context) {
        // Pull what the dialog needs out of the pending request first, so the
        // draw closure below only borrows `self` mutably.
        let (is_save, filter, multiple) = match self.pending.as_ref() {
            Some(p) => match &p.request {
                FileDialogRequest::Open { multiple, extensions, .. } => (false, extensions.clone(), *multiple),
                FileDialogRequest::Save { .. } => (true, None, false),
                FileDialogRequest::Folder { .. } => (false, None, false),
            },
            None => return,
        };
        let title = if is_save { "Save" } else { "Open" };
        let mut done: Option<Option<FileDialogAnswer>> = None;

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
                        self.selected.clear();
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
                match self.entries(filter.as_deref()) {
                    Ok(entries) => {
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for entry in entries {
                                if entry.is_dir {
                                    ui.horizontal(|ui| {
                                        if ui.button(format!("📁 {}", entry.name)).clicked() {
                                            self.trail.push(entry);
                                            self.selected.clear();
                                            self.highlighted = None;
                                        }
                                    });
                                } else if multiple {
                                    let mut on = self.selected.contains(&entry.id);
                                    if ui.checkbox(&mut on, &entry.name).changed() {
                                        if on {
                                            self.selected.push(entry.id.clone());
                                        } else {
                                            self.selected.retain(|id| *id != entry.id);
                                        }
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
                if is_save {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.save_name);
                    });
                    if gallery_offered(&self.save_name) {
                        ui.checkbox(&mut self.gallery_copy, "Add a copy to the Gallery");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            done = Some(None);
                        }
                        let name = self.save_name.trim().to_string();
                        let ok = !name.is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                            match self.reserve(&name) {
                                Ok(id) => {
                                    if self.gallery_copy
                                        && let Ok(mut g) = self.gallery.lock()
                                    {
                                        *g = Some(PathBuf::from(&id));
                                    }
                                    done = Some(Some(FileDialogAnswer::SaveTo(id)));
                                }
                                Err(e) => self.error = Some(e),
                            }
                        }
                        if ui.button("Save elsewhere…").clicked() {
                            self.save_elsewhere(&name);
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            done = Some(None);
                        }
                        let ids: Vec<String> = if multiple { self.selected.clone() } else { self.highlighted.clone().into_iter().collect() };
                        if ui.add_enabled(!ids.is_empty(), egui::Button::new("Open")).clicked() {
                            match self.open_ids(&ids, multiple) {
                                Ok(answer) => done = Some(Some(answer)),
                                Err(e) => self.error = Some(e),
                            }
                        }
                        if ui.button("Browse other locations…").clicked() {
                            self.browse_elsewhere(multiple, filter.as_deref().unwrap_or(&[]));
                        }
                    });
                }
            });
        if let Some(answer) = done {
            self.answer(answer);
        }
    }

    /// Reserve a save target for `name` in the current directory.
    fn reserve(&self, name: &str) -> Result<String, String> {
        if let Some(root) = &self.fallback_dir {
            let mut dir = root.clone();
            for e in &self.trail {
                dir = dir.join(&e.name);
            }
            let clean = name.trim();
            if clean.is_empty() || clean.contains(['/', '\\']) {
                return Err("enter a file name without slashes".to_string());
            }
            return Ok(dir.join(clean).to_string_lossy().into_owned());
        }
        self.storage.reserve(self.current_id(), name)
    }

    /// Answer an Open for the chosen ids.
    fn open_ids(&self, ids: &[String], multiple: bool) -> Result<FileDialogAnswer, String> {
        if ids.iter().all(|id| !id.starts_with("content://")) {
            // Plain filesystem paths: the app reads them itself.
            return Ok(FileDialogAnswer::Paths(ids.to_vec()));
        }
        // SD-card content URIs: read the bytes now (single) or stage them
        // into the cache and answer paths (multi).
        if !multiple {
            let id = ids.first().map(String::as_str).unwrap_or("");
            let entry = MediaEntry { name: String::new(), is_dir: false, id: id.to_string() };
            let (name, bytes) = self.storage.read(&entry)?;
            return Ok(FileDialogAnswer::Contents(name, bytes));
        }
        let mut files = Vec::with_capacity(ids.len());
        let mut paths = Vec::with_capacity(ids.len());
        for id in ids {
            if id.starts_with("content://") {
                let entry = MediaEntry { name: String::new(), is_dir: false, id: id.clone() };
                files.push(self.storage.read(&entry)?);
            } else {
                paths.push(id.clone());
            }
        }
        if !files.is_empty() {
            paths.extend(saf::stage_files(&self.storage, &files)?);
        }
        Ok(FileDialogAnswer::Paths(paths))
    }

    /// "Browse other locations…": hand the reply to the SAF picker and close
    /// the browser without answering; the SAF result answers it later.
    fn browse_elsewhere(&mut self, multiple: bool, extensions: &[String]) {
        let mimes: Vec<String> =
            extensions.iter().filter_map(|e| saf::mime_for_extension(e)).map(str::to_string).collect();
        match saf::launch_open_document(multiple, &mimes) {
            Ok(()) => {
                if let Some(reply) = self.take_reply() {
                    saf::set_pending_open(reply, multiple, self.storage.clone());
                }
                // `pending` is now empty: the window closes and SAF answers later.
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// "Save elsewhere…": same handoff for `ACTION_CREATE_DOCUMENT`.
    fn save_elsewhere(&mut self, name: &str) {
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let mime = saf::mime_for_extension(&ext).unwrap_or("application/octet-stream");
        match saf::launch_create_document(name, mime) {
            Ok(()) => {
                if let Some(reply) = self.take_reply() {
                    saf::set_pending_create(reply, name.to_string(), self.storage.clone());
                }
            }
            Err(e) => self.error = Some(e),
        }
    }
}

struct FsEntry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Directories first, then files, case-insensitive by name. `filter` (open
/// dialog) keeps only files whose extension is listed.
fn list_dir(dir: &Path, filter: Option<&[String]>) -> Result<Vec<FsEntry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|e| format!("couldn't read a directory entry: {e}"))?;
        let path = item.path();
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir
            && let Some(want) = filter
        {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !want.iter().any(|w| w.to_ascii_lowercase() == ext) {
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
    Ok(entries)
}

/// Extensions the Gallery copy checkbox is offered for.
fn gallery_offered(save_name: &str) -> bool {
    let ext = Path::new(save_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp")
}

pub fn gallery_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
