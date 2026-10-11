//! Selection command results and validation for agent callers (#784).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{EngineError, Session};

fn session() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    let a = rectangle(0);
    let b = rectangle(20);
    (s, a, b)
}

#[test]
fn selection_commands_reject_invalid_ids_without_changing_state() {
    for command in ["select.set", "select.add", "select.toggle"] {
        let (mut s, a, b) = session();
        s.execute("select.set", &json!({"ids": [b.0, a.0]})).unwrap();
        s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
        s.execute("select.key", &json!({"id": b.0})).unwrap();
        for invalid in [json!(999), json!(-1), json!(2.5), json!("2"), Value::Null, json!(true), json!(u64::MAX)] {
            let selection = s.active().unwrap().selection.clone();
            let revision = s.active().unwrap().revision;
            let journal = s.journal.clone();
            let error = s.execute(command, &json!({"ids": [b.0, invalid]})).unwrap_err();
            assert!(matches!(error, EngineError::BadParams { .. }), "{command}: {error}");
            assert!(error.to_string().contains(&invalid.to_string()), "the error must identify {invalid}: {error}");
            assert_eq!(s.active().unwrap().selection, selection, "{command}: {invalid}");
            assert_eq!(s.active().unwrap().revision, revision);
            assert_eq!(s.journal, journal);
        }
    }
}

#[test]
fn selection_commands_require_an_id_array() {
    for command in ["select.set", "select.add", "select.toggle"] {
        let (mut s, _, _) = session();
        for params in [json!({}), json!({"ids": null}), json!({"ids": 2}), json!({"ids": "2"}), json!({"ids": true})] {
            let selection = s.active().unwrap().selection.clone();
            assert!(matches!(s.execute(command, &params), Err(EngineError::BadParams { .. })), "{command}: {params}");
            assert_eq!(s.active().unwrap().selection, selection);
        }
    }
}

#[test]
fn selection_commands_report_the_resulting_selection() {
    let (mut s, a, b) = session();
    assert_eq!(s.execute("select.set", &json!({"ids": [b.0, a.0, b.0]})).unwrap(), json!({"count": 2, "ids": [b.0, a.0]}));
    assert_eq!(s.execute("select.set", &json!({"ids": []})).unwrap(), json!({"count": 0, "ids": []}));
    assert_eq!(s.execute("select.add", &json!({"ids": [a.0, a.0]})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    assert_eq!(s.execute("select.add", &json!({"ids": [b.0]})).unwrap(), json!({"count": 2, "ids": [a.0, b.0]}));
    assert_eq!(s.execute("select.toggle", &json!({"id": a.0})).unwrap(), json!({"count": 1, "ids": [b.0]}));
    assert_eq!(s.execute("select.toggle", &json!({"ids": [b.0, a.0]})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    for command in ["select.add", "select.toggle"] {
        assert_eq!(s.execute(command, &json!({"ids": []})).unwrap(), json!({"count": 1, "ids": [a.0]}));
    }
}

#[test]
fn single_id_toggle_validates_before_mutating() {
    let (mut s, a, _) = session();
    for invalid in [json!(999), json!(-1), json!(2.5), json!("2"), Value::Null, json!(true)] {
        let selection = s.active().unwrap().selection.clone();
        assert!(matches!(s.execute("select.toggle", &json!({"id": invalid})), Err(EngineError::BadParams { .. })));
        assert_eq!(s.active().unwrap().selection, selection);
    }
    // A malformed array must not silently fall back to a valid single id.
    assert!(s.execute("select.toggle", &json!({"ids": "bad", "id": a.0})).is_err());
}
