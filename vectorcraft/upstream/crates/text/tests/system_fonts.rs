//! A new session finds the installed system fonts by family name (#130). One test in its own
//! process, so the process-wide database starts fresh, as it does when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_text::{FontDb, system_font_dirs};

#[test]
fn a_fresh_session_finds_installed_fonts_by_name() {
    let bundled = FontDb::with_font_dirs(vec![]).families();
    let probe = FontDb::with_font_dirs(system_font_dirs());
    probe.load_system_fonts();
    let installed = probe.families();
    let Some(family) = installed.iter().find(|f| !bundled.contains(f)) else {
        eprintln!("no system fonts installed: nothing to check");
        return;
    };
    // The first lookup in the session: nothing has scanned the font folders yet.
    let face = FontDb::global().face(family, "Regular").unwrap();
    assert_eq!(&face.family, family);
    assert!(FontDb::global().families().contains(family));
}
