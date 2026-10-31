//! The desktop's own light/dark choice, for View ▸ Theme ▸ System.
//!
//! winit reports no system theme on Linux and the BSDs (`RawInput::system_theme` stays `None`),
//! so the System choice fell back to dark whatever the desktop said. This asks the desktop itself:
//! the XDG Desktop Portal's Settings interface, which answers on GNOME, KDE and XFCE alike, over
//! X11 and Wayland alike. The answer is handed to egui as the system theme
//! (`App::raw_input_hook`), so the app and egui's own widgets follow it as they follow macOS and
//! Windows.
//!
//! Ported from PdfCraft (storytold/pdfcraft#881).

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

/// How often the watcher re-reads the desktop's answer. The portal also publishes a change
/// signal, but subscribing needs a connection kept alive for the life of the window; one round
/// trip every few seconds is cheap and reacts soon enough after a theme switch.
const POLL: Duration = Duration::from_secs(4);

/// Watches the desktop and reports its choice when it changes.
#[derive(Default)]
pub struct DesktopTheme {
    /// `None` when no watcher thread could be started (or on a platform where winit answers).
    changes: Option<Receiver<Option<egui::Theme>>>,
    /// The desktop's last answer.
    current: Option<egui::Theme>,
}

impl DesktopTheme {
    /// Start watching. The first answer is sent straight away; `wake` is called after each
    /// answer, so an idle window repaints and picks it up.
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        if !ASKS_THE_DESKTOP {
            return Self::default();
        }
        let (tx, rx) = channel();
        let started = std::thread::Builder::new().name("cadcraft-desktop-theme".into()).spawn(move || {
            let mut last = read();
            if tx.send(last).is_err() {
                return;
            }
            wake();
            loop {
                std::thread::sleep(POLL);
                let now = read();
                if now != last {
                    last = now;
                    if tx.send(now).is_err() {
                        return;
                    }
                    wake();
                }
            }
        });
        Self { changes: started.ok().map(|_| rx), current: None }
    }

    /// The desktop's choice, `None` when it states none or there is nobody to ask (no session
    /// bus, no portal). Polled once a frame; a desktop that stops answering keeps the last answer.
    pub fn current(&mut self) -> Option<egui::Theme> {
        if let Some(changes) = &self.changes {
            loop {
                match changes.try_recv() {
                    Ok(Some(theme)) => self.current = Some(theme),
                    Ok(None) => {}
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
        }
        self.current
    }
}

/// Where winit has no answer and the portal does.
const ASKS_THE_DESKTOP: bool = cfg!(all(unix, not(target_os = "macos")));

/// Ask the portal once. D-Bus calls block, so this belongs off the UI thread: the watcher's
/// thread is its only caller.
#[cfg(all(unix, not(target_os = "macos")))]
fn read() -> Option<egui::Theme> {
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    // The standard key first (GNOME and KDE answer it: 0 no preference, 1 dark, 2 light); then
    // GNOME's own schema, which XFCE's portal proxies without the standard key.
    const KEYS: [(&str, &str); 2] = [("org.freedesktop.appearance", "color-scheme"), ("org.gnome.desktop.interface", "color-scheme")];
    let connection = zbus::blocking::Connection::session().ok()?;
    for (namespace, key) in KEYS {
        // `ReadOne` (Settings version 2) answers the value; older portals only have `Read`, which
        // wraps it in one more variant. `parse` unwraps either.
        let reply = connection
            .call_method(Some(PORTAL), PATH, Some(SETTINGS), "ReadOne", &(namespace, key))
            .or_else(|_| connection.call_method(Some(PORTAL), PATH, Some(SETTINGS), "Read", &(namespace, key)));
        let Ok(reply) = reply else { continue };
        let Ok((answer,)) = reply.body().deserialize::<(zbus::zvariant::OwnedValue,)>() else { continue };
        if let Some(scheme) = parse(&answer, 0) {
            return scheme;
        }
    }
    None
}

/// The portal's `color-scheme` answer: `Some(choice)` when it read as an answer (`Some(None)`
/// for "no preference"), `None` when it didn't. Variants are unwrapped a few levels deep.
#[cfg(all(unix, not(target_os = "macos")))]
fn parse(value: &zbus::zvariant::Value<'_>, depth: u8) -> Option<Option<egui::Theme>> {
    use zbus::zvariant::Value;
    match value {
        Value::U32(1) => Some(Some(egui::Theme::Dark)),
        Value::U32(2) => Some(Some(egui::Theme::Light)),
        Value::U32(_) => Some(None),
        Value::Str(s) => Some(match s.as_str() {
            "prefer-dark" => Some(egui::Theme::Dark),
            "prefer-light" => Some(egui::Theme::Light),
            _ => None,
        }),
        Value::Value(inner) if depth < 4 => parse(inner, depth + 1),
        _ => None,
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn read() -> Option<egui::Theme> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The standard key answers a number, GNOME's schema a string; either may come wrapped in
    /// variants (the older `Read`). Only "dark" and "light" are a choice.
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn the_portal_answer_reads_as_a_choice_or_as_none() {
        use zbus::zvariant::Value;
        assert_eq!(parse(&Value::U32(1), 0), Some(Some(egui::Theme::Dark)));
        assert_eq!(parse(&Value::U32(2), 0), Some(Some(egui::Theme::Light)));
        assert_eq!(parse(&Value::U32(0), 0), Some(None));
        assert_eq!(parse(&Value::U32(7), 0), Some(None));
        assert_eq!(parse(&Value::from("prefer-dark"), 0), Some(Some(egui::Theme::Dark)));
        assert_eq!(parse(&Value::from("prefer-light"), 0), Some(Some(egui::Theme::Light)));
        assert_eq!(parse(&Value::from("default"), 0), Some(None));
        let wrapped = Value::Value(Box::new(Value::Value(Box::new(Value::U32(1)))));
        assert_eq!(parse(&wrapped, 0), Some(Some(egui::Theme::Dark)));
        assert_eq!(parse(&Value::Bool(true), 0), None);
        let mut deep = Value::U32(1);
        for _ in 0..10 {
            deep = Value::Value(Box::new(deep));
        }
        assert_eq!(parse(&deep, 0), None, "nesting is bounded");
    }

    /// A watcher that was never started is inert rather than a panic.
    #[test]
    fn a_watcher_without_a_thread_is_inert() {
        assert_eq!(DesktopTheme::default().current(), None);
    }

    /// Answers arrive as changes; "no preference" or silence keeps the last real answer.
    #[test]
    fn the_last_answer_stands_until_a_new_one() {
        let (tx, rx) = channel();
        let mut watcher = DesktopTheme { changes: Some(rx), current: None };
        assert_eq!(watcher.current(), None);
        tx.send(Some(egui::Theme::Light)).unwrap();
        assert_eq!(watcher.current(), Some(egui::Theme::Light));
        tx.send(Some(egui::Theme::Dark)).unwrap();
        tx.send(None).unwrap();
        assert_eq!(watcher.current(), Some(egui::Theme::Dark));
        drop(tx);
        assert_eq!(watcher.current(), Some(egui::Theme::Dark));
    }
}
