//! In-app file browser backing `Services::start_pick` on Android.
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
//! pending reply to the Storage Access Framework ([`craftapps_shared::saf`]):
//! the browser closes without answering and the SAF result handler answers
//! the channel later. The app's `picks` machinery reruns whatever asked once
//! the answer arrives, exactly like the desktop's off-thread rfd path.

use craftapps_shared::saf::{self, DialogAnswer, DialogReply};
use craftapps_shared::storage::{MediaEntry, MediaLocation, StorageManager};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use vectorcraft_ui_egui::picks::{PickAnswer, PickRequest};

/// Adapts the shared SAF [`DialogReply`] to the channel the app's `picks`
/// machinery waits on. A picked file's bytes are staged into the cache so the
/// answer is always plain paths.
struct ChannelReply {
    tx: Sender<PickAnswer>,
    cache_dir: PathBuf,
}

impl DialogReply for ChannelReply {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>) {
        let answer: PickAnswer = match answer {
            Some(DialogAnswer::Paths(paths)) => paths,
            Some(DialogAnswer::SaveTo(uri)) => vec![uri],
            Some(DialogAnswer::Contents(name, bytes)) => stage_bytes(&self.cache_dir, &name, &bytes)
                .map(|path| vec![path])
                .unwrap_or_else(|e| {
                    log::error!("couldn't stage the picked file: {e}");
                    vec![]
                }),
            None => vec![],
        };
        let _ = self.tx.send(answer);
    }
}

/// Write `bytes` into `<cache_dir>/saf-inbox/` under a sanitised name → its path.
fn stage_bytes(cache_dir: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
    let dir = cache_dir.join("saf-inbox");
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let safe: String = name.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect();
    let safe = if safe.is_empty() { "file".to_string() } else { safe };
    let dest = dir.join(safe);
    std::fs::write(&dest, bytes).map_err(|e| format!("couldn't stage {}: {e}", dest.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

struct Pending {
    request: PickRequest,
    tx: Option<Sender<PickAnswer>>,
    cache_dir: PathBuf,
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
    /// Tablet media root (for the Folder dialog's "Use this folder" at root).
    media_root: PathBuf,
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
    pub fn new(storage: Arc<StorageManager>, fallback_dir: Option<PathBuf>) -> Result<Self, String> {
        // The media root: `<external>/ArtCraft/Media` (see `StorageManager::new`).
        let media_root = match &fallback_dir {
            Some(dir) => dir.clone(),
            None => {
                let root = craftapps_shared::jni_bridge::external_storage_root()?;
                PathBuf::from(root).join("ArtCraft/Media")
            }
        };
        Ok(FileBrowser {
            storage,
            fallback_dir,
            media_root,
            pending: None,
            trail: Vec::new(),
            selected: Vec::new(),
            highlighted: None,
            save_name: String::new(),
            error: None,
        })
    }

    /// Queue a dialog request from the `start_pick` service closure (the UI thread).
    pub fn push(&mut self, request: PickRequest, tx: Sender<PickAnswer>, cache_dir: PathBuf) {
        if let PickRequest::Save(pick) = &request {
            self.save_name = Path::new(&pick.name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| pick.name.clone());
        }
        self.trail.clear();
        self.selected.clear();
        self.highlighted = None;
        self.error = None;
        self.pending = Some(Pending { request, tx: Some(tx), cache_dir });
    }

    /// Take the pending reply channel without answering (handing it to SAF).
    fn take_tx(&mut self) -> Option<(Sender<PickAnswer>, PathBuf)> {
        self.pending.as_mut().and_then(|p| p.tx.take().map(|tx| (tx, p.cache_dir.clone())))
    }

    fn answer(&mut self, answer: PickAnswer) {
        if let Some(pending) = self.pending.take()
            && let Some(tx) = pending.tx
        {
            let _ = tx.send(answer);
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

    /// Extensions the open dialog accepts (`None` = every file).
    fn open_filter(&self) -> Option<Vec<String>> {
        let pending = self.pending.as_ref()?;
        match &pending.request {
            PickRequest::Open(pick) => {
                let exts: Vec<String> =
                    pick.filters.iter().flat_map(|(_, e)| e.iter()).map(|e| e.to_ascii_lowercase()).collect();
                Some(exts)
            }
            PickRequest::OpenMany => Some(
                vectorcraft_engine::cmd::fileio::place_filters()
                    .flat_map(|(_, e)| e.iter())
                    .map(|e| e.to_ascii_lowercase())
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Entries of the current directory, directories first. `filter` (open dialogs)
    /// keeps only files whose extension is listed; `dirs_only` (folder dialogs)
    /// skips files.
    fn entries(&self, filter: Option<&[String]>, dirs_only: bool) -> Result<Vec<Entry>, String> {
        if let Some(root) = &self.fallback_dir {
            // Plain filesystem fallback (shared storage not granted).
            let mut dir = root.clone();
            for e in &self.trail {
                dir = dir.join(&e.name);
            }
            return list_dir(&dir, filter, dirs_only).map(|v| {
                v.into_iter()
                    .map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.path.to_string_lossy().into_owned() })
                    .collect()
            });
        }
        let mut entries: Vec<Entry> = self
            .storage
            .list(self.current_id())
            .map(|v| v.into_iter().map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.id }).collect())?;
        if dirs_only {
            entries.retain(|e| e.is_dir);
        } else if let Some(want) = filter {
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
        let kind = match self.pending.as_ref() {
            Some(p) => match &p.request {
                PickRequest::Open(_) => 0,
                PickRequest::OpenMany => 1,
                PickRequest::Save(_) => 2,
                PickRequest::Folder => 3,
            },
            None => return,
        };
        // Folder dialogs need a filesystem path: on the SD card the entries are
        // document URIs the app can't use as folders, so offer tablet storage.
        if kind == 3 && self.fallback_dir.is_none() && self.storage.media_location() == MediaLocation::SdCard {
            self.show_folder_sd_card(ctx);
            return;
        }
        let filter = self.open_filter();
        let title = match kind {
            2 => "Save",
            3 => "Choose folder",
            _ => "Open",
        };
        let mut done: Option<PickAnswer> = None;

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
                // Folder dialogs list directories only.
                let dir_filter: Option<&[String]> = if kind == 3 { None } else { filter.as_deref() };
                match self.entries(dir_filter, kind == 3) {
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
                                } else if kind == 1 {
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
                match kind {
                    2 => {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut self.save_name);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(vec![]);
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
                                if name.is_empty() {
                                    self.error = Some("enter a file name first".to_string());
                                } else {
                                    self.save_elsewhere(&name);
                                }
                            }
                        });
                    }
                    3 => {
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(vec![]);
                            }
                            if ui.button("Use this folder").clicked() {
                                done = Some(vec![self.current_dir_id()]);
                            }
                        });
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                done = Some(vec![]);
                            }
                            let ids: Vec<String> = if kind == 1 {
                                self.selected.clone()
                            } else {
                                self.highlighted.clone().into_iter().collect()
                            };
                            if ui.add_enabled(!ids.is_empty(), egui::Button::new("Open")).clicked() {
                                match self.open_ids(&ids, kind == 1) {
                                    Ok(answer) => done = Some(answer),
                                    Err(e) => self.error = Some(e),
                                }
                            }
                            if ui.button("Browse other locations…").clicked() {
                                self.browse_elsewhere(kind == 1, filter.as_deref().unwrap_or(&[]));
                            }
                        });
                    }
                }
            });
        if let Some(answer) = done {
            self.answer(answer);
        }
    }

    /// Folder dialog while the media location is the SD card: its entries are
    /// document URIs, not filesystem folders, so the dialog asks for tablet
    /// storage instead of offering an unusable answer.
    fn show_folder_sd_card(&mut self, ctx: &egui::Context) {
        let mut done: Option<PickAnswer> = None;
        egui::Window::new("Choose folder")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Folder picking needs tablet storage: the SD-card location holds document URIs, not folders.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        done = Some(vec![]);
                    }
                    if ui.button("Use tablet storage").clicked() {
                        self.storage.use_tablet_storage();
                        // Redraw as the tablet browser next frame.
                    }
                });
            });
        if let Some(answer) = done {
            self.answer(answer);
        }
    }

    /// The id of the directory the browser is showing (the media root when at the top).
    fn current_dir_id(&self) -> String {
        self.current_id()
            .map(str::to_string)
            .unwrap_or_else(|| self.media_root.to_string_lossy().into_owned())
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
    fn open_ids(&self, ids: &[String], multiple: bool) -> Result<PickAnswer, String> {
        if ids.iter().all(|id| !id.starts_with("content://")) {
            // Plain filesystem paths: the app reads them itself.
            return Ok(ids.to_vec());
        }
        // SD-card content URIs: read the bytes now (single) or stage them
        // into the cache and answer paths (multi).
        if !multiple {
            let id = ids.first().map(String::as_str).unwrap_or("");
            let entry = MediaEntry { name: String::new(), is_dir: false, id: id.to_string() };
            let (name, bytes) = self.storage.read(&entry)?;
            let cache = self.pending.as_ref().map(|p| p.cache_dir.clone()).unwrap_or_else(|| {
                std::env::temp_dir()
            });
            return Ok(vec![stage_bytes(&cache, &name, &bytes)?]);
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
            let cache = self.pending.as_ref().map(|p| p.cache_dir.clone()).unwrap_or_else(std::env::temp_dir);
            for (name, bytes) in &files {
                paths.push(stage_bytes(&cache, name, bytes)?);
            }
        }
        Ok(paths)
    }

    /// "Browse other locations…": hand the reply to the SAF picker and close
    /// the browser without answering; the SAF result answers it later.
    fn browse_elsewhere(&mut self, multiple: bool, extensions: &[String]) {
        let mimes: Vec<String> =
            extensions.iter().filter_map(|e| saf::mime_for_extension(e)).map(str::to_string).collect();
        match saf::launch_open_document(multiple, &mimes) {
            Ok(()) => {
                if let Some((tx, cache_dir)) = self.take_tx() {
                    saf::set_pending_open(Box::new(ChannelReply { tx, cache_dir }), multiple, self.storage.clone());
                }
                // `pending` now has no channel: the window closes and SAF answers later.
                self.pending = None;
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
                if let Some((tx, cache_dir)) = self.take_tx() {
                    saf::set_pending_create(Box::new(ChannelReply { tx, cache_dir }), name.to_string(), self.storage.clone());
                }
                self.pending = None;
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
/// dialogs) keeps only files whose extension is listed; `dirs_only` (folder
/// dialogs) skips files.
fn list_dir(dir: &Path, filter: Option<&[String]>, dirs_only: bool) -> Result<Vec<FsEntry>, String> {    let read = std::fs::read_dir(dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|e| format!("couldn't read a directory entry: {e}"))?;
        let path = item.path();
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if dirs_only && !is_dir {
            continue;
        }
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
