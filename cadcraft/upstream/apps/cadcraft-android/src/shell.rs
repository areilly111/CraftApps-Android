//! eframe shell: owns the [`CadApp`], running the desktop host's duties
//! (preference persistence, quit) plus the full shared Android input stack
//! — keyboard modifiers, the S Pen side-channel, stylus-mode multi-finger
//! gestures and pen button remap dispatch.
//!
//! The shell deliberately does NOT poll the shared crate's SAF pending
//! flow (`saf::poll_bridge`): CADCraft's dialogs all go through the
//! file bridge's synchronous hooks (see `services.rs`), which own the
//! bridge response file; `poll_bridge` would take those responses and
//! drop them for want of a pending request.
//!
//! Persistence differs from the desktop host in one deliberate way: the
//! desktop writes its workspace throttled while running (and on exit) and
//! keeps profile/prefs/autosave prefs in eframe `Storage`; on Android
//! there is no eframe storage, so this shell writes four plain files into
//! the suite settings folder on exit (`workspace.json` with the whole
//! `UiState`, `profile.json`, `prefs.json`, `autosave-prefs.json`). A
//! process killed without a clean exit therefore loses the last session's
//! interface state — but never drawings, which autosave protects
//! continuously while running.

use std::path::PathBuf;

use cadcraft_ui_egui::CadApp;
use eframe::egui;
use serde_json::json;

use craftapps_shared::gestures::GestureDetector;
use craftapps_shared::modifiers::ModifierKeys;

/// The host's `eframe::App`: the CADCraft UI plus the Android input stack.
pub struct ShellApp {
    app: CadApp,
    /// Multi-finger tap recognition (the "stylus mode" gestures).
    gestures: GestureDetector,
    /// Which modifier keys are held (winit's Android backend sends no
    /// `ModifiersChanged`, so they are tracked from the key events).
    modifiers: ModifierKeys,
    /// `workspace.json` in the suite settings folder.
    workspace_path: PathBuf,
    /// `profile.json` in the suite settings folder.
    profile_path: PathBuf,
    /// `prefs.json` in the suite settings folder.
    prefs_path: PathBuf,
    /// `autosave-prefs.json` in the suite settings folder.
    autosave_prefs_path: PathBuf,
}

impl ShellApp {
    pub fn new(
        app: CadApp,
        workspace_path: PathBuf,
        profile_path: PathBuf,
        prefs_path: PathBuf,
        autosave_prefs_path: PathBuf,
    ) -> Self {
        Self {
            app,
            gestures: GestureDetector::new(),
            modifiers: ModifierKeys::default(),
            workspace_path,
            profile_path,
            prefs_path,
            autosave_prefs_path,
        }
    }

    /// Persist the workspace, profile, interface prefs and autosave prefs,
    /// as the desktop host's `save` does. Atomic per file: a temp file in
    /// the same folder, renamed over.
    fn save_prefs(&self) {
        let files: [(&PathBuf, String); 4] = [
            (&self.workspace_path, serde_json::to_string_pretty(&self.app.ui).unwrap_or_default()),
            (&self.profile_path, self.app.profile_json()),
            (&self.prefs_path, self.app.prefs_json()),
            (&self.autosave_prefs_path, cadcraft_ui_egui::autosave::prefs_json()),
        ];
        for (path, text) in files {
            if text.is_empty() {
                continue;
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let tmp = path.with_extension("tmp");
            let written =
                std::fs::write(&tmp, text.as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
            if let Err(e) = written {
                log::warn!("preferences not saved to {}: {e}", path.display());
            }
        }
    }

    /// Keyboard modifiers, the stylus feed and multi-finger gestures. Runs
    /// before the app's frame, so the app sees the modifier state and the
    /// gestures in the same frame.
    fn process_input(&mut self, ctx: &egui::Context) {
        // Pen-event pump: pulls stylus states off the input queue on its
        // own cadence so a pen button press lands even when no frame is
        // running (the hover fix). Idempotent; first call starts it.
        craftapps_shared::pen::install_pen_pump(ctx.clone());

        // Keyboard modifiers: winit's Android backend never sends
        // `ModifiersChanged`, so egui's modifier state is empty and every
        // Ctrl/Shift/Alt shortcut is dead without this.
        self.modifiers.update(ctx);
        self.modifiers.publish(ctx);

        // The stylus data of the last motion event: the tool type (stylus,
        // eraser end, palm) and the side-button state. winit drops both, so
        // the vendored android-activity records them on the way past.
        self.feed_stylus_state();

        if let Some(tap) = self.gestures.update(ctx) {
            // The shared runtime maps the finger count to the configured
            // action (and ignores the gesture when stylus mode is off).
            // Unlike the paint apps there is no tap cleanup here: the
            // SoundCraft arrangement, kept until a CAD tap's side effects
            // (a placed point, a changed selection) are mapped to an undo
            // the shell can safely take back.
            let ppp = ctx.pixels_per_point();
            craftapps_shared::pen::on_gesture(
                tap.fingers as i32,
                tap.pos.x * ppp,
                tap.pos.y * ppp,
            );
        }
    }

    /// Hand the stylus data of the motion events to the shared pen runtime:
    /// the tool type (stylus / eraser end / palm) and the side button (the
    /// S Pen's barrel button).
    ///
    /// The runtime keeps the pressed state and only acts on the transitions,
    /// so the same state twice is harmless.
    fn feed_stylus_state(&mut self) {
        // Android tool types.
        const TOOL_FINGER: i32 = 1;
        const TOOL_MOUSE: i32 = 3;
        // The stylus' side buttons.
        const STYLUS_BUTTONS: u32 = (1 << 5) | (1 << 6);

        // States arrive via the shared pen pump when it is running (it
        // notes them for the diagnostics itself); otherwise drain the
        // Android queue directly, as before.
        let states = if craftapps_shared::pen::pen_pump_active() {
            craftapps_shared::pen::take_queued_states()
        } else {
            let raw = android_activity::input::drain_stylus_states();
            for state in &raw {
                craftapps_shared::pen::note_queue_state(
                    state.action,
                    state.buttons,
                    state.action_button as i32,
                    state.tool_type,
                );
            }
            raw
        };
        for state in states {
            // A finger's touch is not the pen's business: it must not clear
            // the pen's state (the pen may well still be down).
            if (state.tool_type == TOOL_FINGER || state.tool_type == TOOL_MOUSE)
                && state.buttons & STYLUS_BUTTONS == 0
            {
                continue;
            }
            // The pen runtime speaks the Java protocol: the Android action
            // codes (0 down, 1 up, 2 move, 3 cancel) and the `BUTTON_*` bits.
            craftapps_shared::pen::on_pen_event(
                state.action,
                state.tool_type,
                state.buttons as i32,
                state.action_button as i32,
                state.x,
                state.y,
            );
        }
    }

    /// Shared [`craftapps_shared::pen`] actions → CADCraft commands. Runs
    /// after the app frame so dialogs/menus are settled.
    fn dispatch_pen_actions(&mut self, ctx: &egui::Context) {
        for qa in craftapps_shared::pen::drain_actions() {
            use craftapps_shared::pen::PenAction;
            match (qa.action, qa.pressed) {
                (PenAction::None, _) => {}
                (PenAction::RightClick, true) => {
                    // Synthesize a right-click at the pen event position.
                    let ppp = ctx.pixels_per_point();
                    let pos = egui::pos2(qa.x / ppp, qa.y / ppp);
                    ctx.input_mut(|i| {
                        i.events.push(egui::Event::PointerMoved(pos));
                        i.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        });
                        i.events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed: false,
                            modifiers: egui::Modifiers::default(),
                        });
                    });
                    ctx.request_repaint();
                }
                (PenAction::Undo, true) => {
                    // `u`: the one-step undo (Edit ▸ Undo); `undo` would
                    // prompt for a count/mode instead.
                    if let Err(e) = self.app.run("u", json!({})) {
                        log::warn!("pen undo: {e}");
                    }
                }
                (PenAction::Redo, true) => {
                    if let Err(e) = self.app.run("redo", json!({})) {
                        log::warn!("pen redo: {e}");
                    }
                }
                // CADCraft has no tool-wheel UI and the remaining
                // remappable actions (eraser toggle, eyedropper, pan tool)
                // have no command equivalent here: log and ignore.
                (action, pressed) => {
                    log::debug!("pen action ignored: {action:?} pressed={pressed}");
                }
            }
        }
    }
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Modifiers, stylus and gestures first: the app must see them in
        // the frame they happen.
        self.process_input(ctx);
        self.app.logic(ctx);

        if std::mem::take(&mut self.app.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
        // Capture Tab at the command line, as the desktop host does.
        cadcraft_ui_egui::cmdline::capture_tab(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        // Pen side-channel button actions (e.g. barrel-button right-click).
        // Runs after the app frame so dialogs/menus are settled.
        let ctx = ui.ctx().clone();
        self.dispatch_pen_actions(&ctx);
    }

    fn on_exit(&mut self) {
        self.app.cancel_screenshots();
        self.app.shutdown_autosave();
        self.save_prefs();
    }
}
