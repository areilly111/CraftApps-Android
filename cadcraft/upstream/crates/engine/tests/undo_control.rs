//! UNDO Mark/Back, BEgin/End groups, Auto and Control (All/None/One/Combine/Layer), UNDOCTL and
//! UNDOMARKS, REDO and MREDO counts: JSON form, command line and scripts.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::{Value, json};

fn circles(s: &Session) -> usize {
    s.doc().unwrap().model.len()
}

fn circle(s: &mut Session, x: f64) {
    s.execute("circle", &json!({ "center": [x, 0], "radius": 1 })).unwrap();
}

fn getvar(s: &mut Session, name: &str) -> Value {
    s.execute("getvar", &json!({ "name": name })).unwrap()["value"].clone()
}

#[test]
fn mark_and_back_undo_to_the_mark() {
    let mut s = Session::new();
    let base = circles(&s);
    circle(&mut s, 0.0);
    s.execute("undo", &json!({ "mode": "mark" })).unwrap();
    assert_eq!(getvar(&mut s, "UNDOMARKS"), json!(1));
    circle(&mut s, 2.0);
    circle(&mut s, 4.0);
    let r = s.execute("undo", &json!({ "mode": "back" })).unwrap();
    assert_eq!(r["undone"].as_array().unwrap().len(), 2);
    assert_eq!(circles(&s), base + 1, "back to the mark");
    assert_eq!(getvar(&mut s, "UNDOMARKS"), json!(0), "the mark goes with Back");
    // No mark left: Back asks first; refusing keeps the drawing.
    s.execute("undo", &json!({ "mode": "back", "confirm": false })).unwrap();
    assert_eq!(circles(&s), base + 1);
    s.execute("undo", &json!({ "mode": "back" })).unwrap();
    assert_eq!(circles(&s), base, "no mark: everything undone");
}

#[test]
fn back_at_the_command_line_and_marks_undone_away() {
    let mut s = Session::new();
    let base = circles(&s);
    s.cmdline("undo m").unwrap();
    circle(&mut s, 0.0);
    s.cmdline("undo m").unwrap();
    circle(&mut s, 2.0);
    // Undoing past the second mark (then drawing again) drops it; the first stays.
    s.execute("u", &json!({})).unwrap();
    s.execute("u", &json!({})).unwrap();
    circle(&mut s, 3.0);
    assert_eq!(getvar(&mut s, "UNDOMARKS"), json!(1));
    circle(&mut s, 5.0);
    s.cmdline("undo b").unwrap();
    assert_eq!(circles(&s), base, "back to the first mark");
    // Without a mark the command line asks; No keeps everything.
    circle(&mut s, 0.0);
    s.cmdline("undo b").unwrap();
    assert!(s.running.is_some(), "asks before undoing everything");
    s.cmdline("n").unwrap();
    assert_eq!(circles(&s), base + 1);
    s.cmdline("undo b y").unwrap();
    assert_eq!(circles(&s), base);
}

#[test]
fn begin_end_group_is_one_undo_step() {
    let mut s = Session::new();
    let base = circles(&s);
    let depth = s.state().unwrap().undo.len();
    s.execute("undo", &json!({ "mode": "begin" })).unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 8, 8, "group open");
    for x in 0..3 {
        circle(&mut s, f64::from(x) * 2.0);
    }
    s.execute("undo", &json!({ "mode": "end" })).unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 8, 0);
    assert_eq!(s.state().unwrap().undo.len(), depth + 1, "one step");
    let r = s.execute("u", &json!({})).unwrap();
    assert_eq!(r["undone"], json!(["Group"]));
    assert_eq!(circles(&s), base);
    s.execute("redo", &json!({})).unwrap();
    assert_eq!(circles(&s), base + 3);
    // End without BEgin does nothing.
    s.execute("undo", &json!({ "mode": "end" })).unwrap();
    assert_eq!(circles(&s), base + 3);
}

#[test]
fn group_from_a_script_and_nested_begin() {
    let mut s = Session::new();
    let base = circles(&s);
    let depth = s.state().unwrap().undo.len();
    s.script("UNDO BE\nCIRCLE 0,0 1\nUNDO BE\nCIRCLE 3,0 1\nCIRCLE 6,0 1\nUNDO E\n").unwrap();
    // BEgin inside a group ends that group and starts a new one.
    assert_eq!(s.state().unwrap().undo.len(), depth + 2);
    s.cmdline("undo 1").unwrap();
    assert_eq!(circles(&s), base + 1);
    // UNDO inside an open group undoes the whole group so far.
    s.cmdline("undo be").unwrap();
    circle(&mut s, 9.0);
    circle(&mut s, 12.0);
    s.cmdline("u").unwrap();
    assert_eq!(circles(&s), base + 1);
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 8, 0, "the group was ended");
}

#[test]
fn saving_closes_an_open_group() {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write: |_, _| Ok(b"0\nEOF\n".to_vec()), plot: None });
    let mut s = Session::new();
    let depth = s.state().unwrap().undo.len();
    s.execute("undo", &json!({ "mode": "begin" })).unwrap();
    circle(&mut s, 0.0);
    circle(&mut s, 2.0);
    let path = std::env::temp_dir().join(format!("cadcraft-undo-group-{}.dxf", std::process::id()));
    s.execute("saveas", &json!({ "path": path.to_string_lossy() })).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 8, 0, "saving ends the group");
    assert_eq!(s.state().unwrap().undo.len(), depth + 1);
}

#[test]
fn control_none_and_one() {
    let mut s = Session::new();
    circle(&mut s, 0.0);
    s.execute("undo", &json!({ "mode": "control", "control": "none" })).unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 1, 0);
    assert!(s.state().unwrap().undo.is_empty(), "None discards undo information");
    circle(&mut s, 2.0);
    assert!(s.state().unwrap().undo.is_empty(), "nothing recorded");
    assert!(s.execute("u", &json!({})).is_err(), "U says undo is off");
    assert!(s.execute("undo", &json!({ "mode": "mark" })).is_err());
    // With undo off the command line asks for the control option straight away.
    s.cmdline("undo one").unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 3, 3);
    let before = circles(&s);
    circle(&mut s, 4.0);
    circle(&mut s, 6.0);
    assert_eq!(s.state().unwrap().undo.len(), 1, "One keeps a single step");
    s.cmdline("undo 5").unwrap();
    assert_eq!(circles(&s), before + 1);
    s.cmdline("undo c a").unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL").as_i64().unwrap() & 3, 1);
    circle(&mut s, 8.0);
    circle(&mut s, 10.0);
    assert_eq!(s.state().unwrap().undo.len(), 2);
}

#[test]
fn auto_combine_and_layer_set_undoctl() {
    let mut s = Session::new();
    assert_eq!(getvar(&mut s, "UNDOCTL"), json!(53));
    s.cmdline("undo a off").unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL"), json!(49));
    s.execute("undo", &json!({ "mode": "auto", "on": true })).unwrap();
    s.cmdline("undo c c n").unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL"), json!(37));
    s.execute("undo", &json!({ "mode": "control", "control": "layer", "on": false })).unwrap();
    assert_eq!(getvar(&mut s, "UNDOCTL"), json!(5));
    // Read-only.
    assert!(s.execute("setvar", &json!({ "name": "UNDOCTL", "value": 1 })).is_err());
    assert!(s.execute("setvar", &json!({ "name": "UNDOMARKS", "value": 1 })).is_err());
    assert!(s.execute("undo", &json!({ "mode": "sideways" })).is_err());
    assert!(s.execute("undo", &json!({ "mode": "auto" })).is_err(), "auto needs on");
}

#[test]
fn redo_and_mredo_counts() {
    let mut s = Session::new();
    let base = circles(&s);
    for x in 0..4 {
        circle(&mut s, f64::from(x) * 2.0);
    }
    s.execute("undo", &json!({ "count": 4 })).unwrap();
    assert_eq!(circles(&s), base);
    s.execute("redo", &json!({ "count": 2 })).unwrap();
    assert_eq!(circles(&s), base + 2);
    s.execute("mredo", &json!({ "all": true })).unwrap();
    assert_eq!(circles(&s), base + 4);
    s.cmdline("undo 3").unwrap();
    s.cmdline("mredo 2").unwrap();
    assert_eq!(circles(&s), base + 3);
    s.cmdline("undo 3").unwrap();
    s.cmdline("mredo l").unwrap();
    assert_eq!(circles(&s), base + 1);
    s.cmdline("mredo a").unwrap();
    assert_eq!(circles(&s), base + 4);
    assert!(s.cmdline("mredo -1").is_ok(), "a bad count re-prompts");
    s.cancel();
}

#[test]
fn hostile_undo_params_never_panic() {
    let mut s = Session::new();
    circle(&mut s, 0.0);
    for p in [
        json!({ "count": -1 }),
        json!({ "count": u64::MAX }),
        json!({ "mode": 7 }),
        json!({ "mode": "control", "control": 3 }),
        json!({ "mode": "back", "confirm": "maybe" }),
        json!({ "mode": "mark" }),
    ] {
        let _ = s.execute("undo", &p);
    }
    for p in [json!({ "count": u64::MAX }), json!({ "all": "yes" }), json!(null)] {
        let _ = s.execute("mredo", &p);
    }
    // Many marks stay bounded.
    for _ in 0..1100 {
        s.execute("undo", &json!({ "mode": "mark" })).unwrap();
    }
    assert!(getvar(&mut s, "UNDOMARKS").as_u64().unwrap() <= 1000);
}
