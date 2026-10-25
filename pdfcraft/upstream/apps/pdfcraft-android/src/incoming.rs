//! Separate, durable inbox for ACTION_VIEW. Never consume a picker's response.
use std::path::Path;

/// Consume one atomically published event. The Java importer stages a private
/// copy first; opening that copy never writes back to the sender's document.
pub(crate) fn poll(bridge: &Path) -> Option<Result<String, String>> {
    let root = bridge.join("incoming");
    let mut events: Vec<_> = std::fs::read_dir(root.join("queue"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| matches!(path.extension().and_then(|e| e.to_str()), Some("open" | "error")))
        .collect();
    events.sort();
    let event = events.first()?;
    let result = (|| {
        let size = std::fs::metadata(event).map_err(|e| e.to_string())?.len();
        if size > 64 * 1024 {
            return Err("Invalid external document event".to_string());
        }
        let value = std::fs::read_to_string(event).map_err(|e| e.to_string())?;
        if event.extension().is_some_and(|ext| ext == "error") {
            return Err(value);
        }
        let document = std::fs::canonicalize(&value).map_err(|e| e.to_string())?;
        let allowed = std::fs::canonicalize(root.join("documents")).map_err(|e| e.to_string())?;
        if !document.starts_with(allowed) || !document.is_file() {
            return Err("External document is outside the private inbox".to_string());
        }
        Ok(value)
    })();
    // A failed removal must not reopen a file on every repaint.
    if let Err(error) = std::fs::remove_file(event) {
        return Some(Err(format!("Couldn't consume external document event: {error}")));
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("pdfcraft-incoming-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(dir.join("incoming/queue")).unwrap();
            std::fs::create_dir_all(dir.join("incoming/documents/request")).unwrap();
            Self(dir)
        }
        fn event(&self, name: &str, value: &str) {
            std::fs::write(self.0.join("incoming/queue").join(name), value).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn opens_private_copy_once_without_consuming_picker_response() {
        let f = Fixture::new();
        let file = f.0.join("incoming/documents/request/document with spaces.pdf");
        std::fs::write(&file, b"synthetic").unwrap();
        std::fs::write(f.0.join("response.json"), b"picker").unwrap();
        f.event("001.open", file.to_str().unwrap());
        assert_eq!(poll(&f.0), Some(Ok(file.to_str().unwrap().to_string())));
        assert!(poll(&f.0).is_none());
        assert!(file.is_file() && f.0.join("response.json").is_file());
    }
    #[test]
    fn ignores_unpublished_events_and_preserves_request_order() {
        let f = Fixture::new();
        f.event("001.error.tmp", "not ready");
        assert!(poll(&f.0).is_none());
        f.event("003.error", "third");
        f.event("002.error", "second");
        assert_eq!(poll(&f.0), Some(Err("second".into())));
        assert_eq!(poll(&f.0), Some(Err("third".into())));
    }
    #[test]
    fn rejects_paths_outside_private_document_inbox() {
        let f = Fixture::new();
        let file = f.0.join("outside.pdf");
        std::fs::write(&file, b"synthetic").unwrap();
        f.event("001.open", file.to_str().unwrap());
        assert!(poll(&f.0).unwrap().is_err());
        assert!(file.is_file());
    }
    #[test]
    fn missing_document_reports_error_once() {
        let f = Fixture::new();
        f.event("001.open", "missing.pdf");
        assert!(poll(&f.0).unwrap().is_err());
        assert!(poll(&f.0).is_none());
    }
}
