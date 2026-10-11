//! The eframe shell: the shared [`DesignApp`] plus the Android-only UI
//! (the file-browser dialogs, the updater, and the first-run storage
//! permission gate) drawn on top of it.
//!
//! Upstream crates are not modified for the port; everything Android-specific
//! lives in this crate and is composed here.

use crate::file_browser::FileBrowser;
use craftapps_shared::jni_bridge;
use craftapps_shared::updater::Updater;
use designcraft_ui_egui::DesignApp;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AndroidShell {
    inner: DesignApp,
    browser: Arc<Mutex<FileBrowser>>,
    updater: Arc<Mutex<Updater>>,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
    /// Where `ui.json` / `prefs.json` live (mirrors the desktop's config dir).
    config: PathBuf,
}

impl AndroidShell {
    pub fn new(
        app: DesignApp,
        browser: Arc<Mutex<FileBrowser>>,
        updater: Arc<Mutex<Updater>>,
        gate: Option<StorageGate>,
        config: PathBuf,
    ) -> Self {
        AndroidShell { inner: app, browser, updater, gate, config }
    }

    /// Persist the UI state and engine prefs, like the desktop's `save_prefs`.
    fn save_prefs(&self) {
        let dir = &self.config;
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.inner.ui) {
            let _ = std::fs::write(dir.join("ui.json"), bytes);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.inner.session.prefs) {
            let _ = std::fs::write(dir.join("prefs.json"), bytes);
        }
    }
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.inner.logic(ctx);
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
        // the pending open_async request, if any, then the updater overlay.
        let ctx = ui.ctx().clone();
        if let Ok(mut browser) = self.browser.try_lock() {
            browser.show(&ctx);
        }
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
    }

    fn on_exit(&mut self) {
        self.save_prefs();
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
            ui.heading("Where should DesignCraft keep your work?");
            ui.add_space(12.0);
            match self.stage {
                GateStage::Ask => {
                    ui.label(
                        "DesignCraft and its sibling craft apps share two folders on your tablet:\n\
                         /ArtCraft/Media for your documents, and /ArtCraft/Settings for \
                         preferences and recovery data.\n\n\
                         Grant \"All files access\" so the app can use them.",
                    );
                    ui.add_space(16.0);
                    if ui.button("Grant access").clicked() {
                        match jni_bridge::open_all_files_settings() {
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
                         access\" for DesignCraft, then come back here.",
                    );
                    ui.add_space(16.0);
                    if ui.button("I've granted it — continue").clicked() {
                        match jni_bridge::is_external_storage_manager() {
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
                    ui.label("Shared storage is on. Restart DesignCraft once to switch to the shared folders.");
                    ui.add_space(16.0);
                    if ui.button("Restart now").clicked() {
                        // Never returns; the system restarts the activity.
                        let _ = jni_bridge::restart_process();
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
