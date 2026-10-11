//! Variables (data merge): definitions with kinds, bindings to objects, datasets
//! applied in one undo step, and validation that rejects instead of reinterpreting.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn text(s: &mut Session, content: &str) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 10, "y": 10, "text": content})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn rect(s: &mut Session) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn plain(s: &Session, id: NodeId) -> String {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

#[test]
fn text_and_visibility_datasets_apply_in_one_undo_step() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Show": false}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)));
    assert_eq!(plain(&s, t), "Bob");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);

    // One undo step covers the whole dataset application.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Alice");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Bob");
}

#[test]
fn dataset_next_and_prev_wrap_around() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    for name in ["A", "B"] {
        s.execute("dataset.new", &json!({"name": name, "values": {"Name": name}})).unwrap();
    }
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "A");
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "B");
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "A");
    s.execute("dataset.prev", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "B");
    let v = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(v["active"], "B");
}

#[test]
fn mismatched_and_missing_bindings_are_skipped_and_counted() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    // A text variable bound to a rectangle applies to nothing; a dataset value for an
    // unbound variable is ignored.
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Ghost": "z"}})).unwrap_err();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y"}})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [r.0]})).unwrap_err();
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
    assert_eq!(plain(&s, t), "y");
}

#[test]
fn deleting_a_variable_cleans_bindings_values_and_leaves_datasets_alone() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(0));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert!(sets["datasets"][0].get("values").is_none_or(|v| v == &json!({})));
    // The dataset named "D" is still the active one: `variable.delete` prunes variables, and
    // the dataset's own name is its own namespace.
    assert_eq!(sets["active"], "D");
    s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(0));
    assert!(sets["active"].is_null());
    let v: Value = serde_json::from_str(&serde_json::to_string(&s.doc().unwrap().doc).unwrap()).unwrap();
    assert!(v["variables"].get("bindings").is_none_or(|v| v == &json!({})));
}

/// A dataset may share a variable's name: they are separate namespaces, and deleting the
/// variable must not silently unselect the dataset that happens to be called the same.
#[test]
fn a_dataset_survives_a_variable_of_the_same_name_being_deleted() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Row", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row", "values": {"Row": "y"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Row"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Row"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["name"], "Row", "the dataset is not a variable");
    assert_eq!(sets["active"], "Row", "the active dataset keeps its name");
}

/// Deleting art lets its bindings go, as it does for Asset Export's assets: a binding that
/// outlived its object would count as a skip in every dataset application and grow the file.
#[test]
fn deleted_art_lets_its_bindings_go() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    let of = |name: &str| vars["variables"].as_array().map(|a| a.iter().find(|v| v["name"] == name).unwrap()["bindings"].as_u64().unwrap()).unwrap();
    assert_eq!(of("Name"), 0, "the deleted text object took its binding with it");
    assert_eq!(of("Show"), 1, "the rectangle is still bound");
    // Nothing stale is left for the application to skip.
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
}

/// `deleted` counts what was deleted, not what was asked for.
#[test]
fn delete_reports_what_it_actually_removed() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "A", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "B", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "D"})).unwrap();
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A"]})).unwrap()["deleted"], 1);
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A"]})).unwrap()["deleted"], 0, "already gone");
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A", "B"]})).unwrap()["deleted"], 1, "only B was left");
    assert_eq!(s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap()["deleted"], 1);
    assert_eq!(s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap()["deleted"], 0);
}

/// An object may be driven by several variables, so binding it again adds to what it has
/// instead of quietly dropping the earlier one.
#[test]
fn one_object_can_be_driven_by_several_variables() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Price", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Badge", "kind": "visibility"})).unwrap();

    // A text object driven by two text variables keeps both.
    assert_eq!(s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap()["bound"], 1);
    let v = s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(1), json!([])), "Title is untouched");
    // A rectangle driven by a visibility variable as well as nothing else yet.
    s.execute("variable.bind", &json!({"variable": "Badge", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Title": "Hat", "Price": "9.99", "Badge": false}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(3), Some(0)), "every binding applied");
    // Both text variables are counted; the later one in definition order is what is left.
    assert_eq!(plain(&s, t), "9.99");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);

    let vars = s.execute("variable.list", &json!({})).unwrap();
    let of = |n: &str| vars["variables"].as_array().unwrap().iter().find(|v| v["name"] == n).unwrap()["bindings"].as_u64();
    assert_eq!((of("Title"), of("Price"), of("Badge")), (Some(1), Some(1), Some(1)));

    // Binding again to one it already has is not a change, and says so.
    let v = s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(0), json!([t.0])));
    // Unbinding one variable leaves the object's others alone.
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Price"})).unwrap()["unbound"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][1]["bindings"], 1, "the other binding is still there");
    assert!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Ghost"})).is_err());

    // Deleting a variable drops only its own binding.
    s.execute("variable.delete", &json!({"names": ["Price"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][0]["bindings"], 1, "Title still drives the text");
}

/// `unbound` counts what was actually dropped, like `variable.delete` does.
#[test]
fn unbind_reports_what_it_actually_removed() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Price", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Title"})).unwrap()["unbound"], 1);
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Title"})).unwrap()["unbound"], 0, "already gone");
    // Without `variable` the object's last binding goes and the entry with it.
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0]})).unwrap()["unbound"], 1);
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0]})).unwrap()["unbound"], 0);
    // An object with no binding left is not kept as an empty set.
    let v: Value = serde_json::from_str(&serde_json::to_string(&s.doc().unwrap().doc).unwrap()).unwrap();
    assert!(v["variables"].get("bindings").is_none_or(|v| v == &json!({})), "{v}");
}

/// Locked art is the document's to rewrite only while it isn't locked.
#[test]
fn locked_art_is_neither_bound_nor_rewritten() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    assert!(s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).is_err(), "locked art takes no new binding");
    // The bindings made before the lock still count, and the application skips the locked art.
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(1)));
    assert_eq!(plain(&s, t), "x", "the locked text was left alone");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible, "the unlocked rectangle still applied");
}

/// Make Text Dynamic / Make Visibility Dynamic: what the panel's buttons run. One step from a
/// selection to a bound variable, named after what it drives.
#[test]
fn making_a_selection_dynamic_names_and_binds_it() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    let v = s.execute("variable.makeTextDynamic", &json!({})).unwrap();
    assert_eq!(v["name"], "Alice", "named after the text it drives");
    assert_eq!(v["ids"], json!([t.0]));

    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let v = s.execute("variable.makeVisibilityDynamic", &json!({})).unwrap();
    assert_eq!(v["kind"], "visibility");

    // The same object twice takes a number rather than colliding with the first variable.
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    let again = s.execute("variable.makeTextDynamic", &json!({})).unwrap();
    assert_eq!(again["name"], "Alice 2");
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(3));

    // Nothing selected, art that can't take the kind, and locked art are errors.
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("variable.makeTextDynamic", &json!({})).is_err());
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    assert!(s.execute("variable.makeTextDynamic", &json!({})).is_err(), "a rectangle is not type");
}

/// Capture Data Set records what the art holds now, which is how rows are made — the art is
/// edited and captured, rather than the values typed.
#[test]
fn capturing_records_what_the_art_holds_now() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();

    let v = s.execute("dataset.capture", &json!({})).unwrap();
    assert_eq!((v["name"].clone(), v["values"].clone()), (json!("Data Set 1"), json!(2)));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"]["Name"], json!({"text": "Alice"}));
    assert_eq!(sets["datasets"][0]["values"]["Show"], json!({"visible": true}));
    assert_eq!(sets["active"], "Data Set 1");

    // Change the art, capture again: a second row, and the first still matches its own text.
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Bob"})).unwrap();
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["matches"], json!(false), "the art moved on");
    s.execute("dataset.capture", &json!({})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][1]["name"], "Data Set 2");
    assert_eq!(sets["datasets"][1]["values"]["Name"], json!({"text": "Bob"}));
    assert_eq!(sets["datasets"][0]["matches"], json!(false), "row 1 still describes the old text");

    // A named capture, and a name that is taken is an error rather than a second row.
    s.execute("dataset.capture", &json!({"name": "Winter"})).unwrap();
    assert!(s.execute("dataset.capture", &json!({"name": "Winter"})).is_err());

    // Nothing bound is nothing to capture.
    let mut empty = session();
    empty.execute("variable.define", &json!({"name": "V", "kind": "text"})).unwrap();
    assert!(empty.execute("dataset.capture", &json!({})).is_err());
}

/// Update Data Set writes the art's current values into the row that is active.
#[test]
fn updating_writes_the_art_into_the_active_row() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();

    // No active row yet: say so rather than quietly writing into some other one.
    assert!(s.execute("dataset.update", &json!({})).is_err());
    s.execute("dataset.capture", &json!({})).unwrap();
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Bob"})).unwrap();
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!((v["name"].clone(), v["values"].clone()), (json!("Data Set 1"), json!(1)));
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["matches"], json!(true));
    // One undo step puts the row back as it was.
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Caz"})).unwrap();
    s.execute("dataset.update", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Caz");
}

/// Renaming carries the variable's bindings and its values in every row with it — a name is
/// how a variable is referred to, so leaving the old one behind would orphan both.
#[test]
fn renaming_carries_the_bindings_and_values_with_it() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "Bob"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "D"})).unwrap();

    s.execute("variable.rename", &json!({"name": "Name", "newName": "Title"})).unwrap();
    assert_eq!(plain(&s, t), "Bob", "the row still applied");
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["values"]["Title"], json!({"text": "Bob"}));
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!((vars["variables"][0]["name"].clone(), vars["variables"][0]["bindings"].clone()), (json!("Title"), json!(1)));

    // Renaming a dataset keeps it active and keeps its values.
    s.execute("dataset.rename", &json!({"name": "D", "newName": "Row 1"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!((sets["active"].clone(), sets["datasets"][0]["name"].clone()), (json!("Row 1"), json!("Row 1")));

    // Taken and missing names are errors.
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
    assert!(s.execute("variable.rename", &json!({"name": "Title", "newName": "Other"})).is_err());
    assert!(s.execute("variable.rename", &json!({"name": "Ghost", "newName": "X"})).is_err());
    assert!(s.execute("dataset.rename", &json!({"name": "Ghost", "newName": "X"})).is_err());
    // A dataset and a variable are separate namespaces: a dataset may take a variable's name,
    // and a variable may take a dataset's.
    s.execute("dataset.rename", &json!({"name": "Row 1", "newName": "Title"})).unwrap();
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["active"], "Title");
    s.execute("dataset.new", &json!({"name": "Taken"})).unwrap();
    assert!(s.execute("dataset.rename", &json!({"name": "Title", "newName": "Taken"})).is_err());
}

/// `dataset.set` is how a value changes after the row exists: replacing it must not need the
/// dataset to be deleted and rebuilt (which loses its place in the list and the active one).
#[test]
fn dataset_set_replaces_the_row_and_reports_what_it_dropped() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    // The dataset keeps its place and stays active while its values change.
    let v = s.execute("dataset.set", &json!({"name": "D", "values": {"Name": "z"}})).unwrap();
    assert_eq!((v["values"].clone(), v["removed"].clone()), (json!(1), json!(["Show"])));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "z"}}));

    s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!(plain(&s, t), "z");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible, "`Show` has no value in this row now");

    // Errors are errors, not a silent rewrite of some other row.
    assert!(s.execute("dataset.set", &json!({"name": "Missing"})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": {"Ghost": "z"}})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": {"Show": "yes"}})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": []})).is_err());
    // One step, so one undo puts the row back as it was.
    s.execute("dataset.set", &json!({"name": "D", "values": {"Name": "w"}})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "z");
}

#[test]
fn bad_variables_requests_are_errors_not_panics() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "N", "kind": "text"})).unwrap();
    for (cmd, params) in [
        ("variable.define", json!({})),
        ("variable.define", json!({"name": "", "kind": "text"})),
        ("variable.define", json!({"name": "N", "kind": "colour"})),
        ("variable.define", json!({"name": "N", "kind": "text"})),
        ("variable.delete", json!({})),
        ("variable.bind", json!({})),
        ("variable.bind", json!({"variable": "N"})),
        ("variable.bind", json!({"variable": "Missing", "ids": [1]})),
        ("dataset.new", json!({})),
        ("dataset.new", json!({"name": "D", "values": []})),
        ("dataset.new", json!({"name": "D", "values": {"N": 5}})),
        ("dataset.select", json!({"name": "Missing"})),
        ("dataset.next", json!({})),
    ] {
        assert!(s.execute(cmd, &params).is_err(), "{cmd} {params}");
    }
}

#[test]
fn highlight_selects_without_unbinding_and_deletes_leave_the_rest() {
    // The reviewer's regression case: two variables, two rows.
    let mut s = session();
    let a = text(&mut s, "Alice");
    let b = text(&mut s, "Bob");
    for (name, id) in [("Name", a), ("Other", b)] {
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
        s.execute("variable.bind", &json!({"variable": name, "ids": [id.0]})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "One", "values": {"Name": "Alice", "Other": "Bob"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Two", "values": {"Name": "A2", "Other": "B2"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "One"})).unwrap();

    // What a row click runs (highlight), then what Select Bound Object runs (select).
    s.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
    let bound: Vec<u64> = s.doc().unwrap().doc.variables.objects_of("Name").iter().map(|i| i.0).collect();
    s.execute("select.set", &json!({"ids": bound})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // …and the binding is still there: selecting never unbinds.
    assert_eq!(s.doc().unwrap().doc.variables.objects_of("Name"), vec![a]);

    // Deleting one variable leaves the other; deleting the current dataset leaves the other.
    s.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().unwrap().len(), 1);
    assert_eq!(vars["variables"][0]["name"], "Other");
    s.execute("dataset.delete", &json!({"names": ["One"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().unwrap().len(), 1);
    assert_eq!(sets["datasets"][0]["name"], "Two");
}

#[test]
fn highlight_follows_rename_and_clears_with_delete() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    assert!(s.execute("variable.highlight", &json!({"variable": "Missing"})).is_err(), "unknown names stay errors");
    let v = s.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
    assert_eq!(v["variable"], "Name");
    // The highlight names a variable, so the rename carries it along.
    s.execute("variable.rename", &json!({"name": "Name", "newName": "Title"})).unwrap();
    assert_eq!(s.doc().unwrap().variables_highlight.as_deref(), Some("Title"));
    // Empty clears it, and deleting the highlighted variable clears it too.
    s.execute("variable.highlight", &json!({})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none());
    s.execute("variable.highlight", &json!({"variable": "Title"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Title"]})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none(), "a highlight of a deleted variable highlights nothing");
}

/// A row that hides an object must be able to show it again, and hiding it on the artboard
/// (as the Layers panel does) is what a capture records: a visibility variable reads and
/// writes its object's own visibility, which therefore never makes the object off limits.
#[test]
fn a_hidden_object_comes_back_and_is_captured_hidden() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0, t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Shown", "values": {"Name": "Alice", "Show": true}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Name": "Bob", "Show": false}})).unwrap();

    s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    let v = s.execute("dataset.select", &json!({"name": "Shown"})).unwrap();
    assert_eq!(v["skipped"], json!(0), "{v}");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible, "the row shows it again");
    assert_eq!(plain(&s, t), "Alice", "and rewrites the text of the object it had hidden");

    // Hidden on the artboard, then captured: the row says hidden, and it matches the art.
    s.execute("select.set", &json!({"ids": [t.0, r.0]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Captured"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let captured = sets["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "Captured").unwrap().clone();
    assert_eq!(captured["values"]["Show"], json!({"visible": false}), "{captured}");
    assert_eq!(captured["matches"], json!(true));

    // A hidden object can still be bound by id.
    s.execute("variable.define", &json!({"name": "More", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "More", "ids": [r.0]})).unwrap();
}

/// Variables and datasets are separate namespaces: renaming a variable leaves a dataset of
/// the same name, and which dataset is active, alone.
#[test]
fn renaming_a_variable_leaves_a_dataset_of_the_same_name_active() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Title"})).unwrap();
    s.execute("variable.rename", &json!({"name": "Title", "newName": "Heading"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!((sets["active"].clone(), sets["datasets"][0]["name"].clone()), (json!("Title"), json!("Title")));
    s.execute("dataset.update", &json!({})).unwrap();
}

/// Without `names`, the deletes act on what the panel shows as current: the highlighted
/// variable and the active dataset (Window › Variables runs them with no params).
#[test]
fn deletes_without_names_take_the_highlight_and_the_active_dataset() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    for name in ["A", "B"] {
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
    }
    s.execute("variable.bind", &json!({"variable": "A", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    assert!(s.execute("variable.delete", &json!({})).is_err(), "nothing highlighted");

    s.execute("variable.highlight", &json!({"variable": "B"})).unwrap();
    assert_eq!(s.execute("variable.delete", &json!({})).unwrap()["deleted"], json!(1));
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(1));
    assert_eq!(vars["variables"][0]["name"], "A");

    assert_eq!(s.execute("dataset.delete", &json!({})).unwrap()["deleted"], json!(1));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["name"], "Data Set 1");
    assert!(s.execute("dataset.delete", &json!({})).is_err(), "no active dataset left");
}

/// Ids and names from an agent are checked, never reinterpreted: an `ids` that isn't a list
/// of objects is an error rather than the selection, and names are trimmed and capped.
#[test]
fn ids_and_names_from_an_agent_are_checked() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "  Name  ", "kind": "text"})).unwrap();
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["variables"][0]["name"], "Name");
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    for params in
        [json!({"variable": "Name", "ids": 5}), json!({"variable": "Name", "ids": [t.0, "x"]}), json!({"variable": "Name", "ids": [999_999]})]
    {
        assert!(s.execute("variable.bind", &params).is_err(), "{params}");
    }
    assert!(s.doc().unwrap().doc.variables.objects_of("Name").is_empty(), "nothing was bound");
    assert!(s.execute("variable.unbind", &json!({"ids": "all"})).is_err());
    assert!(s.execute("variable.define", &json!({"name": "x".repeat(256), "kind": "text"})).is_err());
    s.execute("variable.define", &json!({"name": "x".repeat(255), "kind": "text"})).unwrap();
    assert!(s.execute("variable.delete", &json!({"names": []})).is_err());
    assert!(s.execute("variable.delete", &json!({"names": [5]})).is_err());
}
