//! "Data not here yet" signalling for asynchronous byte sources (web Blob reads).
//!
//! On the web, media bytes are read from a `File`/`Blob` asynchronously. A [`ByteReader`] that
//! does not have the requested range yet starts fetching it, calls [`mark`] and fails the read
//! with [`std::io::ErrorKind::WouldBlock`]. Whoever asked for the frame (the frame server, an
//! import) checks [`take`] afterwards: when set, the result is incomplete and the request should
//! be retried once the data has arrived, instead of being cached as a decode error or offline
//! media. The flag travels through a thread-local (like [`crate::cancel`]), so the
//! [`MediaSource`](crate::MediaSource) API is unchanged. Native file reads never set it.
//!
//! [`ByteReader`]: crate::reader::ByteReader

use std::cell::Cell;

thread_local! {
    static PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Record that a read on this thread could not be served yet (its data is being fetched).
pub fn mark() {
    PENDING.with(|p| p.set(true));
}

/// Whether a read on this thread was deferred since the last [`take`].
pub fn is_set() -> bool {
    PENDING.with(Cell::get)
}

/// Return and clear the flag.
pub fn take() -> bool {
    PENDING.with(|p| p.replace(false))
}

/// The error a deferring reader returns.
pub fn would_block() -> std::io::Error {
    mark();
    std::io::Error::new(std::io::ErrorKind::WouldBlock, "media data is still loading")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_clears_the_flag() {
        assert!(!take());
        let e = would_block();
        assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock);
        assert!(is_set());
        assert!(take());
        assert!(!is_set());
    }
}
