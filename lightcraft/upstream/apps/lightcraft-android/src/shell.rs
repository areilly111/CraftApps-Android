//! The eframe shell: the shared [`LightcraftApp`] plus the Android-only UI
//! (the file-browser dialogs, the updater, and the first-run storage
//! permission gate) drawn on top of it, and the `ui.json` settings writer.
//!
//! Upstream crates are not modified for the port beyond the small Android
//! hooks; everything Android-specific lives in this crate and is composed
//! here.

use crate::file_browser::FileBrowser;
use craftapps_shared::updater::Updater;
use eframe::egui;
use lightcraft_ui_egui::{LightcraftApp, UiState};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AndroidShell {
    inner: LightcraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    updater: Arc<Mutex<Updater>>,
    prefs: PrefsWriter,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
}

impl AndroidShell {
    pub fn new(
        app: LightcraftApp,
        browser: Arc<Mutex<FileBrowser>>,
        updater: Arc<Mutex<Updater>>,
        prefs: PrefsWriter,
        gate: Option<StorageGate>,
    ) -> Self {
        AndroidShell { inner: app, browser, updater, prefs, gate }
    }
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.inner.logic(ctx);
        self.prefs.tick(&mut self.inner, ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.inner.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The storage gate blocks the whole app until the user grants shared
        // storage or explicitly chooses internal storage. Nothing behind it is
        // reachable, so there is no dead end.
        if let Some(gate) = self.gate.as_mut() {
            if gate.draw(ui) {
                self.gate = None;
            }
            return;
        }
        self.inner.ui(ui);
        // Drawn after the app, so they sit on top: the file browser answers
        // the pending pick request, if any, then the updater overlay.
        let ctx = ui.ctx().clone();
        if let Ok(mut browser) = self.browser.try_lock() {
            browser.show(&ctx);
        }
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
    }

    fn on_exit(&mut self) {
        if let Err(e) = self.prefs.save(&self.inner) {
            log::error!("{e}");
        }
        if let Err(e) = self.inner.session.close_library() {
            log::error!("saving the library failed: {e}");
        }
        // last, with everything saved: no render may be inside the GPU driver when the process ends
        if !self.inner.shutdown(EXIT_WAIT) {
            log::warn!("quit: background work was still running after {EXIT_WAIT:?}");
        }
    }
}

/// How long quitting waits for renders that are running (mirrors the
/// desktop's issue #620 guard).
const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Writes `ui.json` (the saved UI state and app settings) when it changed,
/// mirroring the desktop's `PrefsWriter`: only changes are written, and a
/// damaged file that was set aside at launch is never overwritten.
pub struct PrefsWriter {
    path: Option<PathBuf>,
    written: Vec<u8>,
    library: String,
    checked: f64,
    failing: bool,
    /// `ui.json` couldn't be read at launch: never overwrite it this session.
    keep_file: bool,
}

impl PrefsWriter {
    pub fn new(path: Option<PathBuf>, written: Vec<u8>, library: String, keep_file: bool) -> Self {
        PrefsWriter { path, written, library, checked: 0.0, failing: false, keep_file }
    }

    fn save(&mut self, app: &LightcraftApp) -> Result<(), String> {
        if self.keep_file || app.library_problem.is_some() {
            return Ok(());
        }
        let Some(path) = self.path.clone() else { return Ok(()) };
        let bytes = serde_json::to_vec_pretty(&app.ui).map_err(|e| e.to_string())?;
        if bytes == self.written {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("saving the app settings failed: {e}"))?;
        }
        write_atomic(&path, &bytes).map_err(|e| format!("saving the app settings failed: {e}"))?;
        self.written = bytes;
        self.library = app.ui.settings.library_path.clone();
        Ok(())
    }

    fn tick(&mut self, app: &mut LightcraftApp, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let moved = app.ui.settings.library_path != self.library;
        if !moved && now - self.checked < 3.0 {
            return;
        }
        self.checked = now;
        match self.save(app) {
            Ok(()) => self.failing = false,
            Err(e) => {
                if !self.failing {
                    log::warn!("{e}");
                    app.notices.push(format!("{e}. LightCraft keeps trying."));
                }
                self.failing = true;
                // don't retry every frame
                self.library = app.ui.settings.library_path.clone();
            }
        }
    }
}

/// Write `bytes` to `path` atomically (temp file + rename), like the engine's
/// own writer: a crash or a full disk leaves the old file or the new one,
/// never a truncated one.
fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read the saved UI state and app settings (`<config>/ui.json`), if any,
/// and a warning for the user when the file exists but can't be used: a
/// damaged file is kept as `ui.json.corrupt-<unix time>` first, so the next
/// save can't lose the library location in it; one that can't be read at all
/// is not written this session (`keep_file`).
pub fn load_prefs(path: &std::path::Path) -> (Option<UiState>, Option<String>, bool) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None, false),
        Err(e) => {
            let msg = format!(
                "The app settings ({}) couldn't be read: {e}. Defaults are used, and the file isn't overwritten.",
                path.display()
            );
            return (None, Some(msg), true);
        }
    };
    match serde_json::from_slice::<UiState>(&bytes) {
        Ok(ui) => (Some(ui.sanitized()), None, false),
        Err(e) => {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let keep = path.with_file_name(format!("ui.json.corrupt-{secs}"));
            let kept = std::fs::rename(path, &keep);
            let what = match &kept {
                Ok(()) => format!("it was kept as {}", keep.display()),
                Err(r) => format!("it couldn't be set aside ({r})"),
            };
            let msg = format!(
                "The app settings ({}) are damaged ({e}); {what}. Defaults are used.",
                path.display()
            );
            (None, Some(msg), kept.is_err())
        }
    }
}

/// First-run screen shown when "All files access" isn't granted yet.
/// Either path resolves: grant (then restart into shared-storage mode) or a
/// deliberate, persisted choice of internal storage.
pub struct StorageGate {
    /// `<internal>/storage-choice.json`; `{"mode":"internal"}` stops the nag.
    choice_file: PathBuf,
    stage: GateStage,
    error: Option<String>,
}

enum GateStage {
    /// Explain shared storage: [Grant access] [Use internal storage].
    Ask,
    /// System settings were opened: [I've granted it — continue] [Use internal storage].
    WaitingGrant,
    /// Grant confirmed: [Restart now].
    Restart,
}

impl StorageGate {
    pub fn new(choice_file: PathBuf) -> Self {
        StorageGate { choice_file, stage: GateStage::Ask, error: None }
    }

    fn choose_internal(&self) {
        // Best effort: if the write fails the gate simply shows again next
        // launch, which is the safe direction.
        let _ = std::fs::write(&self.choice_file, "{\"mode\":\"internal\"}");
    }

    /// Draw the gate. Returns true once the user has resolved it.
    fn draw(&mut self, ui: &mut egui::Ui) -> bool {
        let mut resolved = false;
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.heading("Where should LightCraft keep your work?");
            ui.add_space(12.0);
            match self.stage {
                GateStage::Ask => {
                    ui.label(
                        "LightCraft and its sibling craft apps share two folders on your tablet:\n\
                         /ArtCraft/Media for your photos, and /ArtCraft/Settings for \
                         preferences and presets.\n\n\
                         Grant \"All files access\" so the app can use them.",
                    );
                    ui.add_space(16.0);
                    if ui.button("Grant access").clicked() {
                        match craftapps_shared::jni_bridge::open_all_files_settings() {
                            Ok(()) => self.stage = GateStage::WaitingGrant,
                            Err(e) => self.error = Some(e),
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Use internal storage instead").clicked() {
                        self.choose_internal();
                        resolved = true;
                    }
                    ui.add_space(8.0);
                    ui.label(
                        "Internal storage keeps everything inside the app's private folder. \
                         It works, but the other craft apps can't see your files, and you \
                         can switch later in Settings › Storage.",
                    );
                }
                GateStage::WaitingGrant => {
                    ui.label(
                        "The system settings should now be open. Turn on \"Allow all files \
                         access\" for LightCraft, then come back here.",
                    );
                    ui.add_space(16.0);
                    if ui.button("I've granted it — continue").clicked() {
                        match craftapps_shared::jni_bridge::is_external_storage_manager() {
                            Ok(true) => self.stage = GateStage::Restart,
                            Ok(false) => {
                                self.error = Some(
                                    "Still not granted — flip the switch in the system \
                                     settings, then try again."
                                        .to_string(),
                                )
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("Use internal storage instead").clicked() {
                        self.choose_internal();
                        resolved = true;
                    }
                }
                GateStage::Restart => {
                    ui.label("Shared storage is on. Restart LightCraft once to switch to the shared folders.");
                    ui.add_space(16.0);
                    if ui.button("Restart now").clicked() {
                        // Never returns; the system restarts the activity.
                        let _ = craftapps_shared::jni_bridge::restart_process();
                    }
                }
            }
            if let Some(e) = self.error.as_deref() {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
            }
        });
        resolved
    }
}
