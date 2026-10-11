//! Durable MCP checkpoints, isolated from the desktop and other headless sessions.

use std::path::{Path, PathBuf};

use effectcraft_engine::config::{FileOps, StdFiles};
use effectcraft_engine::{Session, autosave, prefs::Prefs};
use serde_json::{Value, json};

pub(crate) struct AutoSave {
    folder: PathBuf,
    latest: Option<String>,
    error: Option<String>,
    previous: Vec<Value>,
    manifest: Option<Value>,
}

impl AutoSave {
    pub fn new(root: &Path, prefs: &Prefs) -> std::io::Result<Self> {
        let base = if prefs.auto_save.location == "custom" && !prefs.auto_save.folder.trim().is_empty() {
            PathBuf::from(prefs.auto_save.folder.trim()).join("MCP")
        } else {
            root.join(autosave::AUTOSAVE_FOLDER).join("MCP")
        };
        if base.to_str().is_none() {
            return Err(std::io::Error::other("the MCP auto-save folder must be valid UTF-8; set EFFECTCRAFT_CONFIG_DIR to a UTF-8 path"));
        }
        std::fs::create_dir_all(&base)?;
        let previous = previous_sessions(&base);
        let folder = unique_folder(&base)?;
        Ok(Self { folder, latest: None, error: None, previous, manifest: None })
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn info(&self) -> Value {
        json!({
            "enabled": true, "folder": self.folder.to_string_lossy(), "latestPath": self.latest,
            "error": self.error, "previousSessions": self.previous,
            "recovery": "Recover with open_project {path: latestPath}, then save_project {path: a normal project file}. Previous sessions may still be running; they are never opened automatically.",
        })
    }

    pub fn checkpoint(&mut self, s: &mut Session, ended: bool) -> std::io::Result<()> {
        let result = self.write(s, ended);
        if let Err(e) = &result {
            self.error = Some(format!("MCP auto-save in {} failed: {e}", self.folder.display()));
        }
        result.map_err(|e| std::io::Error::other(format!("MCP auto-save in {} failed: {e}", self.folder.display())))
    }

    fn write(&mut self, s: &mut Session, ended: bool) -> std::io::Result<()> {
        if s.is_dirty() && s.autosave.saved_revision != Some(s.revision) {
            s.autosave.background = false;
            s.autosave_now().map_err(std::io::Error::other)?;
        }
        s.autosave_wait().map_err(std::io::Error::other)?;
        // Explicit file.autoSave and save-on-render-start use the same override. Keep their
        // successful latest path too, even when the revision was already checkpointed.
        if let Some(path) = &s.autosave.last_path {
            self.latest = Some(path.clone());
        }
        let record = json!({
            "pid": autosave::pid(), "project": s.path, "autosave": self.latest,
            "dirty": s.is_dirty(), "revision": s.revision, "ended": ended,
        });
        // Queries don't rewrite the same manifest or rotate project slots.
        if self.manifest.as_ref() != Some(&record) && self.latest.is_some() {
            StdFiles.write(&self.folder.join("session.json"), record.to_string().as_bytes())?;
            self.manifest = Some(record);
        }
        self.error = None;
        if ended && self.latest.is_none() {
            let _ = std::fs::remove_dir(&self.folder);
        }
        Ok(())
    }
}

fn unique_folder(base: &Path) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    for _ in 0..100 {
        let folder = base.join(format!("{}-{stamp}-{}", autosave::pid(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("cannot reserve a unique MCP auto-save folder"))
}

fn previous_sessions(base: &Path) -> Vec<Value> {
    // Scan directory metadata with bounded memory, then read only the newest 200 small
    // manifests. Limiting read_dir before sorting could hide a newly killed session once
    // older folders accumulated. Recovery is explicit, so running sessions are safe to list.
    let Ok(entries) = std::fs::read_dir(base) else { return vec![] };
    let mut records = std::collections::BTreeSet::new();
    for entry in entries.flatten() {
        let path = entry.path().join("session.json");
        let Ok(meta) = path.metadata() else { continue };
        if !meta.is_file() || meta.len() > 64 * 1024 {
            continue;
        }
        records.insert((meta.modified().ok(), path));
        if records.len() > 200 {
            records.pop_first();
        }
    }
    records
        .into_iter()
        .rev()
        .filter_map(|(_, path)| {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path).ok()?.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 64 * 1024 {
                return None;
            }
            let record: Value = serde_json::from_slice(&bytes).ok()?;
            let saved = record.get("autosave")?.as_str()?;
            if saved.len() > 4096 || record.get("dirty") != Some(&Value::Bool(true)) || !Path::new(saved).is_file() {
                return None;
            }
            let project = record.get("project").and_then(Value::as_str).filter(|p| p.len() <= 4096);
            Some(json!({"autosave": saved, "project": project, "dirty": true,
                "ended": record.get("ended").and_then(Value::as_bool).unwrap_or(false),
                "pid": record.get("pid").and_then(Value::as_u64),
            }))
        })
        .take(5)
        .collect()
}
