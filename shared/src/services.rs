//! Minimal shared file-writing helper.
//!
//! The per-app `services.rs` in each port owns the app-specific `Services`
//! struct; this module only provides the crash-safe atomic write that the
//! shared [`crate::storage`] needs for plain filesystem paths.

use std::path::Path;

/// Write `bytes` to `path` crash-safely: write to a temp file beside the
/// target, fsync it, then atomically rename over the target. Best-effort
/// directory fsync follows. Mirrors the desktop's `write_atomic` contract;
/// returns `Err(String)` for the UI on any failure.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let parent = path.parent().ok_or_else(|| format!("no parent dir for {}", path.display()))?;
    // Unique temp name beside the target so the rename stays on one filesystem.
    let nonce: u64 = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::time::SystemTime::now().hash(&mut h);
        std::thread::current().id().hash(&mut h);
        h.finish()
    };
    let tmp = parent.join(format!(".tmp-{nonce:016x}"));
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    f.write_all(bytes).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    f.sync_all().map_err(|e| format!("couldn't fsync {}: {e}", tmp.display()))?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| format!("couldn't replace {}: {e}", path.display()))?;
    // Best effort: fsync the directory so the rename itself is durable.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}
