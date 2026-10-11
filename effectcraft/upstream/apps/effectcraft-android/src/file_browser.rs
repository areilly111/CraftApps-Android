//! In-app file browser answering the port's file-dialog hooks on Android.
//!
//! EffectCraft's [`Hooks`](effectcraft_ui_egui::Hooks) file pickers are
//! synchronous (`Fn` returning the picked paths), so there is no channel to
//! answer later like the desktop's off-thread rfd path: the hook opens this
//! browser, returns "cancelled" for this invocation, and the shell reruns the
//! matching engine command with the picked paths once the browser answers.
//! Opens (project / import) go through the browser; saves and folder picks
//! get immediate default paths from the hooks (see `hooks.rs` docs in
//! `lib.rs`), so every flow stays functional without a blocking dialog.
//!
//! The browser is rooted at the suite's shared media location
//! (`/sdcard/ArtCraft/Media`, or the chosen SD-card folder) through
//! [`StorageManager`]; entry ids are filesystem paths on tablet storage and
//! content-URI strings on the SD card (the same convention
//! [`StorageManager`] itself uses). When shared storage isn't granted, it
//! falls back to plain filesystem browsing of the app's files dir.
//!
//! "Browse other locations…" (Open) hands the pending reply to the Storage
//! Access Framework ([`craftapps_shared::saf`]): the browser closes without
//! answering and the SAF result handler answers through the shared answer
//! slot later. A picked file's bytes are staged into the cache so the answer
//! is always plain paths the engine can read.

use craftapps_shared::saf::{self, DialogAnswer, DialogReply};
use craftapps_shared::storage::{MediaEntry, StorageManager};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What the browser was opened for.
pub enum PickKind {
    /// File ▸ Open Project…
    OpenProject,
    /// File ▸ Import… (media extensions).
    ImportFiles { exts: Vec<String> },
    /// File ▸ Save / Save As… (`.ecproj`).
    SaveProject { default: String },
    /// Any other save (frame, preview, Lottie, preset, template, …).
    SaveFile { default: String },
}

/// The browser's answer: picked paths, always plain filesystem paths (SAF
/// picks are staged into the cache first).
pub type PickAnswer = Vec<String>;

/// Adapts the shared SAF [`DialogReply`] to the browser's answer slot. A
/// picked file's bytes are staged into the cache so the answer is always
/// plain paths.
struct SlotReply {
    slot: Arc<Mutex<Option<PickAnswer>>>,
    cache_dir: PathBuf,
}

impl DialogReply for SlotReply {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>) {
        let answer: PickAnswer = match answer {
            Some(DialogAnswer::Paths(paths)) => paths,
            Some(DialogAnswer::SaveTo(_)) => vec![],
            Some(DialogAnswer::Contents(name, bytes)) => stage_bytes(&self.cache_dir, &name, &bytes)
                .map(|path| vec![path])
                .unwrap_or_else(|e| {
                    log::error!("couldn't stage the picked file: {e}");
                    vec![]
                }),
            None => vec![],
        };
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some(answer);
        }
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
    cache_dir: PathBuf,
    pending: Option<PickKind>,
    /// The answer, once the dialog (or SAF) has one; the shell drains it.
    answer: Arc<Mutex<Option<PickAnswer>>>,
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
    pub fn new(
        storage: Arc<StorageManager>,
        fallback_dir: Option<PathBuf>,
        cache_dir: PathBuf,
    ) -> Result<Self, String> {
        Ok(FileBrowser {
            storage,
            fallback_dir,
            cache_dir,
            pending: None,
            answer: Arc::new(Mutex::new(None)),
            trail: Vec::new(),
            selected: Vec::new(),
            highlighted: None,
            save_name: String::new(),
            error: None,
        })
    }

    /// Open the browser for `kind`, called from a hook on the UI thread.
    pub fn push(&mut self, kind: PickKind) {
        if let PickKind::SaveProject { default } | PickKind::SaveFile { default } = &kind {
            self.save_name = Path::new(default)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| default.clone());
        }
        self.trail.clear();
        self.selected.clear();
        self.highlighted = None;
        self.error = None;
        if let Ok(mut slot) = self.answer.lock() {
            *slot = None;
        }
        self.pending = Some(kind);
    }

    /// Take the browser's answer, if it has one. The shell drains this every
    /// frame and reruns the matching engine command.
    pub fn take_answer(&self) -> Option<PickAnswer> {
        self.answer.lock().ok().and_then(|mut slot| slot.take())
    }

    fn answer(&mut self, answer: PickAnswer) {
        self.pending = None;
        if let Ok(mut slot) = self.answer.lock() {
            *slot = Some(answer);
        }
    }

    /// Id of the directory the browser is showing, if any (`None` = root).
    fn current_id(&self) -> Option<&str> {
        self.trail.last().map(|e| e.id.as_str())
    }

    fn root_label(&self) -> String {
        if self.fallback_dir.is_some() { "Internal files".to_string() } else { self.storage.location_label() }
    }

    fn is_save(&self) -> bool {
        matches!(self.pending, Some(PickKind::SaveProject { .. } | PickKind::SaveFile { .. }))
    }

    fn is_multi(&self) -> bool {
        matches!(self.pending, Some(PickKind::ImportFiles { .. }))
    }

    /// Extensions the open dialog accepts (`None` = every file).
    fn open_filter(&self) -> Option<Vec<String>> {
        match self.pending.as_ref()? {
            PickKind::OpenProject => Some(vec!["ecproj".to_string(), "ecprojx".to_string()]),
            PickKind::ImportFiles { exts } => Some(exts.clone()),
            _ => None,
        }
    }

    /// Entries of the current directory, directories first. `filter` (open dialogs)
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
        let is_save = self.is_save();
        let is_multi = self.is_multi();
        if self.pending.is_none() {
            return;
        }
        let filter = self.open_filter();
        let title = if is_save { "Save" } else { "Open" };
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
                    let mut here = self.root_label();
                    for e in &self.trail {
                        here.push('/');
                        here.push_str(&e.name);
                    }
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
                                } else if is_multi {
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
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            done = Some(vec![]);
                        }
                        let ids: Vec<String> = if is_multi {
                            self.selected.clone()
                        } else {
                            self.highlighted.clone().into_iter().collect()
                        };
                        if ui.add_enabled(!ids.is_empty(), egui::Button::new("Open")).clicked() {
                            match self.open_ids(&ids, is_multi) {
                                Ok(answer) => done = Some(answer),
                                Err(e) => self.error = Some(e),
                            }
                        }
                        if ui.button("Browse other locations…").clicked() {
                            self.browse_elsewhere(is_multi, filter.as_deref().unwrap_or(&[]));
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
        let clean = name.trim();
        if clean.is_empty() || clean.contains(['/', '\\']) {
            return Err("enter a file name without slashes".to_string());
        }
        if let Some(root) = &self.fallback_dir {
            let mut dir = root.clone();
            for e in &self.trail {
                dir = dir.join(&e.name);
            }
            return Ok(dir.join(clean).to_string_lossy().into_owned());
        }
        self.storage.reserve(self.current_id(), clean)
    }

    /// Answer an Open for the chosen ids.
    fn open_ids(&self, ids: &[String], multiple: bool) -> Result<PickAnswer, String> {
        if ids.iter().all(|id| !id.starts_with("content://")) {
            // Plain filesystem paths: the engine reads them itself.
            return Ok(ids.to_vec());
        }
        // SD-card content URIs: read the bytes now (single) or stage them
        // into the cache and answer paths (multi).
        let cache = self.cache_dir.clone();
        if !multiple {
            let id = ids.first().map(String::as_str).unwrap_or("");
            let entry = MediaEntry { name: String::new(), is_dir: false, id: id.to_string() };
            let (name, bytes) = self.storage.read(&entry)?;
            return Ok(vec![stage_bytes(&cache, &name, &bytes)?]);
        }
        let mut paths = Vec::with_capacity(ids.len());
        for id in ids {
            if id.starts_with("content://") {
                let entry = MediaEntry { name: String::new(), is_dir: false, id: id.clone() };
                let (name, bytes) = self.storage.read(&entry)?;
                paths.push(stage_bytes(&cache, &name, &bytes)?);
            } else {
                paths.push(id.clone());
            }
        }
        Ok(paths)
    }

    /// "Browse other locations…": hand the reply to the SAF picker and close
    /// the browser without answering; the SAF result answers the slot later.
    fn browse_elsewhere(&mut self, multiple: bool, extensions: &[String]) {
        let mimes: Vec<String> =
            extensions.iter().filter_map(|e| saf::mime_for_extension(e)).map(str::to_string).collect();
        match saf::launch_open_document(multiple, &mimes) {
            Ok(()) => {
                let reply = Box::new(SlotReply { slot: self.answer.clone(), cache_dir: self.cache_dir.clone() });
                saf::set_pending_open(reply, multiple, self.storage.clone());
                // The dialog closes; SAF answers the slot later.
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
/// dialogs) keeps only files whose extension is listed.
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
