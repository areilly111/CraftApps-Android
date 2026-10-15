//! Addresses a document asks to open (links, button URI actions, `app.launchURL`) wait for the
//! user's permission, and anything but a web or email address is refused (#90, #91).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{LinkOrigin, PdfCraftApp, PendingLink};

/// See `tests/view.rs`: WARP's shader JIT crashes when two devices compile at once on ARM64.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

const LINK_URL: &str = "https://example.org/from-a-link";
const BUTTON_URL: &str = "https://example.org/from-a-button";

/// One 300×400 page with a web link (x 50..250, y 300..350), a push button with a URI action
/// ("web") and a push button whose JavaScript asks to open a local program ("sneaky").
fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 6 0 R] /DA (/Helv 0 Tf 0 g) >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R] >>".to_string(),
        format!("<< /Type /Annot /Subtype /Link /Rect [50 300 250 350] /Border [0 0 0] /A << /S /URI /URI ({LINK_URL}) >> >>"),
        format!("<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (web) /Rect [20 200 100 220] /P 3 0 R /A << /S /URI /URI ({BUTTON_URL}) >> >>"),
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (sneaky) /Rect [120 200 200 220] /P 3 0 R /AA << /U << /S /JavaScript /JS (app.launchURL\\(\"file:///C:/Windows/System32/calc.exe\", true\\);) >> >> >>".to_string(),
    ];
    let objs = objs
        .into_iter()
        .chain([
            "<< /Type /Annot /Subtype /Link /Rect [50 250 250 280] /A << /S /GoToR /F (target.pdf) /D [0 /Fit] >> >>".to_string(),
            "<< /Type /Annot /Subtype /Link /Rect [50 220 250 240] /A << /S /Launch /F (program) >> >>".to_string(),
        ])
        .collect::<Vec<_>>();
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn remote_fixture(file: &str, destination: &str) -> Vec<u8> {
    // canonicalize() uses a Windows device prefix; a PDF file specification uses the ordinary drive path.
    #[cfg(windows)]
    let file = file.strip_prefix(r"\\?\").filter(|path| path.as_bytes().get(1) == Some(&b':')).unwrap_or(file);
    let file = file.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_string(),
        format!("<< /Type /Annot /Subtype /Link /Rect [50 300 250 350] /A << /S /GoToR /F ({file}) /D {destination} /NewWindow true >> >>"),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn remote_harness(source_path: Option<String>, file: &str, destination: &str) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("source.pdf", source_path, remote_fixture(file, destination)).unwrap();
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "none").unwrap();
        app.set_option("zoom", "150").unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn linked_pages(count: usize, password: Option<&str>) -> Vec<u8> {
    use pdfcraft_engine::{Edit, Protection, Session};
    let mut session = Session::new();
    let id = session.open("target.pdf", None, std::sync::Arc::new(fixture()), None).unwrap();
    for page in 1..count {
        session.apply(id, Edit::InsertBlankPage { at: page, width: 300.0, height: 400.0 }).unwrap();
    }
    if let Some(password) = password {
        session.apply(id, Edit::Protect(Protection { open_password: Some(password.into()), ..Default::default() })).unwrap();
    }
    session.save_bytes(id).unwrap().as_ref().clone()
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("links.pdf", None, fixture()).unwrap();
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "none").unwrap();
        app.set_option("zoom", "150").unwrap();
        app
    });
    h.run_steps(8);
    h
}

fn click_at(h: &mut Harness<'static, PdfCraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.drop_at(at);
    h.run_steps(4);
}

/// Click the link on page 1, at the middle of its rectangle.
fn click_link_at(h: &mut Harness<'static, PdfCraftApp>, y: f32) {
    let r = h.state().views[0].page_screen_rect(0).expect("page 1 on screen");
    let at = egui::pos2(r.min.x + r.width() * (150.0 / 300.0), r.min.y + r.height() * (1.0 - y / 400.0));
    click_at(h, at);
}

fn click_remote_link(h: &mut Harness<'static, PdfCraftApp>) {
    click_link(h);
}

fn click_link(h: &mut Harness<'static, PdfCraftApp>) {
    click_link_at(h, 325.0);
}

fn click_field(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    let at = {
        let s = h.state();
        let doc = s.session.get(s.views[0].id).unwrap();
        let f = doc.form.iter().find(|f| f.name == name).unwrap();
        pdfcraft_ui_egui::forms_ui::field_screen_rect(&s.views[0], &doc.info, f, 0).expect("on screen").center()
    };
    click_at(h, at);
}

fn pending(url: &str, origin: LinkOrigin) -> Option<PendingLink> {
    Some(PendingLink { url: url.into(), origin })
}

#[test]
fn a_link_asks_first_and_opens_when_allowed() {
    let _gpu = gpu();
    let mut h = harness();
    click_link(&mut h);
    assert_eq!(h.state().pending_link, pending(LINK_URL, LinkOrigin::Link));
    assert_eq!(h.state().last_opened_url, None, "nothing opens before the user answers");
    h.get_by_label("example.org");
    h.get_by_label("Open link").click();
    h.run_steps(3);
    assert_eq!(h.state().last_opened_url.as_deref(), Some(LINK_URL));
    assert_eq!(h.state().pending_link, None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_local_gotor_link_opens_in_a_tab_and_applies_its_destination() {
    let _gpu = gpu();
    let dir = std::env::temp_dir().join(format!("pdfcraft-ui-gotor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("target.pdf"), linked_pages(2, None)).unwrap();
    let mut h = remote_harness(Some(dir.join("source.pdf").to_string_lossy().into_owned()), "target.pdf", "[1 /FitH 200]");
    click_remote_link(&mut h);
    assert_eq!(h.state().views.len(), 2);
    assert_eq!(h.state().active, Some(1));
    assert_eq!(h.state().views[1].current, 1, "the remote page destination is applied");
    assert_eq!(h.state().last_opened_url, None, "no URL or external launcher is involved");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_remote_destination_survives_password_retries_and_cancel_opens_no_tab() {
    let _gpu = gpu();
    let dir = std::env::temp_dir().join(format!("pdfcraft-ui-gotor-password-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("target.pdf"), linked_pages(2, Some("test-password"))).unwrap();

    let mut h = remote_harness(Some(dir.join("source.pdf").to_string_lossy().into_owned()), "target.pdf", "[1 /Fit]");
    click_remote_link(&mut h);
    assert!(h.state().password_prompt.is_some());
    assert_eq!(h.state().views.len(), 1);
    h.state_mut().submit_password(Some("wrong-password".into()));
    assert!(h.state().password_prompt.as_ref().is_some_and(|p| p.error.is_some()));
    h.state_mut().submit_password(Some("test-password".into()));
    assert_eq!(h.state().views.len(), 2);
    assert_eq!(h.state().views[1].current, 1, "the destination survives a wrong-password retry");

    let mut cancelled = remote_harness(Some(dir.join("source.pdf").to_string_lossy().into_owned()), "target.pdf", "[1 /Fit]");
    click_remote_link(&mut cancelled);
    cancelled.state_mut().submit_password(None);
    assert_eq!(cancelled.state().views.len(), 1, "cancelling does not leave a half-open target tab");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_gotor_link_to_a_directory_or_device_is_refused_without_reading_it() {
    let _gpu = gpu();
    let dir = std::env::temp_dir().join(format!("pdfcraft-ui-gotor-not-a-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("folder.pdf")).unwrap();
    // Only Unix adds a target (a device), so elsewhere the list is never changed.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut targets = vec!["folder.pdf"];
    #[cfg(unix)]
    {
        // /dev/zero never ends: reading it would exhaust memory.
        std::os::unix::fs::symlink("/dev/zero", dir.join("zero.pdf")).unwrap();
        targets.push("zero.pdf");
    }
    for target in targets {
        let mut h = remote_harness(Some(dir.join("source.pdf").to_string_lossy().into_owned()), target, "[0 /Fit]");
        click_remote_link(&mut h);
        assert_eq!(h.state().views.len(), 1, "{target} opens no tab");
        let toast = &h.state().toast.as_ref().unwrap().0;
        assert!(toast.contains("not a regular file"), "{target}: {toast}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_in_memory_source_does_not_resolve_a_relative_gotor_against_cwd() {
    let _gpu = gpu();
    let mut h = remote_harness(None, "target.pdf", "[0 /Fit]");
    click_remote_link(&mut h);
    assert_eq!(h.state().views.len(), 1);
    let toast = &h.state().toast.as_ref().unwrap().0;
    assert!(toast.contains("relative link needs a source document path"), "{toast}");
}

#[test]
fn unsupported_launch_action_is_reported_without_claiming_javascript() {
    let _gpu = gpu();
    let mut h = harness();
    click_link_at(&mut h, 230.0);
    assert_eq!(h.state().pending_link, None);
    assert_eq!(h.state().last_opened_url, None);
    assert_eq!(h.state().views.len(), 1);
    assert_eq!(h.state().views[0].current, 0);
    let toast = h.state().toast.clone().expect("the unsupported action is reported").0;
    assert_eq!(toast, "This PDF uses the unsupported Launch action.");
}

#[test]
fn cancel_opens_nothing() {
    let _gpu = gpu();
    let mut h = harness();
    click_link(&mut h);
    assert!(h.state().pending_link.is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().pending_link, None);
    assert_eq!(h.state().last_opened_url, None);
}

#[test]
fn escape_cancels() {
    let _gpu = gpu();
    let mut h = harness();
    click_link(&mut h);
    assert!(h.state().pending_link.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().pending_link, None);
    assert_eq!(h.state().last_opened_url, None);
}

#[test]
fn a_button_uri_action_asks_first() {
    let _gpu = gpu();
    let mut h = harness();
    click_field(&mut h, "web");
    assert_eq!(h.state().pending_link, pending(BUTTON_URL, LinkOrigin::Button));
    assert_eq!(h.state().last_opened_url, None);
}

#[test]
fn a_button_script_cannot_open_a_local_program() {
    let _gpu = gpu();
    let mut h = harness();
    click_field(&mut h, "sneaky");
    assert_eq!(h.state().pending_link, None, "a file: address is never offered");
    assert_eq!(h.state().last_opened_url, None);
    let toast = h.state().toast.clone().expect("the user is told").0;
    assert!(toast.contains("“file:” address"), "{toast}");
}

#[test]
fn a_script_asks_first_and_only_once() {
    let _gpu = gpu();
    let mut h = harness();
    let id = h.state().views[0].id;
    let script = "app.launchURL('https://example.org/?q=' + 'first'); app.launchURL('https://example.org/second');";
    h.state_mut().run_button_script(id, "web", script);
    h.run_steps(3);
    assert_eq!(h.state().pending_link, pending("https://example.org/?q=first", LinkOrigin::Script), "one dialog, for the first request");
    assert_eq!(h.state().last_opened_url, None);
}

#[test]
fn a_script_cannot_open_other_kinds_of_address() {
    let _gpu = gpu();
    // `mailto://a^b/x` and `https://a^b/` don't parse as URLs, so the browser would be handed them
    // as local file paths.
    for url in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ms-settings:privacy",
        "smb://server/share",
        "https://exa\u{202E}gro.elpmaxe",
        "mailto://a^b/x",
        "https://a^b/",
    ] {
        let mut h = harness();
        let id = h.state().views[0].id;
        let script = format!("app.launchURL({});", serde_json::to_string(url).unwrap());
        h.state_mut().run_button_script(id, "web", &script);
        h.run_steps(3);
        assert_eq!(h.state().pending_link, None, "{url}");
        assert_eq!(h.state().last_opened_url, None, "{url}");
        assert!(h.state().toast.clone().is_some_and(|t| t.0.contains("won't open")), "{url}: {:?}", h.state().toast);
    }
}

#[test]
fn an_email_link_cannot_attach_a_local_file() {
    let _gpu = gpu();
    for url in ["mailto:a@example.org?attach=/home/me/.ssh/id_ed25519", "mailto:a@example.org?subject=Hi&%61ttachment=C:/Users/me/x.txt"] {
        let mut h = harness();
        let id = h.state().views[0].id;
        let script = format!("app.launchURL({});", serde_json::to_string(url).unwrap());
        h.state_mut().run_button_script(id, "web", &script);
        h.run_steps(3);
        assert_eq!(h.state().pending_link, None, "{url}");
        assert_eq!(h.state().last_opened_url, None, "{url}");
        let toast = h.state().toast.clone().expect("the user is told").0;
        assert!(toast.contains("attach or insert a file"), "{url}: {toast}");
    }
}

/// Ask to open `url` from a script and return the harness with the prompt showing.
fn prompt_for(url: &str) -> Harness<'static, PdfCraftApp> {
    let mut h = harness();
    let id = h.state().views[0].id;
    let script = format!("app.launchURL({});", serde_json::to_string(url).unwrap());
    h.state_mut().run_button_script(id, "web", &script);
    h.run_steps(3);
    assert_eq!(h.state().pending_link, pending(url, LinkOrigin::Script));
    h
}

#[test]
fn a_lookalike_host_is_shown_in_punycode_and_flagged() {
    let _gpu = gpu();
    // `pаypal.com` with a Cyrillic `а` (U+0430).
    let h = prompt_for("https://p\u{0430}ypal.com/login");
    h.get_by_label("xn--pypal-4ve.com");
    assert_eq!(h.query_all_by_label_contains("p\u{0430}ypal.com").count(), 1, "only the full address shows the lookalike form");
    h.get_by_label_contains("mixes letters from different alphabets");
}

#[test]
fn an_international_host_is_shown_in_punycode_and_marked() {
    let _gpu = gpu();
    let h = prompt_for("https://bücher.example/");
    h.get_by_label("xn--bcher-kva.example");
    assert_eq!(h.query_all_by_label_contains("bücher.example").count(), 1, "only the full address shows the Unicode form");
    h.get_by_label_contains("letters from another alphabet");
    assert!(h.query_by_label_contains("mixes letters").is_none());
}

#[test]
fn a_numeric_host_is_shown_as_the_address_it_goes_to() {
    let _gpu = gpu();
    let h = prompt_for("http://3232235777/admin");
    h.get_by_label("192.168.1.1");
    assert!(h.query_by_label_contains("alphabet").is_none());
}
