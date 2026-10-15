//! Opening documents off the UI thread (desktop): a file the user asks to open (File › Open, Open
//! Recent, the Home screen, files handed to the app) is decoded on a thread of its own while the
//! status bar shows it, and opens in a new tab once it is ready, as it would at once: the same
//! notes, recent files and errors. On wasm (no threads) and in unit tests it opens at once
//! ([`available`]).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, ReadFile};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// A file being read.
pub struct Pending {
    /// Where it is.
    pub path: String,
    /// What the status bar shows ("Opening Poster.vectorcraft").
    pub label: String,
    done: Receiver<Result<ReadFile, String>>,
}

/// The files being read.
#[derive(Default)]
pub struct Opening {
    pub pending: Vec<Pending>,
}

/// Does opening happen off the UI thread here? Not on wasm (no threads), in unit tests (they look
/// at the document right after asking for it), or with `VECTORCRAFT_SYNC_OPEN` set.
pub fn available() -> bool {
    !(cfg!(target_arch = "wasm32") || cfg!(test) || std::env::var_os("VECTORCRAFT_SYNC_OPEN").is_some())
}

/// Decode `bytes`, the file at `path`, on a thread of its own → `{background: true, status}`, or
/// the bytes back when no thread could start (the caller opens the file at once).
pub fn start(app: &mut VectorcraftApp, path: &str, bytes: Vec<u8>) -> Result<Value, Vec<u8>> {
    let (tx, done) = channel();
    let bytes = Arc::new(bytes);
    let (name, file) = (path.to_string(), bytes.clone());
    // The automation roots in force here are in force there.
    let scope = ScopeHandle::capture();
    let spawned = std::thread::Builder::new().name("vectorcraft-open".into()).spawn(move || {
        let r = scope
            .run(|| vectorcraft_engine::guard::catch_panic(|| fileio::read_for_open(&name, &file, &Value::Null).map_err(|e| e.to_string())))
            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
        // The file's bytes go before the document is handed over.
        drop(file);
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send(r);
    });
    if spawned.is_err() {
        return Err(Arc::try_unwrap(bytes).unwrap_or_else(|b| b.to_vec()));
    }
    let label = format!("Opening {}", fileio::file_name(path));
    app.opening.pending.push(Pending { path: path.to_string(), label: label.clone(), done });
    let status = format!("{label}…");
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
}

/// Open the files that are ready (each frame).
pub fn poll(app: &mut VectorcraftApp) {
    let mut i = 0;
    while let Some(p) = app.opening.pending.get(i) {
        let r = match p.done.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => Err("the file stopped being read (please report this bug)".to_string()),
        };
        let p = app.opening.pending.remove(i);
        // The status bar no longer says it is opening.
        if app.ui.status == format!("{}…", p.label) {
            app.ui.status.clear();
        }
        finish(app, &p.path, r);
    }
}

/// Open a file read off the UI thread, as [`crate::io::open_reporting`] opens one read at once.
fn finish(app: &mut VectorcraftApp, path: &str, r: Result<ReadFile, String>) {
    let opened = r.and_then(|file| fileio::open_read(&mut app.session, file, Some(path.to_string())).map_err(|e| e.to_string()));
    match opened {
        Ok(r) => {
            crate::io::opened(app, &r);
            crate::io::note_recent(app, path);
        }
        Err(e) => crate::io::report_open_error(app, &fileio::file_name(path), &e),
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;

    #[test]
    fn a_file_read_off_the_ui_thread_opens_as_one_read_at_once() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        let mut d = vectorcraft_doc::Document::new(120.0, 80.0);
        d.title = "Poster".into();
        let bytes = vectorcraft_format::save(&d, false);
        let before = app.session.documents().len();
        let r = start(&mut app, "/art/Poster.vectorcraft", bytes).unwrap();
        assert_eq!(r["background"], true);
        assert_eq!(app.ui.status, "Opening Poster.vectorcraft…");
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), before + 1);
        assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
        let st = app.session.doc().unwrap();
        assert_eq!((st.path.as_deref(), st.doc.artboards[0].rect.width()), (Some("/art/Poster.vectorcraft"), 120.0));
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/art/Poster.vectorcraft"));
        // A file that can't be read: reported, nothing opened, not a recent file.
        start(&mut app, "/art/Broken.vectorcraft", b"{\"format\":\"vectorcraft\"".to_vec()).unwrap();
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), before + 1);
        assert!(app.ui.status.starts_with("Couldn't open Broken.vectorcraft"), "{}", app.ui.status);
        assert_ne!(app.ui.recent_files.first().map(String::as_str), Some("/art/Broken.vectorcraft"));
    }
}
