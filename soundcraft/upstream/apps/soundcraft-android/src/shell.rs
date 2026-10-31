//! eframe shell: owns the [`SoundApp`], running the desktop host's duties
//! (preference persistence, title, quit) plus the full shared Android
//! input stack — keyboard modifiers, the S Pen side-channel, stylus-mode
//! multi-finger gestures and pen button remap dispatch.
//!
//! The shell deliberately does NOT poll the shared crate's SAF pending
//! flow (`saf::poll_bridge`): SoundCraft's dialogs all go through the
//! file bridge's synchronous hooks (see `services.rs`), which own the
//! bridge response file; `poll_bridge` would take those responses and
//! drop them for want of a pending request.

use std::path::PathBuf;

use eframe::egui;
use serde_json::json;
use soundcraft_ui_egui::{SoundApp, dialogs};

use craftapps_shared::gestures::GestureDetector;
use craftapps_shared::modifiers::ModifierKeys;

/// The host's `eframe::App`: the SoundCraft UI plus the Android input stack.
pub struct ShellApp {
    app: SoundApp,
    /// Multi-finger tap recognition (the "stylus mode" gestures).
    gestures: GestureDetector,
    /// Which modifier keys are held (winit's Android backend sends no
    /// `ModifiersChanged`, so they are tracked from the key events).
    modifiers: ModifierKeys,
    /// `ui.json` in the suite settings folder (SoundCraft's config dir).
    prefs_path: PathBuf,
}

impl ShellApp {
    pub fn new(app: SoundApp, prefs_path: PathBuf) -> Self {
        Self { app, gestures: GestureDetector::new(), modifiers: ModifierKeys::default(), prefs_path }
    }

    /// Persist the UI state, as the desktop host does whenever the recent
    /// list changes and at exit (the recent list matters most after a
    /// crash). Atomic: a temp file in the same folder, renamed over.
    fn save_prefs(&self) {
        // The desktop drops the ephemeral fields before writing; do the same.
        let mut saved = self.app.ui.clone();
        saved.plugin_windows.clear();
        saved.status.clear();
        let bytes = match serde_json::to_vec_pretty(&saved) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("preferences not saved (serialize): {e}");
                return;
            }
        };
        if let Some(parent) = self.prefs_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self.prefs_path.with_extension("json.tmp");
        let written =
            std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &self.prefs_path));
        if let Err(e) = written {
            log::warn!("preferences not saved: {e}");
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
            // Unlike the paint apps there is no tap cleanup: a tap on the
            // timeline or mixer makes no undoable edit of its own.
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

    /// Shared [`craftapps_shared::pen`] actions → SoundCraft commands. Runs
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
                    if let Err(e) = self.app.run("edit.undo", json!({})) {
                        log::warn!("pen undo: {e}");
                    }
                }
                (PenAction::Redo, true) => {
                    if let Err(e) = self.app.run("edit.redo", json!({})) {
                        log::warn!("pen redo: {e}");
                    }
                }
                // SoundCraft has no tool-wheel UI and the remaining
                // remappable actions have no command equivalent here: log
                // and ignore.
                (action, pressed) => {
                    log::debug!("pen action ignored: {action:?} pressed={pressed}");
                }
            }
        }
    }

    /// Open a file by path (dropped on the window — the desktop flow;
    /// Android reports no drops, so this stays unused in practice).
    fn open_path(&mut self, p: &str) {
        let lower = p.to_ascii_lowercase();
        let r = if lower.ends_with(".scraft") {
            dialogs::run_or_ask(&mut self.app, "session.open", json!({"path": p}))
        } else if lower.ends_with(".mid") || lower.ends_with(".midi") || lower.ends_with(".smf") {
            self.app.run("file.import_midi", json!({"path": p}))
        } else {
            let at = self.app.engine.session().edit.selection.start;
            self.app.run("file.import_audio", json!({"path": p, "at": at}))
        };
        if let Err(e) = r {
            log::warn!("{p}: {e}");
        }
    }
}

/// Sends the title only when it changes, remembered in the context data
/// (as the desktop host does).
fn set_title(ctx: &egui::Context, title: String) {
    let id = egui::Id::new("window_title");
    if ctx.data(|d| d.get_temp::<String>(id)).as_deref() != Some(title.as_str()) {
        ctx.data_mut(|d| d.insert_temp(id, title.clone()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

impl eframe::App for ShellApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Modifiers, stylus and gestures first: the app must see them in
        // the frame they happen.
        self.process_input(ctx);
        self.app.guarded(|app| app.logic(ctx));

        soundcraft_ui_egui::fit_window_to_screen(ctx);
        let (inner, maximized) =
            ctx.input(|i| (i.viewport().inner_rect, i.viewport().maximized.unwrap_or(false)));
        self.app.ui.window_maximized = maximized;
        if let Some(r) = inner.filter(|_| !maximized) {
            self.app.ui.window_size = (r.size() * ctx.zoom_factor()).round().into();
        }
        self.app.ui.ui_scale = self.app.extra.ui_scale;

        // Files dropped on the window (never reported on Android; kept for
        // parity with the desktop host).
        let dropped: Vec<String> = self
            .app
            .take_dropped_files()
            .iter()
            .map(|f| f.path().to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .collect();
        for p in dropped {
            self.open_path(&p);
        }

        if self.app.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested())
            && dialogs::ask_to_save(&mut self.app, "app.quit", &json!({}))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        // Saved now rather than at exit: the recent list matters most
        // after a crash.
        if self.app.take_recent_changed() {
            self.save_prefs();
        }

        let title = format!("{} — SoundCraft", self.app.engine.session().name);
        set_title(ctx, title);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.guarded(|app| app.ui(ui));
        // Pen side-channel button actions (e.g. barrel-button right-click).
        // Runs after the app frame so dialogs/menus are settled.
        let ctx = ui.ctx().clone();
        self.dispatch_pen_actions(&ctx);
    }

    fn on_exit(&mut self) {
        self.app.close_autosave();
        self.save_prefs();
        // Release the player's plugin instances and shut plugin hosting
        // down (the desktop host does the same on the way out).
        self.app.player = None;
        soundcraft_engine::shutdown_plugin_hosts();
    }
}
