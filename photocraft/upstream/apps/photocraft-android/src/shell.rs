//! The eframe shell: the shared [`PhotocraftApp`] plus the Android-only UI
//! (the file-browser dialogs, the updater, and the first-run storage
//! permission gate) drawn on top of it.
//!
//! Upstream crates are not modified for the port; everything Android-specific
//! lives in this crate and is composed here.

use crate::file_browser::FileBrowser;
use crate::updater::Updater;
use eframe::egui;
use photocraft_ui_egui::PhotocraftApp;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AndroidShell {
    inner: PhotocraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    updater: Arc<Mutex<Updater>>,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
    /// Tool to restore when a momentary pen-button action (Pan) releases.
    pen_prev_tool: Option<photocraft_ui_egui::Tool>,
}

impl AndroidShell {
    pub fn new(
        app: PhotocraftApp,
        browser: Arc<Mutex<FileBrowser>>,
        updater: Arc<Mutex<Updater>>,
        gate: Option<StorageGate>,
    ) -> Self {
        AndroidShell { inner: app, browser, updater, gate, pen_prev_tool: None }
    }

    /// Reference pen-button dispatch: shared [`craftapps_shared::pen`] actions
    /// → PhotoCraft commands/tools. Sibling ports copy this and map to their
    /// own command ids.
    fn dispatch_pen_actions(&mut self) {
        for qa in craftapps_shared::pen::drain_actions() {
            use craftapps_shared::pen::PenAction;
            match (qa.action, qa.pressed) {
                (PenAction::None, _) => {}
                (PenAction::Undo, true) => {
                    if let Err(e) = self.inner.run_command("edit.undo", serde_json::Value::Null) {
                        log::warn!("pen undo: {e}");
                    }
                }
                (PenAction::Redo, true) => {
                    if let Err(e) = self.inner.run_command("edit.redo", serde_json::Value::Null) {
                        log::warn!("pen redo: {e}");
                    }
                }
                (PenAction::ToggleEraser, true) => {
                    let cur = self.inner.ui.tool;
                    self.inner.ui.tool = if matches!(cur, photocraft_ui_egui::Tool::Eraser) {
                        photocraft_ui_egui::Tool::Brush
                    } else {
                        photocraft_ui_egui::Tool::Eraser
                    };
                }
                (PenAction::Eyedropper, true) => {
                    self.inner.ui.tool = photocraft_ui_egui::Tool::Eyedropper;
                }
                (PenAction::PanTool, true) => {
                    // Momentary: remember the current tool, switch to Hand.
                    if self.pen_prev_tool.is_none() {
                        self.pen_prev_tool = Some(self.inner.ui.tool);
                    }
                    self.inner.ui.tool = photocraft_ui_egui::Tool::Hand;
                }
                (PenAction::PanTool, false) => {
                    if let Some(prev) = self.pen_prev_tool.take() {
                        self.inner.ui.tool = prev;
                    }
                }
                (PenAction::RightClick, true) => {
                    // TODO(port): synthesize a secondary click at the pen tip.
                    // egui has no injectable right-click event on stable; the
                    // pragmatic path is a context-menu command id if the
                    // engine gains one.
                    log::info!("pen right-click pressed (not yet wired to a command)");
                }
                // Release events for one-shot actions are ignored.
                (_, false) => {}
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
        // Pen-button remap: drain the shared queue and dispatch to commands.
        // Runs after the app frame so dialogs/menus are settled.
        self.dispatch_pen_actions();
        // Drawn after the app, so they sit on top: the file browser answers
        // the pending FileDialogRequest, if any, then the updater overlay.
        let ctx = ui.ctx().clone();
        if let Ok(mut browser) = self.browser.try_lock() {
            browser.show(&ctx);
        }
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
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
                ui.heading("Where should PhotoCraft keep your work?");
                ui.add_space(12.0);
                match self.stage {
                    GateStage::Ask => {
                        ui.label(
                            "PhotoCraft and its sibling craft apps share two folders on your tablet:\n\
                             /ArtCraft/Media for your documents, and /ArtCraft/Settings for \
                             preferences, presets and recovery data.\n\n\
                             Grant \"All files access\" so the app can use them.",
                        );
                        ui.add_space(16.0);
                        if ui.button("Grant access").clicked() {
                            match crate::jni_bridge::open_all_files_settings() {
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
                             access\" for PhotoCraft, then come back here.",
                        );
                        ui.add_space(16.0);
                        if ui.button("I've granted it — continue").clicked() {
                            match crate::jni_bridge::is_external_storage_manager() {
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
                            "Shared storage is on. Restart PhotoCraft once to switch to the \
                             shared folders.",
                        );
                        ui.add_space(16.0);
                        if ui.button("Restart now").clicked() {
                            // Never returns; the system restarts the activity.
                            let _ = crate::jni_bridge::restart_process();
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
