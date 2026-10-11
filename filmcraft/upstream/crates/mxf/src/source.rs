//! Random-access byte sources.

use std::io;
use std::sync::Arc;

/// Random-access, read-only byte source (file, in-memory buffer, web Blob…). Same shape as the
/// other FilmCraft demuxers' `ByteSource`, so one adapter serves all of them.
pub trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Fill `buf` entirely from `offset`; `UnexpectedEof` past the end.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    /// True if the source holds no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn eof() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of byte source")
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| eof())?;
        let end = start.checked_add(buf.len()).ok_or_else(eof)?;
        buf.copy_from_slice(self.get(start..end).ok_or_else(eof)?);
        Ok(())
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.as_slice().read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_at(offset, buf)
    }
}

/// Up to `len` bytes at `offset` (fewer at the end of the source).
pub(crate) fn read_upto(src: &(impl ByteSource + ?Sized), offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let n = src.len().saturating_sub(offset).min(len as u64) as usize;
    let mut v = vec![0u8; n];
    src.read_at(offset, &mut v)?;
    Ok(v)
}
