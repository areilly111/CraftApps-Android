//! Android shim for `rfd` 0.17.
//!
//! `rfd` has no Android backend, so on Android this crate — which carries the
//! same package name — takes its place in the dependency graph. It implements
//! the exact subset of `rfd` 0.17's API that `pdfcraft-ui-egui` uses:
//! [`AsyncFileDialog`] (builder) and [`FileHandle`].
//!
//! Instead of showing a desktop native dialog, each terminal method
//! ([`AsyncFileDialog::pick_file`], [`pick_files`](AsyncFileDialog::pick_files),
//! [`pick_folder`](AsyncFileDialog::pick_folder),
//! [`save_file`](AsyncFileDialog::save_file)) builds a [`PickRequest`] and
//! hands it to the handler installed by [`set_pick_handler`]. The Android
//! shell (the `pdfcraft-android` crate) installs a handler that launches the
//! matching Storage Access Framework picker and delivers the chosen paths
//! through the returned channel. The future resolves on the worker thread
//! that `Pickers::spawn` parks on, exactly like the desktop rfd future.
//!
//! If no handler is installed (tests, or a programming error), the pick
//! resolves to `None`/empty immediately instead of hanging.

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Which picker the shell should show.
#[derive(Clone, Debug)]
pub enum PickKind {
    /// Choose one existing file.
    File,
    /// Choose one or more existing files.
    Files,
    /// Choose a folder.
    Folder,
    /// Choose where to save a file.
    Save,
}

/// What the app asked for, handed to the shell's pick handler.
#[derive(Clone, Debug)]
pub struct PickRequest {
    /// Which picker to show.
    pub kind: PickKind,
    /// Dialog title, when the app set one.
    pub title: Option<String>,
    /// Suggested file name, for save dialogs.
    pub file_name: Option<String>,
    /// `(label, extensions)` filter pairs, in the order the app added them.
    pub filters: Vec<(String, Vec<String>)>,
}

/// Install the shell's pick handler. The handler runs on the worker thread
/// that awaits the pick future; it must launch the SAF picker and return a
/// channel that receives the chosen paths (empty when cancelled).
pub fn set_pick_handler(handler: impl Fn(PickRequest) -> std::sync::mpsc::Receiver<Vec<PathBuf>> + Send + Sync + 'static) {
    let _ = HANDLER.set(Box::new(handler));
}

type PickHandler = Box<dyn Fn(PickRequest) -> std::sync::mpsc::Receiver<Vec<PathBuf>> + Send + Sync>;
static HANDLER: OnceLock<PickHandler> = OnceLock::new();

fn handle_request(req: PickRequest) -> Vec<PathBuf> {
    match HANDLER.get() {
        Some(h) => h(req).recv().unwrap_or_default(),
        None => {
            log_warn("rfd shim: no pick handler installed; resolving the pick as cancelled");
            Vec::new()
        }
    }
}

fn log_warn(msg: &str) {
    // No logging dependency in the shim; `eprintln` is invisible on Android
    // but harmless, and useful when the shim is exercised on desktop tests.
    eprintln!("{msg}");
}

/// Builder for an async file dialog. Mirrors the `rfd` 0.17 builder API used
/// by `pdfcraft-ui-egui`; every method records its argument for the
/// [`PickRequest`].
#[derive(Clone, Debug, Default)]
pub struct AsyncFileDialog {
    title: Option<String>,
    file_name: Option<String>,
    filters: Vec<(String, Vec<String>)>,
}

impl AsyncFileDialog {
    /// Start building a dialog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Dialog title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Suggested file name (save dialogs).
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// Add a `(label, extensions)` filter row.
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(|s| s.to_string()).collect()));
        self
    }

    /// Set the starting directory. Recorded but currently unused: the SAF
    /// picker opens where the system decides. Kept for API parity with rfd.
    #[allow(dead_code)]
    pub fn set_directory(self, _path: impl Into<PathBuf>) -> Self {
        self
    }

    fn request(self, kind: PickKind) -> PickRequest {
        PickRequest { kind, title: self.title, file_name: self.file_name, filters: self.filters }
    }

    /// Choose one existing file.
    pub fn pick_file(self) -> impl Future<Output = Option<FileHandle>> + Send {
        let req = self.request(PickKind::File);
        async move { handle_request(req).into_iter().next().map(|path| FileHandle { path }) }
    }

    /// Choose one or more existing files.
    pub fn pick_files(self) -> impl Future<Output = Option<Vec<FileHandle>>> + Send {
        let req = self.request(PickKind::Files);
        async move {
            let paths = handle_request(req);
            if paths.is_empty() { None } else { Some(paths.into_iter().map(|path| FileHandle { path }).collect()) }
        }
    }

    /// Choose a folder.
    pub fn pick_folder(self) -> impl Future<Output = Option<FileHandle>> + Send {
        let req = self.request(PickKind::Folder);
        async move { handle_request(req).into_iter().next().map(|path| FileHandle { path }) }
    }

    /// Choose where to save a file.
    pub fn save_file(self) -> impl Future<Output = Option<FileHandle>> + Send {
        let req = self.request(PickKind::Save);
        async move { handle_request(req).into_iter().next().map(|path| FileHandle { path }) }
    }
}

/// A picked file. Mirrors the `rfd` 0.17 methods `pdfcraft-ui-egui` uses.
#[derive(Clone, Debug)]
pub struct FileHandle {
    path: PathBuf,
}

impl FileHandle {
    /// The picked path. For save picks on Android this may be a `content://`
    /// URI string: write it with the SAF writer, not `std::fs`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// File name of the picked path. Kept for API parity with rfd.
    #[allow(dead_code)]
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Read the picked file's bytes. Kept for API parity with rfd.
    #[allow(dead_code)]
    pub fn read(&self) -> impl Future<Output = Vec<u8>> + Send {
        let path = self.path.clone();
        async move { std::fs::read(&path).unwrap_or_default() }
    }
}

impl fmt::Display for FileHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())
    }
}
