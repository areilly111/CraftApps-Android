//! The eframe shell: the shared [`EffectcraftApp`] plus the Android-only UI
//! (the file-browser dialogs, the updater, and the first-run storage
//! permission gate) drawn on top of it.
//!
//! Upstream crates are not modified for the port beyond the small Android-only
//! hooks (`AndroidHooks`, the Help menu item, the Storage settings page, and
//! the accesskit feature gate); everything Android-specific lives in this
//! crate and is composed here.
//!
//! File I/O design: EffectCraft has no `Services` struct — the engine owns
//! all file access through [`Session::execute`] commands (`file.open`,
//! `file.import`, `file.saveAs`, …) on plain filesystem paths. The port
//! wires the app's [`Hooks`](effectcraft_ui_egui::Hooks) file pickers to the
//! in-app browser ([`FileBrowser`]): a hook opens the browser and reports
//! "cancelled" for that invocation (the hooks are synchronous, so they
//! cannot block), and the shell reruns the matching engine command with the
//! picked paths once the browser answers. Save dialogs and folder picks get
//! immediate default paths under the app's documents folder, so every flow
//! stays functional. `content://` URIs (the system picker's choices, or the
//! SD-card location) are staged into the cache as plain files the engine can
//! read.

use crate::file_browser::FileBrowser;
use craftapps_shared::updater::Updater;
use eframe::egui;
use effectcraft_ui_egui::EffectcraftApp;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What to rerun once the file browser answers. The hook that opened the
/// browser records this; the shell drains both together.
#[derive(Clone, Copy)]
pub enum PendingPick {
    /// File ▸ Open Project…: `file.open {"path"}`.
    OpenProject,
    /// File ▸ Import…: `file.import {"paths", "background": true}`.
    ImportFiles,
    /// A save dialog: rerun `command` with the picked path under `key`
    /// (`"path"`, or `"folder"` for the folder-style saves).
    SaveAs { command: &'static str, key: &'static str },
}

/// Rerun the command that asked for the file dialog, now that the browser
/// answered. An empty answer is a cancellation: nothing runs.
fn rerun(app: &mut EffectcraftApp, pick: PendingPick, answer: &[String]) {
    let result = match pick {
        PendingPick::OpenProject => match answer.first() {
            Some(path) => app.session.execute("file.open", json!({"path": path})),
            None => return,
        },
        PendingPick::ImportFiles => {
            if answer.is_empty() {
                return;
            }
            // Picked files import in the background, with the Importing card
            // showing progress (the desktop's `ask` does the same).
            app.session.execute("file.import", json!({"paths": answer, "background": true}))
        }
        PendingPick::SaveAs { command, key } => match answer.first() {
            Some(path) => {
                let mut params = serde_json::Map::new();
                params.insert(key.to_string(), Value::String(path.clone()));
                app.session.execute(command, Value::Object(params))
            }
            None => return,
        },
    };
    if let Err(e) = result {
        app.ui.status = e.to_string();
    }
}

pub struct AndroidShell {
    inner: EffectcraftApp,
    browser: Arc<Mutex<FileBrowser>>,
    /// Set by the hook that opened the browser; cleared when its answer runs.
    pending: Arc<Mutex<Option<PendingPick>>>,
    updater: Arc<Mutex<Updater>>,
    /// First-run shared-storage permission screen (`None` = resolved).
    gate: Option<StorageGate>,
    /// Where the UI state (dock layout, panel state) is persisted.
    ui_state: PathBuf,
}

impl AndroidShell {
    pub fn new(
        app: EffectcraftApp,
        browser: Arc<Mutex<FileBrowser>>,
        pending: Arc<Mutex<Option<PendingPick>>>,
        updater: Arc<Mutex<Updater>>,
        gate: Option<StorageGate>,
        ui_state: PathBuf,
    ) -> Self {
        AndroidShell { inner: app, browser, pending, updater, gate, ui_state }
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
        // Drawn after the app, so they sit on top: the file browser, then the
        // updater overlay.
        let ctx = ui.ctx().clone();
        if let Ok(mut browser) = self.browser.lock() {
            browser.show(&ctx);
        }
        // The browser answered: rerun whatever asked for the file dialog.
        let answer = self.browser.lock().ok().and_then(|b| b.take_answer());
        if let Some(answer) = answer
            && let Some(pick) = self.pending.lock().ok().and_then(|mut p| p.take())
        {
            rerun(&mut self.inner, pick, &answer);
        }
        let mut updater = self.updater.lock().unwrap_or_else(|e| e.into_inner());
        updater.poll();
        updater.show(&ctx);
    }

    fn on_exit(&mut self) {
        // The session saves prefs on change; flush once more on the way out.
        self.inner.session.save_prefs();
        save_ui_state(&self.inner, &self.ui_state);
    }
}

/// Persist the UI state (dock layout, panel state), best effort: a failed
/// write keeps the previous file.
fn save_ui_state(app: &EffectcraftApp, ui_file: &Path) {
    if let Some(parent) = ui_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&app.ui) {
        // Crash-safe: temp file beside the target, then rename.
        let tmp = ui_file.with_extension("json.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, ui_file);
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
                ui.heading("Where should EffectCraft keep your work?");
                ui.add_space(12.0);
                match self.stage {
                    GateStage::Ask => {
                        ui.label(
                            "EffectCraft and its sibling craft apps share two folders on your tablet:\n\
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
                             can switch later in Settings ▸ Storage.",
                        );
                    }
                    GateStage::WaitingGrant => {
                        ui.label(
                            "The system settings should now be open. Turn on \"Allow all files \
                             access\" for EffectCraft, then come back here.",
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
                            "Shared storage is on. Restart EffectCraft once to switch to the \
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
