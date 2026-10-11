//! Android host services for the FilmCraft port.
//!
//! Two layers, mirroring the desktop's `apps/filmcraft/src/main.rs`:
//!
//! * [`AndroidServices`] implements the engine's [`filmcraft_engine::Services`]
//!   (file access): plain paths go to the filesystem (atomic writes, like the
//!   desktop's `FsServices`), `content://` URIs (system picker saves) are
//!   routed through the shared SAF module.
//! * [`install_hooks`] wires the UI's [`filmcraft_ui_egui::HostHooks`]. The
//!   pickers are asynchronous — the web app's pattern
//!   (`apps/filmcraft-web/src/import.rs`): a hook opens the in-app browser
//!   and returns "no answer"; when the user confirms, the shell runs the
//!   engine command directly. Saves use deterministic paths under the shared
//!   media folder (the web's `/exports/<name>` pattern), so the UI flow
//!   continues immediately; the export dialogs report where the file landed.

use crate::file_browser::{FileBrowser, PickKind};
use craftapps_shared::saf;
use filmcraft_ui_egui::{FilmcraftApp, RelinkHint};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn io_err(e: String) -> std::io::Error {
    std::io::Error::other(e)
}

/// Engine file access on Android: the filesystem for plain paths, SAF for
/// `content://` URIs.
pub struct AndroidServices {
    /// Where the Media Browser starts (shared media dir, or the app files dir).
    media_root: PathBuf,
}

impl AndroidServices {
    pub fn new(media_root: PathBuf) -> Arc<Self> {
        Arc::new(AndroidServices { media_root })
    }

    fn is_uri(path: &str) -> bool {
        path.starts_with("content://")
    }

    fn read_plain(path: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_uri(path: &str) -> std::io::Result<Vec<u8>> {
        saf::saf_read_uri(path).map(|(_, bytes)| bytes).map_err(io_err)
    }
}

impl filmcraft_engine::Services for AndroidServices {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        if Self::is_uri(path) {
            Self::read_uri(path)
        } else {
            Self::read_plain(path)
        }
    }

    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        if Self::is_uri(path) {
            saf::saf_write_uri(path, data).map_err(io_err)
        } else {
            // Atomic + durable, like the desktop's `FsServices`.
            filmcraft_format::atomic_write(Path::new(path), data)
        }
    }

    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        if Self::is_uri(path) {
            return self.read_file(path).map(|b| b.len() as u64);
        }
        let m = std::fs::metadata(path)?;
        if m.is_file() {
            Ok(m.len())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{path} is not a file")))
        }
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        if Self::is_uri(path) {
            // A URI has no cheap range read: fetch and slice (the default).
            let b = self.read_file(path)?;
            let a = (offset as usize).min(b.len());
            return Ok(b[a..(a + len).min(b.len())].to_vec());
        }
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        let mut read = 0;
        while read < len {
            match f.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(e) => return Err(e),
            }
        }
        buf.truncate(read);
        Ok(buf)
    }

    /// Random-access reads for media files, so containers open without
    /// reading the whole file (plain paths only; URIs read whole).
    #[cfg(unix)]
    fn reader(&self, path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        if Self::is_uri(path) {
            return None;
        }
        Some(filmcraft_media::reader::FileReader::open(Path::new(path)).map(|r| Arc::new(r) as filmcraft_media::SharedReader))
    }

    fn list_dir(&self, dir: &str) -> Option<std::io::Result<Vec<String>>> {
        if Self::is_uri(dir) || dir.is_empty() {
            return None;
        }
        Some(
            std::fs::read_dir(dir)
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
                .map_err(|e| std::io::Error::other(format!("couldn't list {dir}: {e}"))),
        )
    }

    fn file_loader(&self) -> Option<filmcraft_media::sequence::FrameLoader> {
        Some(Arc::new(|p: &str| {
            if Self::is_uri(p) {
                Self::read_uri(p)
            } else {
                Self::read_plain(p)
            }
        }))
    }

    fn home_dir(&self) -> Option<String> {
        Some(self.media_root.to_string_lossy().into_owned())
    }
}

/// Directories the port keeps under the media root.
pub struct PortDirs {
    /// `<media>/FilmCraft/Projects` — deterministic `pick_save` targets.
    pub projects: PathBuf,
    /// `<media>/FilmCraft/Exports` — deterministic `pick_save_as` targets.
    pub exports: PathBuf,
}

impl PortDirs {
    pub fn new(media_root: &Path) -> Self {
        let base = media_root.join("FilmCraft");
        PortDirs { projects: base.join("Projects"), exports: base.join("Exports") }
    }

    pub fn ensure(&self) -> Result<(), String> {
        for d in [&self.projects, &self.exports] {
            std::fs::create_dir_all(d).map_err(|e| format!("couldn't create {}: {e}", d.display()))?;
        }
        Ok(())
    }
}

/// File name without directory separators (the UI suggests plain names).
fn safe_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    if n.trim().is_empty() {
        "Untitled".to_string()
    } else {
        n
    }
}

/// Wire the UI hooks: async pickers through the in-app browser (the web
/// pattern), deterministic save paths, Android URL opening.
pub fn install_hooks(
    app: &mut FilmcraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    cache_dir: PathBuf,
    dirs: PortDirs,
) {
    // File ▸ Import (and the other `pick_files` flows): open the browser;
    // the shell runs `file.import` on the answer.
    let pick_browser = browser.clone();
    let pick_cache = cache_dir.clone();
    app.hooks.pick_files = Some(Box::new(move |exts: &[&str]| {
        let extensions = exts.iter().map(|e| e.to_string()).collect();
        if let Ok(mut b) = pick_browser.lock() {
            b.push(PickKind::Import { extensions }, pick_cache.clone());
        }
        Vec::new()
    }));
    // File ▸ Open…: open the browser; the shell runs `file.open` on the answer.
    let project_browser = browser.clone();
    let project_cache = cache_dir.clone();
    app.hooks.pick_open_project = Some(Box::new(move || {
        if let Ok(mut b) = project_browser.lock() {
            b.push(PickKind::OpenProject, project_cache.clone());
        }
        None
    }));
    // Saves: deterministic paths under the shared media folder (the web
    // app's `/projects/<name>` pattern), so the UI flow continues at once.
    let save_projects = dirs.projects.clone();
    app.hooks.pick_save = Some(Box::new(move |name: &str| {
        Some(save_projects.join(safe_name(name)).to_string_lossy().into_owned())
    }));
    let export_dir = dirs.exports.clone();
    app.hooks.pick_save_as = Some(Box::new(move |_filter: &str, _exts: &[&str], name: &str| {
        Some(export_dir.join(safe_name(name)).to_string_lossy().into_owned())
    }));
    // Link Media ▸ Locate…, Attach Proxies, Reconnect Full Resolution: the
    // picker is async, so the hint's command runs once the user has chosen
    // (the web app's #111 pattern).
    let relink_browser = browser.clone();
    let relink_cache = cache_dir;
    app.hooks.pick_file_for_relink = Some(Box::new(move |exts: &[&str], hint: Option<RelinkHint>| {
        let hint = hint?;
        let extensions = exts.iter().map(|e| e.to_string()).collect();
        if let Ok(mut b) = relink_browser.lock() {
            b.push(PickKind::Relink { extensions, hint }, relink_cache.clone());
        }
        None
    }));
    // Help menu and About links: the Android browser.
    app.hooks.open_url = Some(Box::new(|url: &str| craftapps_shared::jni_bridge::open_url(url)));
}
