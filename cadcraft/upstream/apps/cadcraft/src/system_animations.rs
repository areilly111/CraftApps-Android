//! Whether the operating system shows animations, for View ▸ Accessibility ▸ Animation Effects ▸
//! Use System Setting.
//!
//! egui animates whatever the OS says (windows and menus fade in, wheel scrolling glides), and
//! winit reports no such setting. This asks the OS itself:
//!
//! - Windows: Settings ▸ Accessibility ▸ Visual effects ▸ Animation effects.
//! - macOS: System Settings ▸ Accessibility ▸ Reduce motion.
//! - Linux and the BSDs: the XDG Desktop Portal's Settings interface, as for the theme
//!   (`desktop_theme.rs`): its `reduced-motion`, or GNOME's own `enable-animations`.
//!
//! The answer goes to `CadApp::system_animations` once a frame.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

/// How often the OS is asked again, so a change to its setting is followed while CADCraft runs.
const POLL: Duration = Duration::from_secs(3);

/// Watches the OS setting.
#[derive(Default)]
pub struct SystemAnimations {
    /// The watcher thread's answers, where asking can block (the portal); `None` elsewhere, or
    /// when no thread could be started.
    changes: Option<Receiver<Option<bool>>>,
    /// The OS's last answer.
    current: Option<bool>,
    /// When the OS was last asked from the UI thread, where asking is one quick call.
    asked: Option<Instant>,
}

impl SystemAnimations {
    /// Start watching. Where a thread does the asking, `wake` is called after each answer, so an
    /// idle window repaints and picks it up.
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        if !ASKS_THE_PORTAL {
            return Self::default();
        }
        let (tx, rx) = channel();
        let started = std::thread::Builder::new().name("cadcraft-system-animations".into()).spawn(move || {
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
        Self { changes: started.ok().map(|_| rx), ..Self::default() }
    }

    /// Whether the OS shows animations; `None` when it states nothing or there is nobody to ask.
    /// Polled once a frame; an OS that stops answering keeps its last answer.
    pub fn current(&mut self) -> Option<bool> {
        if let Some(changes) = &self.changes {
            loop {
                match changes.try_recv() {
                    Ok(Some(animate)) => self.current = Some(animate),
                    Ok(None) => {}
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
                }
            }
        } else if !ASKS_THE_PORTAL && self.asked.is_none_or(|at| at.elapsed() >= POLL) {
            self.asked = Some(Instant::now());
            if let Some(animate) = read() {
                self.current = Some(animate);
            }
        }
        self.current
    }
}

/// Where the answer comes over D-Bus, which can block: a thread asks. Elsewhere the UI thread asks.
const ASKS_THE_PORTAL: bool = cfg!(all(unix, not(target_os = "macos")));

/// Ask Windows once: `UISettings::AnimationsEnabled`, the setting behind
/// `SPI_GETCLIENTAREAANIMATION`.
#[cfg(windows)]
fn read() -> Option<bool> {
    windows::UI::ViewManagement::UISettings::new().and_then(|settings| settings.AnimationsEnabled()).ok()
}

/// Ask macOS once. Called on the main thread (`SystemAnimations::current`).
#[cfg(target_os = "macos")]
fn read() -> Option<bool> {
    Some(!objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

/// Ask the portal once. D-Bus calls block, so this belongs off the UI thread: the watcher's
/// thread is its only caller.
#[cfg(all(unix, not(target_os = "macos")))]
fn read() -> Option<bool> {
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    // The standard key first (0 no preference, 1 reduced motion); then GNOME's own switch, which
    // its portal answered before the standard key existed.
    const KEYS: [(&str, &str); 2] = [("org.freedesktop.appearance", "reduced-motion"), ("org.gnome.desktop.interface", "enable-animations")];
    let connection = zbus::blocking::Connection::session().ok()?;
    for (namespace, key) in KEYS {
        // `ReadOne` (Settings version 2) answers the value; older portals only have `Read`, which
        // wraps it in one more variant. `parse` unwraps either.
        let reply = connection
            .call_method(Some(PORTAL), PATH, Some(SETTINGS), "ReadOne", &(namespace, key))
            .or_else(|_| connection.call_method(Some(PORTAL), PATH, Some(SETTINGS), "Read", &(namespace, key)));
        let Ok(reply) = reply else { continue };
        let Ok((answer,)) = reply.body().deserialize::<(zbus::zvariant::OwnedValue,)>() else { continue };
        if let Some(animate) = parse(&answer, 0) {
            return Some(animate);
        }
    }
    None
}

/// The portal's answer as "show animations": `reduced-motion` is a number (1 asks for less
/// motion; 0 and values from a later version state no preference), `enable-animations` a
/// boolean. `None` when it reads as neither. Variants are unwrapped a few levels deep.
#[cfg(all(unix, not(target_os = "macos")))]
fn parse(value: &zbus::zvariant::Value<'_>, depth: u8) -> Option<bool> {
    use zbus::zvariant::Value;
    match value {
        Value::U32(reduced) => Some(*reduced != 1),
        Value::Bool(enabled) => Some(*enabled),
        Value::Value(inner) if depth < 4 => parse(inner, depth + 1),
        _ => None,
    }
}

#[cfg(not(any(unix, windows)))]
fn read() -> Option<bool> {
    None
}

// macOS is asked on the main thread only, so there is nothing to test off it.
#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    /// Windows is asked on the first frame, then not again until the poll interval has passed.
    #[test]
    #[cfg(windows)]
    fn the_os_is_asked_on_the_first_frame_and_its_answer_kept() {
        let mut watch = SystemAnimations::start(|| {});
        assert!(watch.changes.is_none(), "no thread: asking is one quick call");
        let first = watch.current();
        assert_eq!(first, read());
        let asked = watch.asked;
        assert!(asked.is_some());
        assert_eq!(watch.current(), first);
        assert_eq!(watch.asked, asked, "not asked again within the poll interval");
    }

    /// The standard key answers a number, GNOME's switch a boolean; either may come wrapped in
    /// variants (the older `Read`).
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn the_portal_answer_reads_as_show_animations_or_as_none() {
        use zbus::zvariant::Value;
        assert_eq!(parse(&Value::U32(1), 0), Some(false));
        assert_eq!(parse(&Value::U32(0), 0), Some(true));
        assert_eq!(parse(&Value::U32(7), 0), Some(true), "unknown values are no preference");
        assert_eq!(parse(&Value::Bool(false), 0), Some(false));
        assert_eq!(parse(&Value::Bool(true), 0), Some(true));
        assert_eq!(parse(&Value::Value(Box::new(Value::U32(1))), 0), Some(false));
        assert_eq!(parse(&Value::Str("off".into()), 0), None);
        // A hostile answer nested without end is given up on.
        let mut nested = Value::U32(1);
        for _ in 0..16 {
            nested = Value::Value(Box::new(nested));
        }
        assert_eq!(parse(&nested, 0), None);
    }
}
