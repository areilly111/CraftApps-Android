//! PDF import names type by the installed fonts in a new session (#130). One test in its own
//! process, so the process-wide font database starts fresh, as it does when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_doc::NodeKind;
use vectorcraft_pdf::{ImportOptions, import_with_report};
use vectorcraft_testkit::pdf::{PdfPage, pdf_with};
use vectorcraft_text::{FontDb, system_font_dirs};

#[test]
fn a_fresh_session_resolves_pdf_fonts_to_installed_families() {
    let bundled = FontDb::with_font_dirs(vec![]).families();
    let probe = FontDb::with_font_dirs(system_font_dirs());
    probe.load_system_fonts();
    let installed = probe.families();
    // Families whose PDF name is their name without the spaces (as "TimesNewRoman"). The font
    // isn't embedded, so the page draws a stand-in; text stays live only where the installed
    // font's glyphs match it, as a plain sans-serif's do and a condensed display face's don't:
    // which family that is depends on the machine's fonts, so the first of them to do is checked.
    let candidates: Vec<&String> = installed
        .iter()
        .filter(|f| !bundled.contains(f) && f.contains(' ') && f.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '))
        .take(200)
        .collect();
    if candidates.is_empty() {
        eprintln!("no system fonts installed: nothing to check");
        return;
    }
    let import = |family: &str| {
        let resources = format!("/Font << /F1 << /Type /Font /Subtype /TrueType /BaseFont /{} >> >>", family.replace(' ', ""));
        let page = PdfPage { resources, ..PdfPage::new(100.0, 100.0, "BT /F1 12 Tf 10 30 Td (Hi) Tj ET") };
        let r = import_with_report(&pdf_with(&[page], &[], None), &ImportOptions::default()).unwrap();
        let mut families = vec![];
        r.document.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                families.extend(t.runs.iter().map(|r| r.style.font_family.clone()));
            }
        });
        (families, r.warnings)
    };
    let found = candidates.iter().find_map(|family| {
        let (families, warnings) = import(family);
        (!families.is_empty()).then_some((family, families, warnings))
    });
    let (family, families, warnings) = found.expect("one of the installed families stays live text");
    assert_eq!(families, std::slice::from_ref(*family));
    assert!(!warnings.iter().any(|w| w.contains(family.as_str())), "{warnings:?}");
}
