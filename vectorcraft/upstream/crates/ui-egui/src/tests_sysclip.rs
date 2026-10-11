//! The system clipboard's formats through a headless fake service (M4.57): what Copy publishes,
//! what Paste takes from other apps (bitmaps, PDF, text), the Paste menu and keyboard paste.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{BITMAP, Flavour, PDF, PNG, SVG, TEXT};
use vectorcraft_geom::Point;

use crate::{Services, SystemClipboard, VectorcraftApp, menus};

/// The fake clipboard's contents: what the app wrote, or what "another app" copied.
#[derive(Default)]
pub(super) struct Board {
    flavours: Vec<Flavour>,
    ours: bool,
}

struct Fake(Rc<RefCell<Board>>);

impl Fake {
    fn find(&self, mimes: &[&'static str]) -> Option<Flavour> {
        let b = self.0.borrow();
        let matches = |m: &str, f: &Flavour| f.mime == m || (m == BITMAP && f.mime.starts_with("image/") && f.mime != SVG);
        mimes.iter().find_map(|m| b.flavours.iter().find(|f| matches(m, f)).cloned())
    }
}

impl SystemClipboard for Fake {
    fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
        *self.0.borrow_mut() = Board { flavours: flavours.to_vec(), ours: true };
        Ok(())
    }
    fn holds_ours(&mut self) -> bool {
        self.0.borrow().ours
    }
    fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
        self.find(mimes)
    }
    fn has(&mut self, mimes: &[&'static str]) -> bool {
        self.find(mimes).is_some()
    }
}

/// An app with a document and a fake system clipboard, the view centred on (250, 180).
pub(super) fn app() -> (VectorcraftApp, Rc<RefCell<Board>>) {
    let board = Rc::new(RefCell::new(Board::default()));
    let services = Services { system_clipboard: Some(Box::new(Fake(board.clone()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    (app, board)
}

/// Another app copies `flavours`.
pub(super) fn copy_elsewhere(board: &Rc<RefCell<Board>>, flavours: Vec<(&'static str, Vec<u8>)>) {
    let flavours = flavours.into_iter().map(|(mime, data)| Flavour { mime, data }).collect();
    *board.borrow_mut() = Board { flavours, ours: false };
}

pub(super) fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

pub(super) fn copy_rect(app: &mut VectorcraftApp) {
    let id = run(app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].clone();
    run(app, "paint.setFill", json!({"color": "#ff0000"}));
    run(app, "select.set", json!({"ids": [id]}));
    run(app, "edit.copy", json!({}));
}

fn written(board: &Rc<RefCell<Board>>) -> Vec<&'static str> {
    board.borrow().flavours.iter().map(|f| f.mime).collect()
}

/// The objects the last paste selected.
pub(super) fn pasted(app: &VectorcraftApp) -> Vec<Node> {
    let st = app.session.active().unwrap();
    st.selection.objects.iter().map(|id| st.doc.node(*id).unwrap().clone()).collect()
}

fn count(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().doc.layers[0].children().unwrap().len()
}

/// A 4 × 2 px blue PNG.
pub(super) fn blue_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// One headless frame of app logic (shortcuts too, from the second frame on) delivering `events`,
/// in the test's one context (in a new one the app starts its fonts over).
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    thread_local! {
        static CTX: egui::Context = egui::Context::default();
    }
    let ctx = CTX.with(Clone::clone);
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| app.logic(ui.ctx())).textures_delta.clear();
}

fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

#[test]
fn copy_publishes_pdf_to_the_service_only_with_the_preference_on() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert!(String::from_utf8_lossy(&board.borrow().flavours[0].data).contains("<svg"));
    run(&mut app, "prefs.set", json!({"key": "copyAsPdf", "value": true}));
    run(&mut app, "edit.cut", json!({}));
    assert_eq!(written(&board), [TEXT, SVG, PDF, PNG]);
    assert!(board.borrow().flavours[2].data.starts_with(b"%PDF"));
    // Nothing goes through egui's text output when the service is there.
    assert!(app.clipboard_out.is_none());
}

#[test]
fn our_own_copy_pastes_the_lossless_internal_clipboard() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    // As if the clipboard held only the PNG: still ours, so the rectangle comes back as a path.
    board.borrow_mut().flavours.retain(|f| f.mime == PNG);
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
}

#[test]
fn a_bitmap_from_another_app_pastes_as_an_embedded_image_at_the_view_centre() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let n = &pasted(&app)[0];
    let NodeKind::Image(im) = &n.kind else { panic!("not an image: {:?}", n.kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    let b = n.geometric_bounds().unwrap();
    assert!((b.center() - Point::new(250.0, 180.0)).hypot() < 1e-6, "{b:?}");
}

#[test]
fn text_from_another_app_pastes_as_point_text_and_svg_text_as_art() {
    let (mut app, board) = app();
    // Word processors offer a picture of the text too: the text wins.
    copy_elsewhere(&board, vec![(TEXT, b"Pasted words".to_vec()), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "Pasted words");
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><circle cx="20" cy="5" r="5"/></svg>"##;
    copy_elsewhere(&board, vec![(TEXT, svg.as_bytes().to_vec())]);
    run(&mut app, "edit.paste", json!({}));
    assert_eq!(pasted(&app).len(), 2);
    assert!(pasted(&app).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // Text that can't be pasted pastes nothing (not the stale internal clipboard).
    let before = count(&app);
    copy_elsewhere(&board, vec![(TEXT, b"  \r\n".to_vec())]);
    assert!(app.run("edit.paste", json!({})).is_err());
    assert!(app.ui.status.starts_with("Couldn't paste"), "{}", app.ui.status);
    assert_eq!(count(&app), before);
}

/// A browser's Copy Image: the picture with its address as text (and HTML, which Paste doesn't
/// read). The picture is pasted; an address alone pastes as text, nothing is fetched (#597).
#[test]
fn an_image_copied_in_a_browser_pastes_as_the_picture_not_its_address() {
    let (mut app, board) = app();
    let url = b"https://example.com/images/cat.png".to_vec();
    copy_elsewhere(&board, vec![(TEXT, url.clone()), ("text/html", b"<img src=\"https://example.com/images/cat.png\">".to_vec()), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Image(im) = &pasted(&app)[0].kind else { panic!("not an image: {:?}", pasted(&app)[0].kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    copy_elsewhere(&board, vec![(TEXT, url)]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "https://example.com/images/cat.png");
}

#[test]
fn a_pdf_from_another_app_pastes_as_vectors_ahead_of_its_bitmap() {
    let (mut app, board) = app();
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = other.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap()["id"].clone();
    other.execute("select.set", &json!({"ids": [id]})).unwrap();
    other.execute("edit.copy", &json!({})).unwrap();
    let pdf = other.clipboard_pdf().unwrap().unwrap();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (PDF, pdf)]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_))), "{p:?}");
}

#[test]
fn a_bitmap_on_the_system_clipboard_enables_paste() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    assert!(!menus::enabled(&app, "edit.paste"), "not looked at before a frame");
    frame(&mut app, vec![]);
    assert!(menus::enabled(&app, "edit.paste") && menus::enabled(&app, "edit.pasteInPlace"));
    menus::invoke(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)));
}

#[test]
fn keyboard_paste_of_a_bitmap_without_a_paste_event() {
    let (mut app, board) = app();
    let (cmd, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    frame(&mut app, vec![]);
    // Cmd+V: egui swallows the press and sends no Paste event (no text); only the release arrives.
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 1);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)));
    // A V typed (its press reported) is no paste, whatever is released with it.
    frame(&mut app, vec![key(egui::Key::V, true, none)]);
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 1);
    // Text on the clipboard: egui's Paste event pastes once, its release adds nothing.
    copy_elsewhere(&board, vec![(TEXT, b"words".to_vec())]);
    frame(&mut app, vec![egui::Event::Paste("words".into())]);
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 2);
}

#[test]
fn without_the_service_plain_text_is_still_not_pasted() {
    // The web: SVG text through egui only (see shortcuts' copy_and_paste_events_use_the_system_clipboard).
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![key(egui::Key::V, false, egui::Modifiers::COMMAND)]);
    frame(&mut app, vec![egui::Event::Paste("hello".into())]);
    assert_eq!(count(&app), 0);
}

#[test]
fn a_picture_the_host_read_from_a_paste_pastes_without_the_service() {
    // The web: the page's paste event carries the picture, egui's Paste event no text.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    let ctx = egui::Context::default();
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: blue_png() }, egui::Modifiers::COMMAND);
    let n = &pasted(&app)[0];
    let NodeKind::Image(im) = &n.kind else { panic!("not an image: {:?}", n.kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    assert!((n.geometric_bounds().unwrap().center() - Point::new(250.0, 180.0)).hypot() < 1e-6);
    // Cmd+Shift+V held: Paste in Place, where the picture was put (the view's centre) again.
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: blue_png() }, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(count(&app), 2);
    // A picture that isn't one pastes nothing, the stale internal clipboard neither.
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: b"not a picture".to_vec() }, egui::Modifiers::COMMAND);
    assert_eq!(count(&app), 2);
    assert!(app.ui.status.starts_with("Couldn't paste"), "{}", app.ui.status);
    // A copied SVG file pastes as art.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
    app.paste_from_host(&ctx, Flavour { mime: SVG, data: svg.as_bytes().to_vec() }, egui::Modifiers::COMMAND);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
}
