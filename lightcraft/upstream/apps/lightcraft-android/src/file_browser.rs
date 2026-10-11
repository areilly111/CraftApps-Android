//! In-app file browser backing `Services::picker` on Android.
//!
//! The browser is rooted at the suite's shared media location
//! (`/sdcard/ArtCraft/Media`, or the chosen SD-card folder) through
//! [`StorageManager`]; entry ids are filesystem paths on tablet storage and
//! content-URI strings on the SD card (the same convention
//! [`StorageManager`] itself uses). When shared storage isn't granted, it
//! falls back to plain filesystem browsing of the app's files dir.
//!
//! "Browse other locations…" (Open) and "Save elsewhere…" (Save) hand the
//! pending reply to the Storage Access Framework
//! ([`craftapps_shared::saf`]): the browser closes without answering and the
//! SAF result handler answers later. The answer goes back through the
//! picker's channel as paths, exactly like the desktop's rfd path, so the app
//! logic is unchanged.

use craftapps_shared::saf::{self, DialogAnswer, DialogReply};
use craftapps_shared::storage::{MediaEntry, StorageManager};
use eframe::egui;
use lightcraft_ui_egui::pick::{PickKind, PickRequest};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Adapts the shared SAF [`DialogReply`] to the picker's `Sender<Vec<String>>`.
struct PickReply {
    tx: Sender<Vec<String>>,
    storage: Arc<StorageManager>,
}

impl DialogReply for PickReply {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>) {
        let paths = match answer {
            // Cancelled: `pick::poll` drops empty answers.
            None => Vec::new(),
            Some(DialogAnswer::Paths(paths)) => paths,
            Some(DialogAnswer::SaveTo(uri)) => vec![uri],
            // A single picked file's bytes: stage them and answer the path.
            Some(DialogAnswer::Contents(name, bytes)) => stage(&self.storage, &name, &bytes).into_iter().collect(),
        };
        let _ = self.tx.send(paths);
    }
}

/// Stage picked bytes into the SAF inbox and answer its path.
fn stage(storage: &StorageManager, name: &str, bytes: &[u8]) -> Option<String> {
    let dir = storage.saf_inbox_dir();
    if std::fs::create_dir_all(dir).is_err() {
        return None;
    }
    let safe: String =
        name.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect();
    let safe = if safe.is_empty() { "file".to_string() } else { safe };
    let dest = dir.join(safe);
    std::fs::write(&dest, bytes).ok()?;
    Some(dest.to_string_lossy().into_owned())
}

struct Pending {
    request: PickRequest,
    tx: Option<Sender<Vec<String>>>,
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
    error: Option<String>,
}

impl FileBrowser {
    pub fn new(storage: Arc<StorageManager>, fallback_dir: Option<PathBuf>) -> Self {
        FileBrowser {
            storage,
            fallback_dir,
            pending: None,
            trail: Vec::new(),
            selected: Vec::new(),
            highlighted: None,
            save_name: String::new(),
            error: None,
        }
    }

    /// Queue a dialog request from the `picker` service closure (UI thread).
    pub fn push(&mut self, request: PickRequest, tx: Sender<Vec<String>>) {
        if let Some(name) = &request.file_name {
            self.save_name = Path::new(name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
        }
        self.trail.clear();
        self.selected.clear();
        self.highlighted = None;
        self.error = None;
        self.pending = Some(Pending { request, tx: Some(tx) });
    }

    /// Answer the pending request (empty = cancelled) and close the dialog.
    fn answer(&mut self, paths: Vec<String>) {
        if let Some(mut pending) = self.pending.take()
            && let Some(tx) = pending.tx.take()
        {
            let _ = tx.send(paths);
        }
    }

    /// Take the pending reply without answering (handing it to SAF).
    fn take_reply(&mut self) -> Option<Box<PickReply>> {
        let pending = self.pending.take()?;
        let tx = pending.tx?;
        Some(Box::new(PickReply { tx, storage: self.storage.clone() }))
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

    /// Entries of the current directory, directories first. `filter` (open
    /// dialog) keeps only files whose extension is listed.
    fn entries(&self, filter: Option<&[&str]>) -> Result<Vec<Entry>, String> {
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
        let (kind, title, filter, file_name) = match self.pending.as_ref() {
            Some(p) => (
                p.request.kind,
                p.request.title.clone().unwrap_or_else(|| match p.request.kind {
                    PickKind::Save => "Save".to_string(),
                    _ => "Open".to_string(),
                }),
                p.request.filter.clone().map(|(_, exts)| exts),
                p.request.file_name.clone(),
            ),
            None => return,
        };
        let _ = file_name;
        let mut done: Option<Vec<String>> = None;

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
                match self.entries(filter) {
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
                                } else if kind == PickKind::Files {
                                    let mut on = self.selected.contains(&entry.id);
                                    if ui.checkbox(&mut on, &entry.name).changed() {
                                        if on {
                                            self.selected.push(entry.id.clone());
                                        } else {
                                            self.selected.retain(|id| *id != entry.id);
                                        }
                                    }
                                } else if kind == PickKind::File {
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
                match kind {
                    PickKind::Save => {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut self.save_name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(Vec::new());
                            }
                            let name = self.save_name.trim().to_string();
                            let ok = !name.is_empty();
                            if ui.add_enabled(ok, egui::Button::new("Save")).clicked() {
                                match self.reserve(&name) {
                                    Ok(id) => done = Some(vec![id]),
                                    Err(e) => self.error = Some(e),
                                }
                            }
                            if ui.button("Save elsewhere…").clicked() {
                                self.save_elsewhere(&name);
                            }
                        });
                    }
                    PickKind::Folder => {
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(Vec::new());
                            }
                            if ui.button("Choose this folder").clicked() {
                                let id = self
                                    .current_id()
                                    .map(str::to_string)
                                    .or_else(|| self.fallback_dir.as_ref().map(|d| d.to_string_lossy().into_owned()));
                                match id {
                                    Some(id) => done = Some(vec![id]),
                                    None => self.error = Some("couldn't use this folder".to_string()),
                                }
                            }
                        });
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(Vec::new());
                            }
                            let ids: Vec<String> = if kind == PickKind::Files {
                                self.selected.clone()
                            } else {
                                self.highlighted.clone().into_iter().collect()
                            };
                            if ui.add_enabled(!ids.is_empty(), egui::Button::new("Open")).clicked() {
                                match self.open_ids(&ids) {
                                    Ok(paths) => done = Some(paths),
                                    Err(e) => self.error = Some(e),
                                }
                            }
                            if ui.button("Browse other locations…").clicked() {
                                self.browse_elsewhere(kind == PickKind::Files, filter.unwrap_or(&[]));
                            }
                        });
                    }
                }
            });
        if let Some(paths) = done {
            self.answer(paths);
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

    /// Answer an Open for the chosen ids: plain paths go back as-is;
    /// content-URI ids (SD card) are read and staged into the cache, and the
    /// staged paths go back (the app reads files by path).
    fn open_ids(&self, ids: &[String]) -> Result<Vec<String>, String> {
        let mut paths = Vec::with_capacity(ids.len());
        for id in ids {
            if id.starts_with("content://") {
                let entry = MediaEntry { name: String::new(), is_dir: false, id: id.clone() };
                let (name, bytes) = self.storage.read(&entry)?;
                match stage(&self.storage, &name, &bytes) {
                    Some(p) => paths.push(p),
                    None => return Err(format!("couldn't stage {name}")),
                }
            } else {
                paths.push(id.clone());
            }
        }
        Ok(paths)
    }

    /// "Browse other locations…": hand the reply to the SAF picker and close
    /// the browser without answering; the SAF result answers it later.
    fn browse_elsewhere(&mut self, multiple: bool, extensions: &[&str]) {
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
fn list_dir(dir: &Path, filter: Option<&[&str]>) -> Result<Vec<FsEntry>, String> {
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
