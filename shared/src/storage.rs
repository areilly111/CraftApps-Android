//! Shared suite storage: `/sdcard/CraftApps/Media` (documents) and
//! `/sdcard/CraftApps/Settings/PhotoCraft` (prefs, crash recovery, brush
//! presets, updater settings).
//!
//! With `MANAGE_EXTERNAL_STORAGE` ("All files access") these are ordinary
//! directories for Rust's `std::fs` — no SAF needed. The SD-card option keeps
//! its media in a persisted SAF tree instead, reached through [`crate::saf`].
//!
//! Everything here is fallible with `Err(String)` for the UI; nothing panics.

use crate::jni_bridge;
use crate::saf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where documents live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaLocation {
    Tablet,
    SdCard,
}

/// Owns the suite directories and the media-location choice. `Send + Sync` so
/// the file dialogs, the services and the SAF result handler can share it.
/// Per-app identity for the shared storage helpers. Each port fills this once; the
/// `/sdcard/CraftApps` layout itself is identical across the suite.
pub struct StorageAppInfo {
    /// Display name, for logs (e.g. `"PhotoCraft"`).
    pub app_name: &'static str,
    /// Subfolder under `/sdcard/CraftApps/Settings/` (e.g. `"PhotoCraft"`). Keeps the
    /// suite's apps from colliding in shared settings.
    pub settings_folder: &'static str,
}

pub struct StorageManager {
    /// e.g. `/storage/emulated/0`.
    external_root: PathBuf,
    /// `<root>/CraftApps/Media`.
    tablet_media: PathBuf,
    /// `<root>/CraftApps/Settings/PhotoCraft`.
    settings_dir: PathBuf,
    /// `<root>/CraftApps/Plugins` (suite-shared WebAssembly plug-ins).
    plugins_dir: PathBuf,
    /// `<cache>/saf-inbox` (multi-open staging).
    inbox_dir: PathBuf,
    state: Mutex<State>,
}

struct State {
    location: MediaLocation,
    sd_tree_uri: Option<String>,
    /// Content-URI → display name (for export format detection on save).
    uri_names: HashMap<String, String>,
}

/// One-time migration of the legacy suite folder: early builds used
/// `<root>/ArtCraft`; the trademark-clean name is `<root>/CraftApps`.
/// If the old folder exists and the new one doesn't, rename it in place so
/// the user's documents, settings and presets carry over untouched.
fn migrate_legacy_suite_dir(external_root: &Path) {
    let legacy = external_root.join("ArtCraft");
    let current = external_root.join("CraftApps");
    if !legacy.is_dir() || current.exists() {
        return;
    }
    match std::fs::rename(&legacy, &current) {
        Ok(()) => log::info!(
            "migrated suite storage {} -> {}",
            legacy.display(),
            current.display()
        ),
        Err(e) => log::warn!(
            "couldn't migrate suite storage {} -> {}: {e}; using the new folder",
            legacy.display(),
            current.display()
        ),
    }
}

impl StorageManager {
    pub fn new(cache_dir: &Path, app: &StorageAppInfo) -> Result<Self, String> {
        let root = jni_bridge::external_storage_root()?;
        log::info!("{} shared storage root: {root}", app.app_name);
        let external_root = PathBuf::from(&root);
        // Migration (2026-10): the suite folder was renamed from `ArtCraft` to
        // `CraftApps` for trademark compliance. Move the old tree over once.
        migrate_legacy_suite_dir(&external_root);
        let tablet_media = external_root.join("CraftApps/Media");
        let settings_dir = external_root.join("CraftApps/Settings").join(app.settings_folder);
        let plugins_dir = external_root.join("CraftApps/Plugins");
        let inbox_dir = cache_dir.join("saf-inbox");
        let mut this = StorageManager {
            external_root,
            tablet_media,
            settings_dir,
            plugins_dir,
            inbox_dir,
            state: Mutex::new(State { location: MediaLocation::Tablet, sd_tree_uri: None, uri_names: HashMap::new() }),
        };
        this.load_state();
        Ok(this)
    }

    fn state_file(&self) -> PathBuf {
        self.settings_dir.join("storage.json")
    }

    /// Read back the persisted media location (best effort; defaults apply).
    fn load_state(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.state_file()) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let Ok(mut st) = self.state.lock() {
            if v.get("location").and_then(|l| l.as_str()) == Some("sdcard") {
                st.location = MediaLocation::SdCard;
            }
            st.sd_tree_uri = v.get("sd_tree_uri").and_then(|u| u.as_str()).map(str::to_string);
            // A remembered SD location without its tree is unusable: fall back.
            if st.location == MediaLocation::SdCard && st.sd_tree_uri.is_none() {
                st.location = MediaLocation::Tablet;
            }
        }
    }

    fn save_state(&self) {
        let (location, sd_tree_uri) = match self.state.lock() {
            Ok(st) => (
                match st.location {
                    MediaLocation::Tablet => "tablet",
                    MediaLocation::SdCard => "sdcard",
                }
                .to_string(),
                st.sd_tree_uri.clone(),
            ),
            Err(_) => return,
        };
        let text = serde_json::json!({ "location": location, "sd_tree_uri": sd_tree_uri }).to_string();
        let _ = std::fs::write(self.state_file(), text);
    }

    /// Create the suite directories (call once storage access is granted).
    /// Also clears the SAF multi-open staging dir.
    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [
            &self.tablet_media,
            &self.settings_dir,
            &self.settings_dir.join("Recovery"),
            &self.settings_dir.join("Presets"),
            &self.plugins_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        }
        // Stale staged opens from a previous run must not linger.
        let _ = std::fs::remove_dir_all(&self.inbox_dir);
        Ok(())
    }

    pub fn settings_dir(&self) -> &Path {
        &self.settings_dir
    }

    /// Suite-shared WebAssembly plug-in folder (`/sdcard/CraftApps/Plugins/`).
    /// Unused for now: the engine's `PlugIns` default already points at this
    /// path (see `crates/engine/src/prefs.rs`); the method stays for the
    /// sibling ports' shared crate.
    #[allow(dead_code)]
    pub fn plugins_dir(&self) -> &Path {
        &self.plugins_dir
    }

    pub fn saf_inbox_dir(&self) -> &Path {
        &self.inbox_dir
    }

    pub fn media_location(&self) -> MediaLocation {
        self.state.lock().map(|st| st.location).unwrap_or(MediaLocation::Tablet)
    }

    /// Human-readable current media location for Preferences › Storage.
    pub fn location_label(&self) -> String {
        match self.media_location() {
            MediaLocation::Tablet => format!("Tablet storage ({}/CraftApps/Media)", self.external_root.display()),
            MediaLocation::SdCard => "SD card (chosen folder)".to_string(),
        }
    }

    pub fn use_tablet_storage(&self) {
        if let Ok(mut st) = self.state.lock() {
            st.location = MediaLocation::Tablet;
        }
        self.save_state();
    }

    /// Remember a picked SD-card tree and switch to it. Called from the SAF
    /// result handler.
    pub fn set_sd_tree_uri(&self, tree_uri: &str) -> Result<(), String> {
        // Sanity: the tree must list before we trust it.
        saf::saf_tree_children(tree_uri, None).map_err(|e| format!("that folder couldn't be used: {e}"))?;
        if let Ok(mut st) = self.state.lock() {
            st.sd_tree_uri = Some(tree_uri.to_string());
            st.location = MediaLocation::SdCard;
        }
        self.save_state();
        Ok(())
    }

    fn sd_tree(&self) -> Option<String> {
        self.state.lock().ok().and_then(|st| {
            if st.location == MediaLocation::SdCard { st.sd_tree_uri.clone() } else { None }
        })
    }

    /// Remember a content-URI's display name (for export format detection).
    pub fn record_uri_name(&self, uri: &str, name: &str) {
        if let Ok(mut st) = self.state.lock() {
            st.uri_names.insert(uri.to_string(), name.to_string());
        }
    }

    /// Display name for a media id: the recorded name for content URIs, else
    /// the file name. Used so `export()` picks the right format for SAF saves.
    pub fn name_for_id(&self, id: &str) -> String {
        if let Ok(st) = self.state.lock()
            && let Some(name) = st.uri_names.get(id)
        {
            return name.clone();
        }
        Path::new(id)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.to_string())
    }

    // ------------------------------------------------------------------
    // Media I/O. Ids are filesystem paths on tablet storage, document-URI
    // strings on the SD card. `None` = the media root.

    /// List a directory: entries sorted directories-first, case-insensitive.
    pub fn list(&self, id: Option<&str>) -> Result<Vec<MediaEntry>, String> {
        match self.sd_tree() {
            Some(tree) => {
                let entries = saf::saf_tree_children(&tree, id)?;
                Ok(entries
                    .into_iter()
                    .map(|e| MediaEntry { name: e.name, is_dir: e.is_dir, id: e.uri })
                    .collect())
            }
            None => {
                let dir = match id {
                    Some(p) => PathBuf::from(p),
                    None => self.tablet_media.clone(),
                };
                let read = std::fs::read_dir(&dir).map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
                let mut entries = Vec::new();
                for entry in read {
                    let entry = entry.map_err(|e| format!("couldn't list {}: {e}", dir.display()))?;
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "?".to_string());
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    entries.push(MediaEntry { name, is_dir, id: path.to_string_lossy().into_owned() });
                }
                entries.sort_by(|a, b| {
                    b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                Ok(entries)
            }
        }
    }

    /// Read an entry: display name + bytes.
    pub fn read(&self, entry: &MediaEntry) -> Result<(String, Vec<u8>), String> {
        if entry.id.starts_with("content://") {
            saf::saf_read_uri(&entry.id)
        } else {
            let bytes =
                std::fs::read(&entry.id).map_err(|e| format!("couldn't read {}: {e}", entry.name))?;
            Ok((entry.name.clone(), bytes))
        }
    }

    /// Reserve a save target inside directory `parent_id` and return its id.
    /// The Save dialog picks the location before the app has encoded any
    /// bytes; the bytes arrive later through [`Self::write_id`]. On the SD
    /// card this creates the (empty) document now; on tablet storage the file
    /// is created by the write. An existing file of the same name is reused
    /// (the save overwrites it).
    pub fn reserve(&self, parent_id: Option<&str>, name: &str) -> Result<String, String> {
        let clean = clean_name(name)?;
        match self.sd_tree() {
            Some(tree) => {
                if let Ok(entries) = self.list(parent_id)
                    && let Some(existing) = entries.iter().find(|e| !e.is_dir && e.name == clean)
                {
                    return Ok(existing.id.clone());
                }
                let parent_uri = parent_id.unwrap_or(&tree).to_string();
                let uri = saf::saf_tree_create(&parent_uri, &clean, mime_for_name(&clean))?;
                self.record_uri_name(&uri, &clean);
                Ok(uri)
            }
            None => {
                let dir = match parent_id {
                    Some(p) => PathBuf::from(p),
                    None => self.tablet_media.clone(),
                };
                Ok(dir.join(&clean).to_string_lossy().into_owned())
            }
        }
    }

    /// Overwrite an existing id (filesystem path or content URI).
    pub fn write_id(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        if id.starts_with("content://") {
            saf::saf_write_uri(id, bytes)
        } else {
            crate::services::write_atomic(Path::new(id), bytes)
        }
    }

}

/// One file or folder in the media browser.
#[derive(Clone, Debug)]
pub struct MediaEntry {
    pub name: String,
    pub is_dir: bool,
    /// Filesystem path (tablet) or document URI (SD card).
    pub id: String,
}

/// Reject empty names and path separators (the browser edits a single name).
fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("enter a file name".to_string());
    }
    if name.contains(['/', '\\']) {
        return Err("the name can't contain slashes".to_string());
    }
    Ok(name.to_string())
}

fn mime_for_name(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    saf::mime_for_extension(&ext).unwrap_or("application/octet-stream")
}
