//! File dialogs shown off the UI thread (Linux, #592): what asked runs again with the answer.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use serde_json::json;
use vectorcraft_engine::Session;

use crate::picks::{self, PickRequest};
use crate::state::Dialog;
use crate::{FilePick, Services, VectorcraftApp};

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="10" height="10"/></svg>"#;

/// A dialog asked for, and where its answer goes.
type Asked = (PickRequest, Sender<Vec<String>>);

/// The dialogs showing.
#[derive(Clone, Default)]
struct Shown(Rc<RefCell<Vec<Asked>>>);

impl Shown {
    fn requests(&self) -> Vec<PickRequest> {
        self.0.borrow().iter().map(|(r, _)| r.clone()).collect()
    }
    /// The last dialog shown answers `paths`.
    fn answer(&self, paths: &[&str]) {
        let (_, tx) = self.0.borrow_mut().pop().expect("a dialog is showing");
        tx.send(paths.iter().map(|p| p.to_string()).collect()).unwrap();
    }
}

/// A desktop app whose dialogs show off the UI thread (none in line), reading [`SVG`] from any path
/// and writing files to `written`.
fn app() -> (VectorcraftApp, Shown, Rc<RefCell<Vec<String>>>) {
    let (shown, written) = (Shown::default(), Rc::new(RefCell::new(vec![])));
    let (s, w) = (shown.clone(), written.clone());
    let in_line = |_: &FilePick| -> Option<String> { panic!("no dialog on the UI thread") };
    let services = Services {
        pick_open: Some(Box::new(in_line)),
        pick_save: Some(Box::new(in_line)),
        pick_folder: Some(Box::new(|| panic!("no dialog on the UI thread"))),
        read: Some(Box::new(|_: &str| Ok(SVG.to_vec()))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        start_pick: Some(Box::new(move |request: PickRequest| {
            let (tx, rx) = channel();
            s.0.borrow_mut().push((request, tx));
            Some(rx)
        })),
        ..Default::default()
    };
    (VectorcraftApp::new(Session::new(), services), shown, written)
}

fn poll(app: &mut VectorcraftApp) {
    picks::poll(app, &egui::Context::default());
}

#[test]
fn file_open_opens_what_its_dialog_picks_when_it_answers() {
    let (mut app, shown, _) = app();
    // The dialog shows; File › Open gives up for now, without a "cancelled".
    assert!(app.run("file.open", json!({})).is_err());
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    poll(&mut app);
    assert!(app.session.documents().is_empty() && app.ui.status.is_empty(), "{}", app.ui.status);
    // Another dialog waits for this one.
    assert!(app.run("file.open", json!({})).is_err());
    assert_eq!(shown.requests().len(), 1);
    assert!(app.ui.status.contains("file dialog is open"), "{}", app.ui.status);
    shown.answer(&["/art/a.svg"]);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some("/art/a.svg"));
    // Cancelled: nothing happens, and dialogs show again.
    assert!(app.run("file.open", json!({})).is_err());
    shown.answer(&[]);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.run("file.open", json!({})).is_err());
    assert_eq!(shown.requests().len(), 1);
}

#[test]
fn save_as_goes_on_with_the_path_picked() {
    let (mut app, shown, written) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    assert!(app.run("file.saveAs", json!({})).is_err());
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(_)]));
    shown.answer(&["/art/poster.svg"]);
    poll(&mut app);
    // SVG Options asks next, and its OK writes the file there.
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("svgOptions"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(*written.borrow(), ["/art/poster.svg"]);
}

#[test]
fn a_dialogs_folder_button_fills_its_field_when_the_picker_answers() {
    let (mut app, shown, _) = app();
    let mut d = Dialog::new("exportForScreens", json!({"folder": ""}));
    app.ui.dialog = Some(d.clone());
    picks::folder_field(&mut app, &mut d, "folder");
    assert_eq!(shown.requests(), [PickRequest::Folder]);
    shown.answer(&["/out"]);
    poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("folder")).as_deref(), Some("/out"));
}
