// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft");

/// Invalid roots must exit before desktop initialization on every platform, including a CI
/// machine with no display. A regression must fail rather than hang with a window open.
fn rejects(args: &[&str], message: &str) {
    let mut child = Command::new(BIN)
        .args(args)
        .env_remove("VECTORCRAFT_CONTROL_PORT")
        .env_remove("VECTORCRAFT_AUTOMATION_READ_ROOT")
        .env_remove("VECTORCRAFT_AUTOMATION_WRITE_ROOT")
        .env("VECTORCRAFT_NO_PREFS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = child.wait_with_output().unwrap();
    assert!(!timed_out, "{args:?}: desktop started or did not exit promptly");
    assert!(!out.status.success(), "{args:?}");
    assert!(out.stdout.is_empty(), "{args:?}: {:?}", out.stdout);
    // Exact stderr excludes unrelated startup warnings or display errors that could otherwise
    // make a malformed invocation fail for an unrelated reason.
    assert_eq!(String::from_utf8(out.stderr).unwrap(), format!("vectorcraft: {message}\n"), "{args:?}");
}

#[test]
fn missing_empty_or_option_operands_fail_before_startup() {
    for flag in ["--automation-read-root", "--automation-write-root"] {
        rejects(&["--control", "7979", flag], &format!("{flag} needs a folder"));
        rejects(&[flag], &format!("{flag} needs a folder"));
        rejects(&["--control", "7979", &format!("{flag}=")], &format!("{flag} needs a non-empty folder"));
        rejects(&[flag, ""], &format!("{flag} needs a non-empty folder"));
        for next in ["--version", "--control", "--automation-write-root", "-folder"] {
            rejects(&[flag, next], &format!("{flag} needs a folder; use {flag}=<dir> for a folder starting with '-'"));
        }
    }
}

#[test]
fn duplicate_roots_fail_for_both_spellings() {
    for flag in ["--automation-read-root", "--automation-write-root"] {
        rejects(&[flag, ".", &format!("{flag}=.")], &format!("{flag} given twice"));
        rejects(&[&format!("{flag}=."), flag, "."], &format!("{flag} given twice"));
    }
}

#[test]
fn explicit_dash_folder_does_not_consume_version() {
    let out = Command::new(BIN).args(["--automation-read-root=-folder", "--automation-write-root", "./-folder", "--version"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("vectorcraft {}\n", env!("CARGO_PKG_VERSION")));
    assert!(out.stderr.is_empty());
}
