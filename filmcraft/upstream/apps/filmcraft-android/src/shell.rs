//! The eframe shell: the shared [`FilmcraftApp`] plus the Android-only UI
//! (the file-browser dialogs, the updater, and the first-run storage
//! permission gate) drawn on top of it.
//!
//! Upstream crates are not modified for the port beyond the small Android-only
//! hooks (`AndroidHooks`, the Help menu item, the Storage preferences
//! section); everything Android-specific lives in this crate and is composed
//! here.

use crate::audio::AndroidAudioOut;
use crate::file_browser::{FileBrowser, ResolvedPick};
use craftapps_shared::updater::Updater;
use eframe::egui;
use filmcraft_ui_egui::FilmcraftApp;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AndroidShell {
    inner: FilmcraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    updater: Arc<Mutex<Updater>>,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
}

impl AndroidShell {
    pub fn new(app: FilmcraftApp, browser: Arc<Mutex<FileBrowser>>, updater: Arc<Mutex<Updater>>, gate: Option<StorageGate>) -> Self {
        AndroidShell { inner: app, browser, updater, gate }
    }

    /// Run the engine command for a resolved browser pick (the web app's
    /// `picked` → `take_files` pattern in `apps/filmcraft-web/src/import.rs`).
    fn apply_pick(&mut self, pick: ResolvedPick) {
        match pick {
            ResolvedPick::Import(paths) => {
                let (projects, media): (Vec<String>, Vec<String>) =
                    paths.into_iter().partition(|p| p.to_ascii_lowercase().ends_with(".fcproj"));
                for p in projects {
                    if let Err(e) = self.inner.session.execute("file.open", json!({"path": p})) {
                        self.inner.status(e.to_string());
                    }
                }
                if !media.is_empty() {
                    let bin = self.inner.import_bin().0;
                    match self.inner.session.execute("file.import", json!({"paths": media, "bin": bin})) {
                        Ok(v) => {
                            if let Some(errs) = v.get("errors").and_then(|e| e.as_array())
                                && !errs.is_empty()
                            {
                                self.inner.status(
                                    errs.iter().filter_map(|e| e.as_str()).collect::<Vec<_>>().join("; "),
                                );
                            }
                        }
                        Err(e) => self.inner.status(e.to_string()),
                    }
                }
            }
            ResolvedPick::OpenProject(path) => {
                if let Err(e) = self.inner.session.execute("file.open", json!({"path": path})) {
                    self.inner.status(e.to_string());
                }
            }
            ResolvedPick::Relink { path, hint } => {
                let mut params = hint.params;
                if !params.is_object() {
                    params = json!({});
                }
                params["path"] = json!(path);
                match self.inner.session.execute(&hint.command, params).map_err(|e| e.to_string()) {
                    Ok(v) => {
                        if let Some(n) = v.get("relinked").and_then(|v| v.as_array()).map(Vec::len) {
                            self.inner.status(format!("Linked {n} clip(s)"));
                        }
                    }
                    Err(e) => self.inner.status(e),
                }
            }
        }
    }
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // The storage gate blocks the whole app until the user grants shared
        // storage or explicitly chooses internal storage. Nothing behind it is
        // reachable, so there is no dead end.
        if let Some(gate) = self.gate.as_mut() {
            if gate.draw(ui) {
                self.gate = None;
            }
            return;
        }
        self.inner.ui(ui, frame);
        // Drawn after the app, so they sit on top: the file browser answers
        // the pending pick, if any, then the updater overlay.
        let ctx = ui.ctx().clone();
        let mut resolved = None;
        if let Ok(mut browser) = self.browser.try_lock() {
            browser.show(&ctx);
            resolved = browser.take_resolved();
        }
        if let Some(pick) = resolved {
            self.apply_pick(pick);
        }
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
    }
}

/// Attach the default audio output, best effort: with no device, playback
/// runs silent (the audio layer reports it) instead of failing to start.
pub fn attach_audio(app: &mut FilmcraftApp) {
    app.audio = Some(Box::new(AndroidAudioOut::new()));
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
                ui.heading("Where should FilmCraft keep your work?");
                ui.add_space(12.0);
                match self.stage {
                    GateStage::Ask => {
                        ui.label(
                            "FilmCraft and its sibling craft apps share two folders on your tablet:\n\
                             /ArtCraft/Media for your documents, and /ArtCraft/Settings for \
                             preferences, presets and recovery data.\n\n\
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
                             can switch later in Preferences › Storage.",
                        );
                    }
                    GateStage::WaitingGrant => {
                        ui.label(
                            "The system settings should now be open. Turn on \"Allow all files \
                             access\" for FilmCraft, then come back here.",
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
                        ui.label(
                            "Shared storage is on. Restart FilmCraft once to switch to the \
                             shared folders.",
                        );
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
