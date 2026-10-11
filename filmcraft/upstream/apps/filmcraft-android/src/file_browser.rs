//! In-app file browser backing the Android pick hooks on FilmCraft.
//!
//! FilmCraft's [`filmcraft_ui_egui::HostHooks`] pickers are synchronous, so the
//! port follows the web app's pattern (`apps/filmcraft-web/src/import.rs`):
//! a hook opens the browser and returns "no answer"; when the user confirms,
//! the shell runs the engine command directly (`file.import`, `file.open`,
//! the relink hint's command) with the picked paths. Nothing blocks the UI
//! thread.
//!
//! The browser is rooted at the suite's shared media location
//! (`/sdcard/ArtCraft/Media`, or the chosen SD-card folder) through
//! [`StorageManager`]; entry ids are filesystem paths on tablet storage and
//! content-URI strings on the SD card (the same convention
//! [`StorageManager`] itself uses). When shared storage isn't granted, it
//! falls back to plain filesystem browsing of the app's files dir.
//!
//! "Browse other locations…" hands the pending reply to the Storage Access
//! Framework ([`craftapps_shared::saf`]): the browser closes without answering
//! and the SAF result handler answers later. SD-card content URIs are staged
//! into the cache so the answer is always plain paths the engine can read.

use craftapps_shared::saf::{self, DialogAnswer, DialogReply};
use craftapps_shared::storage::{MediaEntry, StorageManager};
use eframe::egui;
use filmcraft_ui_egui::RelinkHint;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the browser should ask for.
pub enum PickKind {
    /// Import media: multi-select, extension-filtered.
    Import { extensions: Vec<String> },
    /// Open a `.fcproj` project: single-select.
    OpenProject,
    /// Pick one file, then run the relink hint's command on it
    /// (Link Media ▸ Locate…, Attach Proxies, Reconnect Full Resolution).
    Relink { extensions: Vec<String>, hint: RelinkHint },
}

/// A resolved pick, handed back to the shell to run the engine command.
pub enum ResolvedPick {
    Import(Vec<String>),
    OpenProject(String),
    Relink { path: String, hint: RelinkHint },
}

/// Adapts the shared SAF [`DialogReply`] to the browser's resolved slot. Picked
/// files are staged into the cache so the answer is always plain paths; a
/// relink pick resolves straight to its hint command.
struct SafReply {
    cache_dir: PathBuf,
    storage: Arc<StorageManager>,
    /// Where the resolved pick goes.
    slot: Arc<std::sync::Mutex<Option<ResolvedPick>>>,
    kind: SafKind,
    /// Set for relink picks: the hint's command runs on the staged path.
    relink_hint: Option<RelinkHint>,
}

#[derive(Clone, Copy)]
enum SafKind {
    Import,
    OpenProject,
    Relink,
}

impl SafReply {
    fn resolve(&self, ids: Vec<String>) -> Option<ResolvedPick> {
        let paths = stage_ids(&ids, &self.cache_dir, &self.storage);
        match self.kind {
            SafKind::Import => {
                if paths.is_empty() {
                    None
                } else {
                    Some(ResolvedPick::Import(paths))
                }
            }
            SafKind::OpenProject => paths.into_iter().next().map(ResolvedPick::OpenProject),
            SafKind::Relink => {
                let path = paths.into_iter().next()?;
                let hint = self.relink_hint.clone()?;
                Some(ResolvedPick::Relink { path, hint })
            }
        }
    }
}

impl DialogReply for SafReply {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>) {
        let ids: Vec<String> = match answer {
            Some(DialogAnswer::Paths(paths)) => paths,
            Some(DialogAnswer::SaveTo(uri)) => vec![uri],
            Some(DialogAnswer::Contents(name, bytes)) => {
                stage_bytes(&self.cache_dir, &name, &bytes).map(|p| vec![p]).unwrap_or_else(|e| {
                    log::error!("couldn't stage the picked file: {e}");
                    vec![]
                })
            }
            None => vec![],
        };
        let resolved = self.resolve(ids);
        if let Ok(mut slot) = self.slot.lock() {
            *slot = resolved;
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

/// Stage picked ids into plain paths: SD-card content URIs are read through the
/// storage manager into the cache; filesystem paths pass through unchanged.
fn stage_ids(ids: &[String], cache_dir: &Path, storage: &StorageManager) -> Vec<String> {
    let mut paths: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        if id.starts_with("content://") {
            let entry = MediaEntry { name: String::new(), is_dir: false, id: id.clone() };
            match storage.read(&entry) {
                Ok((name, bytes)) => match stage_bytes(cache_dir, &name, &bytes) {
                    Ok(p) => paths.push(p),
                    Err(e) => log::error!("couldn't stage the picked file: {e}"),
                },
                Err(e) => log::error!("couldn't read the picked file: {e}"),
            }
        } else {
            paths.push(id.clone());
        }
    }
    paths
}

struct Pending {
    kind: PickKind,
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
    pending: Option<Pending>,
    /// Resolved picks waiting for the shell to run their engine command.
    resolved: Arc<std::sync::Mutex<Option<ResolvedPick>>>,
    /// Directory stack; empty = browser root.
    trail: Vec<Entry>,
    /// Selected ids for a multi-open.
    selected: Vec<String>,
    /// Single-click selection for a single-open.
    highlighted: Option<String>,
    error: Option<String>,
}

impl FileBrowser {
    pub fn new(storage: Arc<StorageManager>, fallback_dir: Option<PathBuf>) -> Result<Self, String> {
        Ok(FileBrowser {
            storage,
            fallback_dir,
            pending: None,
            resolved: Arc::new(std::sync::Mutex::new(None)),
            trail: Vec::new(),
            selected: Vec::new(),
            highlighted: None,
            error: None,
        })
    }

    /// Queue a dialog request from a hook closure (the UI thread).
    pub fn push(&mut self, kind: PickKind, cache_dir: PathBuf) {
        self.trail.clear();
        self.selected.clear();
        self.highlighted = None;
        self.error = None;
        self.pending = Some(Pending { kind, cache_dir });
    }

    /// Take a resolved pick for the shell to act on, if any.
    pub fn take_resolved(&self) -> Option<ResolvedPick> {
        self.resolved.lock().ok().and_then(|mut slot| slot.take())
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

    /// Extensions the open dialog accepts.
    fn open_filter(&self) -> Vec<String> {
        let pending = match self.pending.as_ref() {
            Some(p) => p,
            None => return Vec::new(),
        };
        match &pending.kind {
            PickKind::Import { extensions } | PickKind::Relink { extensions, .. } => {
                extensions.iter().map(|e| e.to_ascii_lowercase()).collect()
            }
            PickKind::OpenProject => vec!["fcproj".to_string()],
        }
    }

    fn is_multi(&self) -> bool {
        matches!(self.pending.as_ref().map(|p| &p.kind), Some(PickKind::Import { .. }))
    }

    /// Entries of the current directory, directories first, extension-filtered for opens.
    fn entries(&self, filter: &[String]) -> Result<Vec<Entry>, String> {
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
        let entries: Vec<Entry> = self
            .storage
            .list(self.current_id())
            .map(|v| v.into_iter().map(|e| Entry { name: e.name, is_dir: e.is_dir, id: e.id }).collect())?;
        Ok(entries
            .into_iter()
            .filter(|e| {
                e.is_dir || {
                    let ext = e.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                    filter.contains(&ext)
                }
            })
            .collect())
    }

    /// Answer the pending pick with these ids (staging content URIs first).
    fn answer_ids(&mut self, ids: &[String]) {
        let pending = match self.pending.take() {
            Some(p) => p,
            None => return,
        };
        let paths = stage_ids(ids, &pending.cache_dir, &self.storage);
        let resolved = match &pending.kind {
            PickKind::Import { .. } => {
                if paths.is_empty() {
                    None
                } else {
                    Some(ResolvedPick::Import(paths))
                }
            }
            PickKind::OpenProject => paths.into_iter().next().map(ResolvedPick::OpenProject),
            // Relink resolves straight to its hint command.
            PickKind::Relink { hint, .. } => {
                paths.into_iter().next().map(|path| ResolvedPick::Relink { path, hint: hint.clone() })
            }
        };
        if let Ok(mut slot) = self.resolved.lock() {
            *slot = resolved;
        }
    }

    /// Draw the pending dialog, if any. Called by the shell after the app frame.
    /// Returns nothing; the shell drains [`Self::take_resolved`].
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let multi = self.is_multi();
        let filter = self.open_filter();
        let title = match self.pending.as_ref().map(|p| &p.kind) {
            Some(PickKind::OpenProject) => "Open project",
            Some(PickKind::Relink { .. }) => "Choose file",
            _ => "Import media",
        };
        let mut done: Option<Vec<String>> = None;
        let mut cancelled = false;

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
                match self.entries(&filter) {
                    Ok(entries) => {
                        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                            for entry in entries {
                                if entry.is_dir {
                                    if ui.button(format!("📁 {}", entry.name)).clicked() {
                                        self.trail.push(entry);
                                        self.selected.clear();
                                        self.highlighted = None;
                                    }
                                } else if multi {
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
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                    let ids: Vec<String> = if multi {
                        self.selected.clone()
                    } else {
                        self.highlighted.clone().into_iter().collect()
                    };
                    if ui.add_enabled(!ids.is_empty(), egui::Button::new(if multi { "Import" } else { "Open" })).clicked() {
                        done = Some(ids);
                    }
                    if ui.button("Browse other locations…").clicked() {
                        self.browse_elsewhere(multi, &filter);
                    }
                });
            });
        if cancelled {
            self.pending = None;
        } else if let Some(ids) = done {
            self.answer_ids(&ids);
        }
    }

    /// "Browse other locations…": hand the reply to the SAF picker and close
    /// the browser without answering; the SAF result answers later.
    fn browse_elsewhere(&mut self, multiple: bool, extensions: &[String]) {
        let mimes: Vec<String> =
            extensions.iter().filter_map(|e| saf::mime_for_extension(e)).map(str::to_string).collect();
        match saf::launch_open_document(multiple, &mimes) {
            Ok(()) => {
                let (kind, relink_hint) = match self.pending.as_ref().map(|p| &p.kind) {
                    Some(PickKind::Import { .. }) => (SafKind::Import, None),
                    Some(PickKind::OpenProject) => (SafKind::OpenProject, None),
                    Some(PickKind::Relink { hint, .. }) => (SafKind::Relink, Some(hint.clone())),
                    None => return,
                };
                let cache_dir = self.pending.as_ref().map(|p| p.cache_dir.clone()).unwrap_or_else(std::env::temp_dir);
                let reply = SafReply {
                    cache_dir,
                    storage: self.storage.clone(),
                    slot: self.resolved.clone(),
                    kind,
                    relink_hint,
                };
                saf::set_pending_open(Box::new(reply), multiple, self.storage.clone());
                // `pending` is dropped: the window closes and SAF answers later.
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
fn list_dir(dir: &Path, filter: &[String]) -> Result<Vec<FsEntry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|e| format!("couldn't read a directory entry: {e}"))?;
        let path = item.path();
        let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !filter.contains(&ext) {
                continue;
            }
        }
        let name = item.file_name().to_string_lossy().into_owned();
        entries.push(FsEntry { name, path, is_dir });
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });
    Ok(entries)
}
