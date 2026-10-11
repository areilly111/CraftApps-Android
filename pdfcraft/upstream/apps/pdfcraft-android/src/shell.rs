//! The eframe shell: the shared [`PdfCraftApp`] plus the Android-only UI (the file-browser
//! dialogs, the updater, the storage preferences overlay, and the first-run storage
//! permission gate) drawn on top of it.
//!
//! Upstream crates are not modified for the port beyond the small Android-only hooks (the
//! `android_*` fields on [`PdfCraftApp`][pdfcraft_ui_egui::PdfCraftApp], the Help ▸ Check for
//! updates hook, and the rfd/arboard compile gates); everything Android-specific lives in
//! this crate and is composed here.
//!
//! File I/O design (PdfCraft has no `Services` struct to map, unlike the sibling ports):
//! the app's file dialogs queue an [`AndroidPickRequest`][pdfcraft_ui_egui::AndroidPickRequest]
//! instead of showing an rfd panel; the [`FileBrowser`] shows the equivalent and its answer
//! runs the pick's own follow-up closure, so every flow (Open, Combine, Insert, Save As,
//! exports, …) works unchanged. Answers are plain filesystem paths, except a SAF "save
//! elsewhere" choice, which arrives as a `content://` URI string that
//! [`PdfCraftApp::write_picked_file`][pdfcraft_ui_egui::PdfCraftApp::write_picked_file]
//! routes through the shell's SAF writer.

use crate::file_browser::{FileBrowser, SafAnswers};
use craftapps_shared::saf;
use craftapps_shared::storage::StorageManager;
use craftapps_shared::updater::Updater;
use eframe::egui;
use pdfcraft_ui_egui::PdfCraftApp;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct AndroidShell {
    inner: PdfCraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    /// SAF answers queued by the result handler, drained on the UI thread.
    saf_answers: SafAnswers,
    updater: Arc<Mutex<Updater>>,
    storage: Arc<StorageManager>,
    granted: bool,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
    /// Where `app.ron` (UI state + preferences) lives.
    prefs_file: PathBuf,
    /// The Storage preferences overlay.
    storage_ui: StorageUi,
}

impl AndroidShell {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: PdfCraftApp,
        browser: Arc<Mutex<FileBrowser>>,
        saf_answers: SafAnswers,
        updater: Arc<Mutex<Updater>>,
        storage: Arc<StorageManager>,
        granted: bool,
        gate: Option<StorageGate>,
        prefs_file: PathBuf,
    ) -> Self {
        AndroidShell {
            inner: app,
            browser,
            saf_answers,
            updater,
            storage,
            granted,
            gate,
            prefs_file,
            storage_ui: StorageUi::default(),
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
        let ctx = ui.ctx().clone();
        // Feed the browser the app's oldest queued pick, if it isn't showing one.
        let mut browser = self.browser.lock().unwrap_or_else(|e| e.into_inner());
        if !browser.has_pending()
            && let Some(request) = self.inner.android_pick_request()
        {
            browser.push(request);
        }
        // The browser's answer runs the pick's follow-up (empty = cancelled).
        if let Some(answer) = browser.show(&ctx) {
            drop(browser);
            self.inner
                .answer_android_pick(answer.into_iter().map(PathBuf::from).collect());
        } else {
            drop(browser);
        }
        // SAF answers (system picker) arrive on the JVM thread; deliver them here.
        let saf_answers: Vec<Vec<String>> = self
            .saf_answers
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        for answer in saf_answers {
            self.inner
                .answer_android_pick(answer.into_iter().map(PathBuf::from).collect());
        }
        // Drawn after the app, so they sit on top: the updater overlay, then the
        // storage preferences overlay.
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
        drop(updater);
        self.storage_ui.show(&ctx, &self.storage, self.granted);
    }

    fn on_exit(&mut self) {
        save_prefs(&self.inner, &self.prefs_file);
    }
}

/// Persist the UI state and preferences (`app.ron`), best effort: a failed write keeps the
/// previous file (the desktop's eframe persistence).
fn save_prefs(app: &PdfCraftApp, prefs_file: &Path) {
    if let Some(parent) = prefs_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(prefs_file, app.persist());
}

/// Read the saved UI state and preferences (`app.ron`), like the desktop's eframe restore.
pub fn load_prefs(app: &mut PdfCraftApp, prefs_file: &Path) {
    if let Ok(json) = std::fs::read_to_string(prefs_file) {
        app.restore(&json);
    }
}

/// The Storage preferences overlay: a small floating button opening the shared-folders
/// settings (PdfCraft's own Preferences dialog has no Storage section on any platform).
#[derive(Default)]
struct StorageUi {
    open: bool,
    error: Option<String>,
}

impl StorageUi {
    fn show(&mut self, ctx: &egui::Context, storage: &Arc<StorageManager>, granted: bool) {
        egui::Area::new(egui::Id::new("android-storage-fab"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
            .show(ctx, |ui| {
                if ui.small_button("🗂 Storage").clicked() {
                    self.open = !self.open;
                    self.error = None;
                }
            });
        if !self.open {
            return;
        }
        let mut close = false;
        egui::Window::new("Storage")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -48.0])
            .show(ctx, |ui| {
                ui.label("PdfCraft and its sibling craft apps share two folders on your tablet:");
                ui.monospace("/ArtCraft/Media — your documents\n/ArtCraft/Settings — preferences and recovery data");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Location:");
                    ui.label(if granted {
                        storage.location_label()
                    } else {
                        "Internal app storage (shared folders not enabled)".to_string()
                    });
                });
                ui.add_space(8.0);
                if granted {
                    if ui.button("Use tablet storage").clicked() {
                        storage.use_tablet_storage();
                    }
                    if ui.button("Choose SD-card folder…").clicked() {
                        // The folder picker needs no permission: the chosen tree is
                        // read and written through SAF directly.
                        saf::set_pending_tree(storage.clone());
                        if let Err(e) = saf::launch_open_document_tree() {
                            log::error!("couldn't open the folder picker: {e}");
                            saf::cancel_pending();
                            self.error = Some(e);
                        }
                    }
                } else if ui.button("Grant all-files access").clicked() {
                    match craftapps_shared::jni_bridge::open_all_files_settings() {
                        Ok(()) => {
                            self.error = Some(
                                "Turn on \"Allow all files access\" for PdfCraft in the system settings, then restart the app.".to_string(),
                            );
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                if let Some(e) = self.error.as_deref() {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
                }
                ui.add_space(8.0);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            self.open = false;
            self.error = None;
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
                ui.heading("Where should PdfCraft keep your work?");
                ui.add_space(12.0);
                match self.stage {
                    GateStage::Ask => {
                        ui.label(
                            "PdfCraft and its sibling craft apps share two folders on your tablet:\n\
                             /ArtCraft/Media for your documents, and /ArtCraft/Settings for \
                             preferences and recovery data.\n\n\
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
                             can switch later from the Storage button.",
                        );
                    }
                    GateStage::WaitingGrant => {
                        ui.label(
                            "The system settings should now be open. Turn on \"Allow all files \
                             access\" for PdfCraft, then come back here.",
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
                            "Shared storage is on. Restart PdfCraft once to switch to the \
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
