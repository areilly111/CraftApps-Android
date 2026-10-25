//! Android shell wrapper for `PdfCraftApp`.
//!
//! Wraps the portable `PdfCraftApp` (an `eframe::App`) to dispatch stylus
//! pen actions each frame. The Java side forwards pen button presses through
//! the `nativeOnPenEvent` side-channel; the shared [`pen`](craftapps_shared::pen)
//! module translates them into [`PenAction`](craftapps_shared::pen::PenAction)s
//! which are drained here.
//!
//! The `RightClick` action synthesizes a real egui Secondary-button
//! press/release at the current pointer position, so stylus barrel buttons
//! open context menus exactly like a physical right-click. Other actions
//! (Undo, Redo, eraser, eyedropper — image-editing tools) have no PDF
//! equivalent and are logged.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craftapps_shared::pen;

/// eframe app wrapper that dispatches pen actions around `PdfCraftApp`.
pub struct PdfCraftShell {
    inner: pdfcraft_ui_egui::PdfCraftApp,
    /// Multi-finger tap recognition (the "stylus mode" gestures).
    gestures: craftapps_shared::gestures::GestureDetector,
    /// Which modifier keys are held (winit's Android backend sends no
    /// `ModifiersChanged`, so they are tracked from the key events).
    modifiers: craftapps_shared::modifiers::ModifierKeys,
}

impl PdfCraftShell {
    /// Create the shell around a fresh `PdfCraftApp`.
    pub fn new(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut inner = pdfcraft_ui_egui::PdfCraftApp::new();
        if let Some(json) = storage.and_then(|s| s.get_string("pdfcraft")) {
            inner.restore(&json);
        }
        Self { inner, gestures: craftapps_shared::gestures::GestureDetector::new(), modifiers: craftapps_shared::modifiers::ModifierKeys::default() }
    }

    /// Multi-finger taps (the "stylus mode" gestures) and the keyboard
    /// modifiers. Runs before the app's frame, so the app sees the
    /// modifiers and the gesture's side effects in the same frame.
    fn process_input(&mut self, ctx: &eframe::egui::Context) {
        // Pen input pump: wakes the app on pen-button transitions so a
        // hover press dispatches immediately instead of waiting for the
        // next touch to run a frame (the PhotoCraft 0.4.5 fix).
        craftapps_shared::pen::install_pen_pump(ctx.clone());
        // Keyboard modifiers: winit's Android backend never sends
        // `ModifiersChanged`, so egui's modifier state is empty and every
        // Ctrl/Shift/Alt shortcut is dead without this.
        self.modifiers.update(ctx);
        self.modifiers.publish(ctx);

        // The file bridge's answer to a system picker, if one came back.
        craftapps_shared::saf::poll_bridge();
        crate::install_document_waker(ctx);
        if let Some(event) = crate::incoming::poll(&craftapps_shared::picker_bridge::bridge_dir()) {
            match event {
                Ok(path) => self.inner.open_path(&path),
                Err(error) => self.inner.notify_fmt("Couldn't open document from another app: {e}", &[("e", &error)]),
            }
            // Drain additional completed imports without an idle repaint timer.
            ctx.request_repaint();
        }

        // The stylus data of the last motion event: the tool type (stylus,
        // eraser end, palm) and the side-button state. winit drops both, so
        // the vendored android-activity records them on the way past.
        self.feed_stylus_state();

        if let Some(tap) = self.gestures.update(ctx) {
            // The shared runtime maps the finger count to the configured
            // action (and ignores the gesture when stylus mode is off).
            let ppp = ctx.pixels_per_point();
            pen::on_gesture(tap.fingers as i32, tap.pos.x * ppp, tap.pos.y * ppp);
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

        // Stylus states via the shared pen pump when it is running (it
        // notes them for the diagnostics itself); otherwise drain the
        // Android queue directly, as before.
        let states = if craftapps_shared::pen::pen_pump_active() {
            craftapps_shared::pen::take_queued_states()
        } else {
            let raw = android_activity::input::drain_stylus_states();
            for state in &raw {
                craftapps_shared::pen::note_queue_state(state.action, state.buttons, state.action_button as i32, state.tool_type);
            }
            raw
        };
        for state in states {
            // A finger's touch is not the pen's business: it must not clear
            // the pen's state (the pen may well still be down).
            if (state.tool_type == TOOL_FINGER || state.tool_type == TOOL_MOUSE) && state.buttons & STYLUS_BUTTONS == 0 {
                continue;
            }
            // The pen runtime speaks the Java protocol: the Android action
            // codes (0 down, 1 up, 2 move, 3 cancel) and the `BUTTON_*` bits.
            pen::on_pen_event(state.action, state.tool_type, state.buttons as i32, state.action_button as i32, state.x, state.y);
        }
    }

    /// Drain queued pen actions and dispatch them. Takes `&egui::Context`
    /// so `RightClick` can synthesize pointer events.
    fn dispatch_pen_actions(&mut self, ctx: &eframe::egui::Context) {
        for qa in pen::drain_actions() {
            use pen::PenAction;
            match (qa.action, qa.pressed) {
                (PenAction::None, _) => {}
                (PenAction::RightClick, true) => {
                    // Synthesize a right-click at the pen event position.
                    // The Java side passes MotionEvent coordinates, which work
                    // for hover button presses where egui has no pointer pos.
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
                // Release events for one-shot actions are ignored.
                (PenAction::RightClick, false) => {}
                // Image-editing actions with no PDF equivalent.
                (action, pressed) => {
                    log::debug!("pen action ignored on Android PDF shell: {action:?} pressed={pressed}");
                }
            }
        }
    }
}

impl Default for PdfCraftShell {
    fn default() -> Self {
        Self::new(None)
    }
}

impl eframe::App for PdfCraftShell {
    fn logic(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        // Modifiers, stylus side-channel and gestures first.
        self.process_input(ctx);
        self.dispatch_pen_actions(ctx);
        self.inner.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.inner.ui(ui, frame);
    }

    fn raw_input_hook(&mut self, ctx: &eframe::egui::Context, raw: &mut eframe::egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
    }
}
