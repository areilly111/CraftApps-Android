//! DRAWINGRECOVERY: list, open and delete the autosave files a stopped app left behind, and never
//! touch a file that isn't one of them. Its own test binary: SAVEFILEPATH is process-wide.

#![cfg(not(target_arch = "wasm32"))]

use cadcraft_engine::Session;
use cadcraft_engine::autosave;
use serde_json::json;

#[test]
fn lists_and_discards_only_recoverable_autosave_files() {
    let dir = std::env::temp_dir().join(format!("cadcraft-drawingrecovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    autosave::set_savefilepath(&dir.to_string_lossy());

    // A file from an app that stopped (no lock file), and one that isn't an autosave file.
    let left = dir.join(autosave::file_name("Bracket", "123-456", 7));
    std::fs::write(&left, b"0\nEOF\n").unwrap();
    let other = dir.join("notes.txt");
    std::fs::write(&other, b"keep me").unwrap();

    let mut s = Session::new();
    let r = s.execute("drawingrecovery", &json!({})).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["drawing"], "Bracket");

    // Unlisted paths are refused, whatever they are.
    for path in [other.to_string_lossy().to_string(), "/etc/passwd".to_string()] {
        assert!(s.execute("drawingrecovery", &json!({ "action": "discard", "path": path })).is_err());
        assert!(s.execute("drawingrecovery", &json!({ "action": "open", "path": path })).is_err());
    }
    assert!(other.exists());
    assert!(s.execute("drawingrecovery", &json!({ "action": "frobnicate" })).is_err());

    let r = s.execute("drawingrecovery", &json!({ "action": "discard", "path": left.to_string_lossy() })).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(0));
    assert!(!left.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
