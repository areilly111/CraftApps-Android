//! Prepare for accessibility ▸ Check for accessibility in the real shell (egui_kittest): the
//! options dialog, the results panel, Fix, Skip Rule and the report.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::a11y::{Rule, Status};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp, RightPanel};

const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 37 >> stream
BT /F1 12 Tf 20 50 Td (Hello) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn status(h: &Harness<'static, PdfCraftApp>, rule: Rule) -> Status {
    h.state().a11y.report.as_ref().unwrap().1.result(rule).unwrap().status
}

fn right_click(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn checking_fixing_skipping_and_reporting() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-a11y-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = dir.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("notes.pdf", None, FIXTURE.to_vec()).unwrap();
        app.export_dir_override = Some(d.to_string_lossy().into_owned());
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("a11y.check"));
    h.run_steps(2);
    h.get_by_label("Accessibility Checker Options");
    h.get_by_label_contains("31 of 32");
    h.get_by_label("Document title is showing in title bar");
    h.get_by_label("Start Checking").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, None);
    assert_eq!(h.state().right, Some(RightPanel::Accessibility));
    h.get_by_label("Document (3 issues)");
    h.get_by_label("Title - Failed");
    h.get_by_label("Color contrast - Skipped");
    // Fix the title: the file name becomes the title and it shows; the description opens to edit it.
    right_click(&mut h, "Title - Failed");
    h.get_by_label("Fix").click();
    h.run_steps(3);
    assert_eq!(status(&h, Rule::Title), Status::Passed);
    assert_eq!(h.state().a11y.previous.as_ref().unwrap().1.result(Rule::Title).unwrap().status, Status::Failed);
    assert!(matches!(h.state().dialog, Some(Dialog::Properties(_))));
    assert_eq!(h.state().window_title, "notes — PdfCraft");
    h.state_mut().dialog = None;
    h.run_steps(2);
    h.get_by_label("Compare");
    h.get_by_label_contains("Title: Failed (2) → Passed (0)");
    // Skip a rule; it stays skipped when checking again.
    right_click(&mut h, "Tagged PDF - Failed");
    h.get_by_label("Skip Rule").click();
    h.run_steps(2);
    h.get_by_label("Check again").click();
    h.run_steps(3);
    assert_eq!(status(&h, Rule::TaggedPdf), Status::Skipped);
    h.get_by_label("Document (1 issue)");
    // Explain shows why the rule matters; findings list the problems.
    right_click(&mut h, "Primary language - Failed");
    h.get_by_label("Explain").click();
    h.run_steps(2);
    h.get_by_label_contains("Screen readers choose their voice");
    h.get_by_label("Primary language - Failed").click();
    h.run_steps(2);
    h.get_by_label("No document language is set");
    // The report.
    h.get_by_label("Report").click();
    h.run_steps(2);
    let html = std::fs::read_to_string(dir.join("notes Accessibility Report.html")).unwrap();
    assert!(html.contains("Accessibility Report") && html.contains("Skipped"));
}

#[test]
fn checker_summary_does_not_hide_manual_review() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("notes.pdf", None, FIXTURE.to_vec()).unwrap();
        app
    });
    h.run_steps(3);
    h.state_mut().run_accessibility_check();
    let report = &mut h.state_mut().a11y.report.as_mut().unwrap().1;
    for result in &mut report.results {
        result.status = Status::Passed;
        result.findings.clear();
    }
    report.results[0].status = Status::Manual;
    h.run_steps(2);
    h.get_by_label("Needs manual check: 1");
    assert!(h.query_by_label("No issues found").is_none());
    // Every rule passing keeps the plain success wording.
    h.state_mut().a11y.report.as_mut().unwrap().1.results[0].status = Status::Passed;
    h.run_steps(2);
    h.get_by_label("No issues found");
}

/// Two tagged figures: an image without alternate text and a bar (role-mapped Photo) with.
const FIGURES: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /StructParents 0 /Resources << /XObject << /Im 10 0 R >> >> >> endobj
4 0 obj << /Length 97 >> stream
/Figure << /MCID 0 >> BDC q 40 0 0 20 10 150 cm /Im Do Q EMC /Photo << /MCID 1 >> BDC 100 100 50 30 re f EMC
endstream endobj
5 0 obj << /Type /StructTreeRoot /K 6 0 R /ParentTree 9 0 R /RoleMap << /Photo /Figure >> >> endobj
6 0 obj << /S /Document /P 5 0 R /K [7 0 R 8 0 R] >> endobj
7 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 0 >> endobj
8 0 obj << /S /Photo /P 6 0 R /Pg 3 0 R /Alt (A bar) /K 1 >> endobj
9 0 obj << /Nums [0 [7 0 R 8 0 R]] >> endobj
10 0 obj << /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >> stream
\x00
endstream endobj
trailer << /Root 1 0 R >>
%%EOF";

#[test]
fn checker_shows_a_figure_finding_on_its_page() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("figures.pdf", None, FIGURES.to_vec()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().run_accessibility_check();
    h.run_steps(3);
    h.get_by_label("Figures alternate text - Failed").click();
    h.run_steps(2);
    h.get_by_label_contains("Figure element on page 1: no alternate text").click();
    h.run_steps(1);
    let flash = h.state().views[0].flash.map(|(page, rect, _)| (page, rect));
    assert_eq!(flash, Some((0, [10.0, 150.0, 50.0, 170.0])));
}

#[test]
fn setting_alternate_text_figure_by_figure() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("figures.pdf", None, FIGURES.to_vec()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("a11y.alt_text"));
    h.run_steps(2);
    h.get_by_label("Set Alternate Text");
    h.get_by_label("Figure 1 of 2 on page 1");
    h.state_mut().alt_draft.texts[0] = "A grey square".into();
    h.run_steps(2);
    h.get_by_label("Next figure").click();
    h.run_steps(2);
    h.get_by_label("Figure 2 of 2 on page 1");
    h.get_by_label("Decorative figure").click();
    h.run_steps(1);
    h.get_by_label("Save & Close").click();
    h.run_steps(3);
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    assert_eq!(doc.can_undo(), Some("Set alternate text"));
    let f = doc.figures();
    assert_eq!(f.len(), 1, "the decorative one left the tags");
    assert_eq!(f[0].alt.as_deref(), Some("A grey square"));
}

#[test]
fn tags_panel_edits_a_tag_property_and_undoes_it() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("figures.pdf", None, FIGURES.to_vec()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("a11y.tags"));
    h.run_steps(2);
    assert_eq!(h.state().right, Some(RightPanel::Tags));
    h.get_by_label("Collapse tag").click();
    h.run_steps(2);
    assert!(h.query_by_label("Figure · 7").is_none());
    h.get_by_label("Expand tag").click();
    h.run_steps(2);
    h.get_by_label("Figure · 7").click();
    h.run_steps(2);
    let title = h.get_by_role_and_label(Role::TextInput, "Title");
    title.focus();
    title.type_text("First figure");
    h.run_steps(1);
    h.get_by_label("Save tag properties").click();
    h.run_steps(3);
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    assert_eq!(doc.tags()[1].title.as_deref(), Some("First figure"));
    assert_eq!(doc.can_undo(), Some("Change tag properties"));
    let search = h.get_by_role_and_label(Role::TextInput, "Find a tag");
    search.focus();
    search.type_text("Photo");
    h.run_steps(2);
    h.get_by_label("Document · 6");
    h.get_by_label("Photo · 8");
    assert!(h.query_by_label("First figure · 7").is_none());
}

#[test]
fn checker_finding_opens_its_structure_tag() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("figures.pdf", None, FIGURES.to_vec()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().run_accessibility_check();
    h.state_mut().a11y.expanded.insert(Rule::FiguresAltText);
    h.run_steps(3);
    let finding = h.state().a11y.report.as_ref().unwrap().1.result(Rule::FiguresAltText).unwrap().findings[0].clone();
    assert_eq!(finding.object.map(|o| o.num), Some(7));
    h.get_by_label(&finding.message).click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Tags));
    h.get_by_label("7 0 R");
    let title = h.get_by_role_and_label(Role::TextInput, "Title");
    title.focus();
    title.type_text("Unsaved title");
    h.run_steps(1);
    h.state_mut().right = Some(RightPanel::Accessibility);
    h.run_steps(2);
    h.get_by_label(&finding.message).click();
    h.run_steps(2);
    assert_eq!(h.state().right, Some(RightPanel::Accessibility), "an unfinished tag edit is preserved");
}
