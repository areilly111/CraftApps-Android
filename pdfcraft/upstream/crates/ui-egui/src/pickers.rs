//! Native file pickers that never block the frame.
//!
//! A blocking `rfd::FileDialog` runs `-[NSOpenPanel runModal]` on macOS: a nested run loop inside
//! winit's event handler. An event that arrives while it spins re-enters the handler, winit
//! panics, and because the panic can't unwind out of the AppKit callback the app aborts.
//! `rfd::AsyncFileDialog` returns at once instead, and the app uses the choice on a later frame
//! ([`PdfCraftApp::process_picked`]); a worker thread waits for it.
//!
//! How rfd 0.17 shows the panel (checked against its source):
//! - macOS: creating the future shows the panel, so it must be created on the main thread. With
//!   no parent given, rfd uses the app's main (or first) window and opens the panel as a sheet
//!   on it (`beginSheetModalForWindow`). Without a running app and a window it falls back to the
//!   blocking `runModal`, which is why this only runs inside the running eframe app.
//! - Windows: the panel runs on a thread rfd starts when the future is first polled (here, by
//!   the worker), not inside winit's callback.
//! - Linux and the BSDs: the XDG portal backend (with a Zenity fallback) also works off the UI
//!   thread; the optional GTK3 backend shows the panel on rfd's own GTK thread.
//! - Android: rfd has no backend (and doesn't compile there). [`PdfCraftApp::ask`] queues an
//!   [`AndroidPick`] instead, and the port's shell answers it from its own file browser; the
//!   answer runs the same `then` closure on a later frame.
//!
//! Every native picker goes through [`PdfCraftApp::ask`]: open, save, folder and multi-file
//! panels alike. What happens with the choice is a closure that runs on that later frame, so
//! work that depends on the file being written (closing the tab after Save, deleting extracted
//! pages) happens there and nowhere else. One picker shows at a time, and it keeps that claim
//! until its closure starts, so a closure that asks again (merge data, then save the
//! spreadsheet) always gets its second picker.

#[cfg(not(target_os = "android"))]
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::files::{FilePurpose, PickTarget};

/// The file-dialog builder. On desktop this is rfd's async dialog; on Android rfd has no
/// backend (it doesn't compile there), so this records the dialog's metadata (filters,
/// suggested name, title) and [`PdfCraftApp::ask`] hands it to the shell's file browser.
#[cfg(not(target_os = "android"))]
pub(crate) use rfd::AsyncFileDialog as FileDialog;

/// The file-dialog builder's Android stand-in: records what the dialog would have shown so
/// the shell's file browser can show the equivalent (see [`AndroidPickRequest`]).
#[cfg(target_os = "android")]
#[derive(Clone, Debug, Default)]
pub(crate) struct FileDialog {
    pub(crate) filters: Vec<(String, Vec<String>)>,
    pub(crate) file_name: Option<String>,
    pub(crate) title: Option<String>,
}

#[cfg(target_os = "android")]
impl FileDialog {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Mirrors `rfd::AsyncFileDialog::add_filter`'s signature so the call sites compile
    /// unchanged on every platform.
    pub(crate) fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(|e| e.to_string()).collect()));
        self
    }

    pub(crate) fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    pub(crate) fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// What kind of panel the Android shell's file browser should show for a queued pick.
#[cfg(target_os = "android")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndroidPickKind {
    /// Choose one existing file.
    Open,
    /// Choose one or more existing files.
    OpenMany,
    /// Choose where to save a file.
    Save,
    /// Choose a folder.
    Folder,
}

/// A file pick the Android shell's browser must answer (see [`PdfCraftApp::answer_android_pick`]).
/// The shell answers with plain filesystem paths; a SAF "save elsewhere" choice arrives as a
/// `content://` URI string, which [`PdfCraftApp::write_picked_file`] routes through the shell.
#[cfg(target_os = "android")]
#[derive(Clone, Debug)]
pub struct AndroidPickRequest {
    pub kind: AndroidPickKind,
    pub title: Option<String>,
    /// `(label, extensions)` pairs, as passed to the dialog builder.
    pub filters: Vec<(String, Vec<String>)>,
    /// Suggested file name (save panels).
    pub file_name: Option<String>,
}

#[cfg(target_os = "android")]
impl AndroidPickRequest {
    pub(crate) fn from_ask(ask: &Ask) -> Self {
        let (kind, dialog) = match ask {
            Ask::File(d) => (AndroidPickKind::Open, d),
            Ask::Files(d) => (AndroidPickKind::OpenMany, d),
            Ask::Folder(d) => (AndroidPickKind::Folder, d),
            Ask::Save(d) => (AndroidPickKind::Save, d),
        };
        AndroidPickRequest {
            kind,
            title: dialog.title.clone(),
            filters: dialog.filters.clone(),
            file_name: dialog.file_name.clone(),
        }
    }
}

/// A pick queued for the Android shell, with what to do once it answers.
#[cfg(target_os = "android")]
pub(crate) struct AndroidPick {
    pub(crate) request: AndroidPickRequest,
    pub(crate) target: Option<PickTarget>,
    pub(crate) then: Then,
}

/// What the chosen files are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickFor {
    /// File ▸ Open: open the chosen file in a new tab.
    Open,
    /// Combine, insert, replace or OCR (see [`FilePurpose`]).
    Files(FilePurpose),
}

/// Which native panel to show.
pub(crate) enum Ask {
    /// Choose one existing file.
    File(FileDialog),
    /// Choose one or more existing files.
    Files(FileDialog),
    /// Choose a folder.
    Folder(FileDialog),
    /// Choose where to save a file.
    Save(FileDialog),
}

/// What to do with the chosen paths, on a later frame.
type Then = Box<dyn FnOnce(&mut PdfCraftApp, Vec<PathBuf>) + Send>;

/// A finished pick, waiting for the next frame.
struct Picked {
    target: Option<PickTarget>,
    /// The chosen paths; empty when the picker was cancelled.
    paths: Vec<PathBuf>,
    then: Then,
}

impl std::fmt::Debug for Picked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picked").field("target", &self.target).field("paths", &self.paths).finish_non_exhaustive()
    }
}

/// Pickers in flight and the picks they finished.
#[derive(Clone, Debug, Default)]
pub struct Pickers {
    /// A picker is showing, or its pick is waiting for the frame; a second request is refused
    /// until the pick's closure starts.
    showing: Arc<AtomicBool>,
    done: Arc<Mutex<Vec<Picked>>>,
}

/// Clears `showing` if the worker ends without queueing a pick (it panicked). Desktop only:
/// on Android no worker ever spawns (see [`PdfCraftApp::ask`]).
#[cfg(not(target_os = "android"))]
struct Showing(Option<Arc<AtomicBool>>);

#[cfg(not(target_os = "android"))]
impl Showing {
    /// The pick is queued: the frame that uses it clears the flag.
    fn hand_over(mut self) {
        self.0 = None;
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Showing {
    fn drop(&mut self) {
        if let Some(flag) = &self.0 {
            flag.store(false, Ordering::SeqCst);
        }
    }
}

impl Pickers {
    /// Wait for `pick` on a worker thread and queue its result for the next frame. On macOS
    /// `pick` must already have shown the picker (rfd starts the panel when the future is
    /// created, on the main thread). Returns false when the worker couldn't start.
    /// Desktop only: on Android [`PdfCraftApp::ask`] queues the pick for the shell instead.
    #[cfg(not(target_os = "android"))]
    fn spawn<F>(&self, target: Option<PickTarget>, ctx: Option<egui::Context>, pick: F, then: Then) -> bool
    where
        F: Future<Output = Vec<PathBuf>> + Send + 'static,
    {
        self.spawn_worker(target, ctx, pick, then).is_some()
    }

    /// [`Self::spawn`], returning the worker so tests can wait for it instead of polling.
    #[cfg(not(target_os = "android"))]
    fn spawn_worker<F>(&self, target: Option<PickTarget>, ctx: Option<egui::Context>, pick: F, then: Then) -> Option<std::thread::JoinHandle<()>>
    where
        F: Future<Output = Vec<PathBuf>> + Send + 'static,
    {
        let showing = Showing(Some(self.showing.clone()));
        let done = self.done.clone();
        std::thread::Builder::new()
            .name("file-picker".into())
            .spawn(move || {
                let paths = pollster::block_on(pick);
                done.lock().unwrap_or_else(PoisonError::into_inner).push(Picked { target, paths, then });
                showing.hand_over();
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            })
            .ok()
    }

    /// Queue a pick that is already known (tests and automation). Like a real pick, it keeps
    /// the flag until the frame uses it.
    fn deliver(&self, target: Option<PickTarget>, paths: Vec<PathBuf>, then: Then) {
        self.done.lock().unwrap_or_else(PoisonError::into_inner).push(Picked { target, paths, then });
    }

    fn take(&self) -> Vec<Picked> {
        std::mem::take(&mut *self.done.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl PdfCraftApp {
    /// Show a native picker for `pick_for` without blocking the frame. The choice is used on a
    /// later frame by [`Self::process_picked`] (desktop) or [`Self::answer_android_pick`].
    pub(crate) fn pick(&mut self, pick_for: PickFor, dialog: FileDialog, multiple: bool) {
        // Insert and Replace edit the active document: remember which one.
        let target = match pick_for {
            PickFor::Files(purpose) if purpose.edits_active_document() => self.active_ids().map(|(_, id)| id),
            _ => None,
        };
        let ask = if multiple { Ask::Files(dialog) } else { Ask::File(dialog) };
        self.ask(ask, target, move |app, paths| match pick_for {
            PickFor::Open => paths.iter().for_each(|p| app.open_path(&p.to_string_lossy())),
            PickFor::Files(purpose) => app.use_paths(purpose, &paths),
        });
    }

    /// Show `ask` without blocking the frame, and call `then` with the chosen paths on a later
    /// frame ([`Self::process_picked`]; on Android [`Self::answer_android_pick`]). Returns
    /// false, after telling the user, when no picker could be shown: another one is still
    /// open, or its worker couldn't start.
    ///
    /// `then` is not called when the picker is cancelled, or when `target` is given and by then
    /// that document is no longer the active one or has been edited (the user is told). Use
    /// `target` whenever `then` edits the active document or refers to its pages or objects by
    /// position.
    ///
    /// `pick_override` answers instead of a native panel (tests and automation); the answer
    /// still arrives on a later frame, like a real pick.
    pub(crate) fn ask(&mut self, ask: Ask, target: Option<DocId>, then: impl FnOnce(&mut PdfCraftApp, Vec<PathBuf>) + Send + 'static) -> bool {
        if self.pickers.showing.swap(true, Ordering::SeqCst) {
            self.notify_tr("Another file dialog is still open. Finish with it first.");
            return false;
        }
        let target = self.pick_target(target);
        let then: Then = Box::new(then);
        if let Some(paths) = self.pick_override.clone() {
            self.pickers.deliver(target, paths.into_iter().map(PathBuf::from).collect(), then);
            return true;
        }
        // Android: rfd has no backend there. Queue the pick (with the dialog's metadata) for
        // the shell's file browser; it answers through `answer_android_pick`.
        #[cfg(target_os = "android")]
        {
            let request = AndroidPickRequest::from_ask(&ask);
            self.android_picks.push_back(AndroidPick { request, target, then });
            // The browser draws on top of the app: wake the frame so it appears at once.
            if let Some(ctx) = self.ctx.clone() {
                ctx.request_repaint();
            }
            true
        }
        // On macOS creating the future shows the panel; it must happen here, on the main thread.
        #[cfg(not(target_os = "android"))]
        {
            let ctx = self.ctx.clone();
            let one = |h: Option<rfd::FileHandle>| h.map(|h| h.path().to_path_buf()).into_iter().collect::<Vec<_>>();
            let started = match ask {
                Ask::File(d) => {
                    let pick = d.pick_file();
                    self.pickers.spawn(target, ctx, async move { one(pick.await) }, then)
                }
                Ask::Folder(d) => {
                    let pick = d.pick_folder();
                    self.pickers.spawn(target, ctx, async move { one(pick.await) }, then)
                }
                Ask::Save(d) => {
                    let pick = d.save_file();
                    self.pickers.spawn(target, ctx, async move { one(pick.await) }, then)
                }
                Ask::Files(d) => {
                    let pick = d.pick_files();
                    self.pickers.spawn(
                        target,
                        ctx,
                        async move { pick.await.unwrap_or_default().into_iter().map(|h| h.path().to_path_buf()).collect() },
                        then,
                    )
                }
            };
            if !started {
                self.pickers.showing.store(false, Ordering::SeqCst);
                self.notify_tr("Couldn't show the file picker. Please try again.");
            }
            started
        }
    }

    /// [`Self::ask`] for one path: `then` gets the first chosen path.
    pub(crate) fn ask_one(&mut self, ask: Ask, target: Option<DocId>, then: impl FnOnce(&mut PdfCraftApp, PathBuf) + Send + 'static) -> bool {
        self.ask(ask, target, move |app, paths| {
            if let Some(path) = paths.into_iter().next() {
                then(app, path);
            }
        })
    }

    /// Use the picks that finished since the last frame. Called every frame.
    pub(crate) fn process_picked(&mut self) {
        for Picked { target, paths, then } in self.pickers.take() {
            // The pick is in hand: the next picker may show, including one `then` asks for.
            self.pickers.showing.store(false, Ordering::SeqCst);
            if paths.is_empty() {
                continue;
            }
            if !self.still_pick_target(target) {
                continue;
            }
            // Last-resort guard (AGENTS.md §4), as for commands: this work used to run inside
            // the command that asked, and a panic in it must not take the app down.
            if let Err(m) = pdfcraft_engine::guard(|| then(self, paths)) {
                self.notify_fmt("That didn't work: an internal error stopped it ({m}).", &[("m", m.as_str())]);
            }
        }
    }

    /// The oldest pick the Android shell hasn't answered yet, if any. The shell shows its
    /// file browser for it and answers through [`Self::answer_android_pick`].
    #[cfg(target_os = "android")]
    pub fn android_pick_request(&self) -> Option<AndroidPickRequest> {
        self.android_picks.front().map(|p| p.request.clone())
    }

    /// Answer the oldest queued Android pick: run its follow-up with `paths` on this frame
    /// (empty = cancelled). The follow-up runs behind the engine's panic guard, like
    /// [`Self::process_picked`], and the answer may be a `content://` URI string for a SAF
    /// "save elsewhere" choice ([`Self::write_picked_file`] routes the write).
    #[cfg(target_os = "android")]
    pub fn answer_android_pick(&mut self, paths: Vec<PathBuf>) {
        // The pick is in hand: the next picker may show, including one `then` asks for.
        self.pickers.showing.store(false, Ordering::SeqCst);
        let Some(pick) = self.android_picks.pop_front() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        if !self.still_pick_target(pick.target) {
            return;
        }
        if let Err(m) = pdfcraft_engine::guard(|| (pick.then)(self, paths)) {
            self.notify_fmt("That didn't work: an internal error stopped it ({m}).", &[("m", m.as_str())]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(into: &Arc<Mutex<Vec<PathBuf>>>) -> Then {
        let into = into.clone();
        Box::new(move |_, paths| into.lock().unwrap().extend(paths))
    }

    #[test]
    fn worker_queues_the_pick_and_keeps_the_flag_until_the_frame_uses_it() {
        let mut app = PdfCraftApp::new();
        app.pickers.showing.store(true, Ordering::SeqCst);
        let got = Arc::new(Mutex::new(Vec::new()));
        let worker = app.pickers.spawn_worker(None, None, async { vec![PathBuf::from("a.pdf")] }, record(&got)).expect("worker starts");
        // Wait for the worker itself: polling with a deadline was flaky on loaded CI runners.
        assert!(worker.join().is_ok());
        assert!(app.pickers.showing.load(Ordering::SeqCst), "the queued pick still owns the flag");
        app.process_picked();
        assert!(!app.pickers.showing.load(Ordering::SeqCst));
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("a.pdf")]);
    }

    #[test]
    fn a_panicking_worker_clears_showing_and_queues_nothing() {
        let pickers = Pickers::default();
        pickers.showing.store(true, Ordering::SeqCst);
        fn fail() -> Vec<PathBuf> {
            panic!("picker failed")
        }
        let got = Arc::new(Mutex::new(Vec::new()));
        let worker = pickers.spawn_worker(None, None, async { fail() }, record(&got)).expect("worker starts");
        // join returns once the panic has unwound and the drop guard has run.
        assert!(worker.join().is_err());
        assert!(!pickers.showing.load(Ordering::SeqCst));
        assert!(pickers.take().is_empty());
    }

    #[test]
    fn the_follow_up_runs_on_a_later_frame_and_not_when_cancelled() {
        let mut app = PdfCraftApp::new();
        let got = Arc::new(Mutex::new(Vec::new()));
        app.pickers.deliver(None, vec![], record(&got));
        app.pickers.deliver(None, vec![PathBuf::from("b.pdf")], record(&got));
        assert!(got.lock().unwrap().is_empty(), "nothing runs until the frame uses the picks");
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("b.pdf")], "a cancelled pick runs nothing");
    }

    #[test]
    fn a_second_picker_is_refused_while_one_is_showing() {
        let mut app = PdfCraftApp::new();
        app.pick_override = Some(vec!["c.pdf".into()]);
        app.pickers.showing.store(true, Ordering::SeqCst);
        let got = Arc::new(Mutex::new(Vec::new()));
        let into = got.clone();
        assert!(!app.ask(Ask::File(FileDialog::new()), None, move |_, paths| into.lock().unwrap().extend(paths)));
        app.process_picked();
        assert!(got.lock().unwrap().is_empty());
        assert!(app.pickers.showing.load(Ordering::SeqCst), "the showing picker still owns the flag");
        assert!(app.toast.as_ref().is_some_and(|(m, _)| m.contains("Another file dialog")), "the user is told");
    }

    #[test]
    fn a_follow_up_that_asks_again_gets_its_picker_even_if_another_request_came_first() {
        let mut app = PdfCraftApp::new();
        app.pick_override = Some(vec!["first.fdf".into()]);
        let got = Arc::new(Mutex::new(Vec::new()));
        let into = got.clone();
        assert!(app.ask(Ask::Files(FileDialog::new()), None, move |app, _| {
            // Merge data: the spreadsheet's save picker, asked from the first pick's closure.
            assert!(app.ask(Ask::Save(FileDialog::new()), None, move |_, paths| into.lock().unwrap().extend(paths)));
        }));
        // Another command asks for a picker before the frame uses the first pick: refused.
        assert!(!app.ask(Ask::File(FileDialog::new()), None, |_, _| panic!("must not run")));
        app.process_picked();
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("first.fdf")], "the chained picker answered");
    }

    #[test]
    fn a_panicking_follow_up_is_reported_and_the_next_pick_still_runs() {
        let mut app = PdfCraftApp::new();
        let got = Arc::new(Mutex::new(Vec::new()));
        app.pickers.deliver(None, vec![PathBuf::from("x.pdf")], Box::new(|_, _| panic!("follow-up failed")));
        app.pickers.deliver(None, vec![PathBuf::from("y.pdf")], record(&got));
        app.process_picked();
        assert_eq!(*got.lock().unwrap(), vec![PathBuf::from("y.pdf")]);
        let toast = app.toast.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(toast.contains("internal error"), "the user is told: {toast:?}");
        assert!(!app.pickers.showing.load(Ordering::SeqCst));
    }
}
