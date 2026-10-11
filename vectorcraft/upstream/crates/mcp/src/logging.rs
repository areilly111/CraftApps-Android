//! `logging/setLevel` and the `notifications/message` it turns on.
//!
//! VectorCraft's crates log through the `log` facade, and until now nothing installed a logger, so
//! every `log::debug!` went nowhere. This module is the sink for them: a `log` logger that turns
//! records into MCP `notifications/message`, filtered to the level the client asked for.
//!
//! The **binary** owns the choice of logger and calls [`install`]; the library never does. A
//! library that installed a process-wide logger and set the max level would silence whatever
//! logger an embedder had already set up, without being asked — so `Server::new` only forwards a
//! `logging/setLevel` the client explicitly sent.
//!
//! Nothing is sent before the client asks for it, because MCP has no business pushing log lines at
//! a client that never asked.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use log::{Level, LevelFilter};
use serde_json::{Value, json};

/// The severity from the last `logging/setLevel`, as an index into [`LEVELS`]; [`OFF`] means silent.
static LEVEL: AtomicU8 = AtomicU8::new(OFF);

/// The stored value that means "no logging".
const OFF: u8 = u8::MAX;

/// Records waiting to be drained into the next `notifications/message` batch.
static QUEUE: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// RFC 5424 severities, from the least to the most severe, as the spec orders them.
const LEVELS: [&str; 8] = ["debug", "info", "notice", "warning", "error", "critical", "alert", "emergency"];

/// The severity a `logging/setLevel` level names, and the `log` level it filters at. `None` for
/// anything the spec doesn't name.
///
/// Four of the eight severities collapse onto `log`'s Error, so the index is kept as well: a
/// client that asked for `emergency` must be able to hear that it is `emergency`, not `error`.
fn parse(name: &str) -> Option<(u8, Level)> {
    let level = match name {
        "debug" => Level::Debug,
        "info" | "notice" => Level::Info,
        "warning" => Level::Warn,
        "error" | "critical" | "alert" | "emergency" => Level::Error,
        _ => return None,
    };
    Some((LEVELS.iter().position(|l| *l == name)? as u8, level))
}

/// The severity name a `log` record is reported under.
fn severity(level: Level) -> &'static str {
    match level {
        Level::Trace | Level::Debug => "debug",
        Level::Info => "info",
        Level::Warn => "warning",
        Level::Error => "error",
    }
}

/// A `logging/setLevel` level the spec does not name. Nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownLevel;

/// Turn logging on at `level`, or off with `None`.
///
/// `Err(UnknownLevel)` means the name is not one the spec uses and nothing changed; a rejected
/// level must leave the accepted one in place rather than silencing logging entirely.
pub fn set_level(level: Option<&str>) -> Result<(), UnknownLevel> {
    let Some(name) = level else {
        LEVEL.store(OFF, Ordering::Relaxed);
        log::set_max_level(LevelFilter::Off);
        clear();
        return Ok(());
    };
    let Some((index, parsed)) = parse(name) else { return Err(UnknownLevel) };
    LEVEL.store(index, Ordering::Relaxed);
    log::set_max_level(parsed.to_level_filter());
    // Records already queued at the old level are dropped rather than leaked at the new one.
    clear();
    Ok(())
}

/// The level currently requested, if any.
pub fn level() -> Option<&'static str> {
    let index = LEVEL.load(Ordering::Relaxed);
    if index == OFF {
        return None;
    }
    LEVELS.get(usize::from(index)).copied()
}

/// Take the queued records, so the server can send them as `notifications/message`.
pub fn drain() -> Vec<Value> {
    match QUEUE.lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        // A poisoned lock only means some other thread panicked while logging; the records in it
        // are still readable and worth sending.
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
}

/// Forget whatever is queued (used when the client stops listening).
pub fn clear() {
    if let Ok(mut q) = QUEUE.lock() {
        q.clear();
    }
}

/// The `log` logger that feeds [`QUEUE`].
struct Sink;

impl log::Log for Sink {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        LEVEL.load(Ordering::Relaxed) != OFF && metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = json!({
            "level": severity(record.level()),
            "logger": record.target(),
            "data": record.args().to_string(),
        });
        // A full queue means the client is not draining; drop rather than grow without bound.
        if let Ok(mut q) = QUEUE.lock()
            && q.len() < MAX_QUEUED
        {
            q.push(message);
        }
    }

    fn flush(&self) {}
}

/// Records kept while the client is behind.
const MAX_QUEUED: usize = 512;

static SINK: Sink = Sink;

/// Install the sink once per process. The binary calls this; the library does not.
///
/// Later calls are ignored: the second `Server` in a test run finds it already there, which is
/// exactly what it wants.
pub fn install() {
    // `set_logger` fails only when a logger is already installed; keeping the first one is correct.
    let _already_installed = log::set_logger(&SINK);
    // Silent until a client sends `logging/setLevel`, so an embedder that installs this sink for
    // its own reasons gets no records it did not ask for.
    log::set_max_level(LevelFilter::Off);
}

/// The level and the queue are process-wide, so the tests that move them take turns. The server
/// itself is one thread, so nothing else has to.
#[cfg(test)]
pub(crate) fn test_turn() -> std::sync::MutexGuard<'static, ()> {
    match TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_round_trip_and_reject_nonsense() {
        let _turn = test_turn();
        install();
        assert_eq!(set_level(Some("warning")), Ok(()));
        assert_eq!(level(), Some("warning"));
        assert_eq!(set_level(Some("nope")), Err(UnknownLevel), "an unknown level changes nothing");
        assert_eq!(level(), Some("warning"), "the old level survives a bad request");
        assert_eq!(set_level(Some("emergency")), Ok(()));
        assert_eq!(level(), Some("emergency"));
        assert_eq!(set_level(None), Ok(()));
        assert_eq!(level(), None);
    }

    #[test]
    fn every_spec_level_is_understood() {
        let _turn = test_turn();
        install();
        for name in LEVELS {
            assert_eq!(set_level(Some(name)), Ok(()), "{name}");
            assert_eq!(level(), Some(name), "{name}");
        }
        set_level(None).ok();
    }

    #[test]
    fn records_appear_as_messages_only_when_asked_for() {
        let _turn = test_turn();
        install();
        set_level(None).ok();
        clear();
        log::error!("before logging was enabled");
        assert!(drain().is_empty(), "silent until set_level");

        set_level(Some("info")).ok();
        clear();
        log::warn!("hello {name}", name = "world");
        // Another test thread may log while logging is on, so look for ours rather than counting.
        let got = drain();
        let ours = got.iter().find(|m| m["data"] == "hello world").unwrap_or_else(|| panic!("not in {got:?}"));
        assert_eq!(ours["level"], "warning");
        assert!(ours["logger"].as_str().unwrap_or_default().starts_with("vectorcraft_mcp"), "{ours}");

        // Below the requested level stays out.
        set_level(Some("error")).ok();
        clear();
        log::info!("too chatty");
        assert!(drain().is_empty());
        set_level(None).ok();
    }
}
