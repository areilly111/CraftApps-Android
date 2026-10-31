//! NEW and OPEN end a command left waiting for input (#715): it belongs to the drawing it started
//! in, so its changes are undone there, and undo in the new drawing never brings back the other.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::Drawing;
use serde_json::json;

fn count(s: &Session) -> usize {
    s.doc().unwrap().entity_count()
}

/// LINE with one segment drawn, still asking for the next point.
fn line_running(s: &mut Session) {
    s.cmdline("line").unwrap();
    s.cmdline("0,0").unwrap();
    s.cmdline("1,1").unwrap();
    assert!(s.running.is_some());
    assert_eq!(count(s), 1);
}

/// Draw a line in the current drawing, undo it and check what is left.
fn line_then_undo(s: &mut Session, title: &str, left: usize) {
    s.cmdline("line").unwrap();
    s.cmdline("0,0").unwrap();
    s.cmdline("5,5").unwrap();
    s.cmdline("").unwrap();
    assert_eq!(count(s), left + 1);
    s.undo().unwrap();
    assert_eq!(s.state().unwrap().title, title);
    assert_eq!(count(s), left, "undo restores this drawing, not the other one");
}

/// The first drawing kept its segment, and its own undo removes it.
fn first_drawing_keeps_its_line(s: &mut Session) {
    s.execute("document.switch", &json!({"index": 0})).unwrap();
    assert_eq!(count(s), 1);
    s.undo().unwrap();
    assert_eq!(count(s), 0);
}

#[test]
fn new_ends_the_command_of_the_previous_drawing() {
    let mut s = Session::new();
    line_running(&mut s);
    s.execute("new", &json!({"metric": true})).unwrap();
    assert!(s.running.is_none(), "LINE ends with its drawing");
    assert_eq!(count(&s), 0);
    line_then_undo(&mut s, "Drawing2", 0);
    first_drawing_keeps_its_line(&mut s);
}

fn two_circles(_: &[u8], _: &str) -> Result<Drawing, String> {
    let mut s = Session::new();
    for x in [0, 10] {
        s.execute("circle", &json!({"center": [x, 0], "radius": 1})).unwrap();
    }
    Ok(s.doc().unwrap().clone())
}

#[test]
fn open_ends_the_command_of_the_previous_drawing() {
    set_io(IoHooks { read: two_circles, write: |_, _| Err("no writer in tests".into()), plot: None });
    let mut s = Session::new();
    line_running(&mut s);
    s.execute("open", &json!({"data": "AA==", "name": "Circles.dxf"})).unwrap();
    assert!(s.running.is_none(), "LINE ends with its drawing");
    assert_eq!(count(&s), 2);
    line_then_undo(&mut s, "Circles.dxf", 2);
    first_drawing_keeps_its_line(&mut s);
}

#[test]
fn new_without_a_command_keeps_the_selection_of_the_previous_drawing() {
    let mut s = Session::new();
    s.execute("circle", &json!({"center": [0, 0], "radius": 1})).unwrap();
    s.execute("selectall", &json!({})).unwrap();
    let selected = s.selection();
    assert_eq!(selected.len(), 1);
    s.execute("new", &json!({"metric": true})).unwrap();
    assert_eq!(s.docs[0].selection, selected);
}
