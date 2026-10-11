//! Help ▸ Enable Logging / Reveal Logging File: a `log` logger that, while enabled, appends
//! every record to `Logs/EffectCraft Log.txt` in the settings folder (or the temporary folder),
//! and always keeps the last few hundred warnings and errors in memory for the System
//! Compatibility Report.
//!
//! Frontends call [`install`] once at startup; [`set_enabled`] (the `help.enableLogging`
//! command) turns file logging on and off. `RUST_LOG` (a level such as `debug`, or per target:
//! `effectcraft=debug,wgpu=warn`) also prints what it lets through to stderr, so a start-up that
//! fails before the window opens leaves a trace.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use log::LevelFilter;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// The `RUST_LOG` filter for stderr: (target prefix or every target, level).
static STDERR: OnceLock<Vec<(Option<String>, LevelFilter)>> = OnceLock::new();
static STATE: Mutex<State> = Mutex::new(State { file: None, recent: VecDeque::new() });

struct State {
    file: Option<PathBuf>,
    recent: VecDeque<String>,
}

struct Logger;

const RECENT: usize = 300;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (ENABLED.load(Ordering::Relaxed) && m.target().starts_with("effectcraft")) || to_stderr(m)
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}: {}", unix_secs(), r.level(), r.target(), r.args());
        if to_stderr(r.metadata()) {
            eprintln!("{line}");
        }
        let Ok(mut st) = STATE.lock() else { return };
        if r.level() <= log::Level::Warn {
            st.recent.push_back(line.clone());
            if st.recent.len() > RECENT {
                st.recent.pop_front();
            }
        }
        if ENABLED.load(Ordering::Relaxed)
            && let Some(f) = &st.file
            && let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(f)
        {
            let _ = writeln!(out, "{line}");
        }
    }
    fn flush(&self) {}
}

fn unix_secs() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Install the logger (once; later calls and other installed loggers are left alone).
pub fn install() {
    static LOGGER: Logger = Logger;
    if log::set_logger(&LOGGER).is_ok() {
        let stderr = STDERR.get_or_init(|| std::env::var("RUST_LOG").map(|s| parse_filter(&s)).unwrap_or_default());
        log::set_max_level(stderr.iter().map(|d| d.1).fold(LevelFilter::Info, Ord::max));
    }
}

/// Whether `RUST_LOG` lets a record through to stderr.
fn to_stderr(m: &log::Metadata) -> bool {
    STDERR.get().is_some_and(|f| m.level() <= level_for(f, m.target()))
}

/// `RUST_LOG`'s directives, as env_logger reads them: `level`, `target=level` or a bare target
/// (every level); unknown levels are skipped.
fn parse_filter(spec: &str) -> Vec<(Option<String>, LevelFilter)> {
    spec.split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .filter_map(|d| match d.split_once('=') {
            Some((t, l)) => Some((Some(t.trim().to_string()), l.trim().parse().ok()?)),
            None => Some(d.parse().map_or_else(|_| (Some(d.to_string()), LevelFilter::Trace), |l| (None, l))),
        })
        .collect()
}

/// The level of the directive whose target is the longest prefix of `target` (a directive
/// without a target matches every target); off when none does.
fn level_for(filter: &[(Option<String>, LevelFilter)], target: &str) -> LevelFilter {
    filter
        .iter()
        .filter(|(t, _)| t.as_deref().is_none_or(|t| target.starts_with(t)))
        .max_by_key(|(t, _)| t.as_ref().map_or(0, String::len))
        .map_or(LevelFilter::Off, |d| d.1)
}

/// Log every panic (message and location) through the logger, then run the previous hook
/// (which prints it). Panics in commands are then caught by [`crate::guard::guarded`].
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}

/// The log file inside `config_dir` (the settings folder), or the temporary folder.
pub fn log_path(config_dir: Option<&Path>) -> PathBuf {
    config_dir.map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir).join("Logs").join("EffectCraft Log.txt")
}

/// Turn file logging on (writing to `file`) or off.
pub fn set_enabled(on: bool, file: PathBuf) -> std::io::Result<()> {
    install();
    if on {
        if let Some(d) = file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&file)?;
        writeln!(f, "{} INFO  effectcraft: logging enabled ({} {})", unix_secs(), std::env::consts::OS, env!("CARGO_PKG_VERSION"))?;
    }
    if let Ok(mut st) = STATE.lock() {
        st.file = Some(file);
    }
    ENABLED.store(on, Ordering::Relaxed);
    Ok(())
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The current log file (set once logging was enabled).
pub fn current_file() -> Option<PathBuf> {
    STATE.lock().ok().and_then(|s| s.file.clone())
}

/// Recent warnings and errors, oldest first.
pub fn recent() -> Vec<String> {
    STATE.lock().map(|s| s.recent.iter().cloned().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{LevelFilter, level_for, parse_filter};

    /// `RUST_LOG` picks what start-up prints to stderr (#234).
    #[test]
    fn rust_log_filters_by_level_and_target() {
        let f = parse_filter("info, wgpu=warn,wgpu_core=debug, effectcraft , bogus=loud");
        assert_eq!(level_for(&f, "eframe::native"), LevelFilter::Info);
        assert_eq!(level_for(&f, "wgpu_hal::metal"), LevelFilter::Warn);
        assert_eq!(level_for(&f, "wgpu_core::device"), LevelFilter::Debug, "the longest matching target wins");
        assert_eq!(level_for(&f, "effectcraft_ui_egui"), LevelFilter::Trace, "a bare target: every level");
        assert_eq!(f.len(), 4, "an unknown level is skipped");
        assert_eq!(level_for(&parse_filter("wgpu=debug"), "winit"), LevelFilter::Off);
        assert!(parse_filter("").is_empty());
    }
}
