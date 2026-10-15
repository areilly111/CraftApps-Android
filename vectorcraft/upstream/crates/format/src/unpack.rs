//! Reading a file: its JSON, then the entries of the data after it (v4) one by one. A compressed
//! file is unpacked as a stream: the JSON into one buffer, each entry straight into its own, so
//! the file is never held unpacked as a whole. Limits apply per part ([`MAX_JSON`], [`MAX_BLOB`],
//! [`MAX_BLOBS`]), guarding against decompression bombs.

use std::io::{self, Read};

use flate2::read::GzDecoder;

use crate::{BLOB_MAGIC, FormatError, MAX_BLOB, MAX_BLOBS, MAX_JSON, MAX_UNPACKED, is_compressed, split_tail};

/// Bytes unpacked at a time while looking for the end of the JSON.
const CHUNK: u64 = 64 << 10;
/// Most bytes reserved for an entry before any of it is unpacked.
const RESERVE_MAX: usize = 64 << 20;

fn damaged(e: impl std::fmt::Display) -> FormatError {
    FormatError::NotVectorcraft(format!("damaged compressed data: {e}"))
}

/// A file opened for reading: its JSON, and where the data after it is.
pub(crate) struct Unpacked<'a> {
    json: Json<'a>,
    tail: Tail<'a>,
}

enum Json<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
    /// Unpacked whole (a compressed file whose JSON isn't followed by the marker where the writer
    /// puts it): `buf[..json]` is the JSON ([`Tail::Whole`] says where the data after it is).
    Whole {
        buf: Vec<u8>,
        json: usize,
    },
}

enum Tail<'a> {
    /// The data after the JSON, in memory (an uncompressed file, or one unpacked whole).
    Slice(&'a [u8]),
    /// In memory, in [`Json::Whole`]'s buffer from this offset.
    Whole(usize),
    /// Still to unpack from `file`: it starts at `at` in the unpacked stream; `read` holds what
    /// was unpacked past the marker already, and `rest` the stream from there.
    Stream {
        file: &'a [u8],
        at: u64,
        read: Vec<u8>,
        rest: Box<GzDecoder<&'a [u8]>>,
    },
    None,
}

impl<'a> Unpacked<'a> {
    /// Open `bytes` (compressed or not).
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if !is_compressed(bytes) {
            let (json, tail) = split_tail(bytes);
            return Ok(Self { json: Json::Borrowed(json), tail: Tail::Slice(tail) });
        }
        let mut dec = GzDecoder::new(bytes);
        let mut buf = Vec::new();
        let mut nul = None;
        loop {
            let start = buf.len();
            let n = (&mut dec).take(CHUNK).read_to_end(&mut buf).map_err(damaged)?;
            if nul.is_none() {
                nul = buf.get(start..).and_then(|b| b.iter().position(|&c| c == 0)).map(|p| p + start);
            }
            if n == 0 || nul.is_some_and(|z| buf.len() >= z + BLOB_MAGIC.len()) {
                break;
            }
            if buf.len() as u64 > MAX_JSON + CHUNK {
                return Err(FormatError::TooBig);
            }
        }
        let Some(z) = nul else {
            // No NUL, so no marker: all of it is JSON.
            if buf.len() as u64 > MAX_JSON {
                return Err(FormatError::TooBig);
            }
            return Ok(Self { json: Json::Owned(buf), tail: Tail::None });
        };
        // JSON holds no raw NUL, so the marker (which has one) starts right before the first NUL;
        // the writer puts it right after the JSON's last byte.
        if let Some(end) = z.checked_sub(1)
            && buf.get(end..).is_some_and(|r| r.starts_with(BLOB_MAGIC))
            && end.checked_sub(1).and_then(|k| buf.get(k)).is_some_and(|b| matches!(b, b'}' | b']' | b'"'))
        {
            if end as u64 > MAX_JSON {
                return Err(FormatError::TooBig);
            }
            let read = buf.split_off(end + BLOB_MAGIC.len());
            buf.truncate(end);
            let at = (end + BLOB_MAGIC.len()) as u64;
            return Ok(Self { json: Json::Owned(buf), tail: Tail::Stream { file: bytes, at, read, rest: Box::new(dec) } });
        }
        // Anything else (hand-made or damaged files) is unpacked whole and split as an
        // uncompressed file is.
        dec.take(MAX_UNPACKED + 1 - (buf.len() as u64).min(MAX_UNPACKED)).read_to_end(&mut buf).map_err(damaged)?;
        if buf.len() as u64 > MAX_UNPACKED {
            return Err(FormatError::TooBig);
        }
        let (json, tail) = split_tail(&buf);
        let (json, tail) = (json.len(), buf.len() - tail.len());
        Ok(Self { json: Json::Whole { buf, json }, tail: Tail::Whole(tail) })
    }

    /// The JSON text.
    pub(crate) fn json(&self) -> &[u8] {
        match &self.json {
            Json::Borrowed(j) => j,
            Json::Owned(j) => j,
            Json::Whole { buf, json, .. } => buf.get(..*json).unwrap_or_default(),
        }
    }

    /// The entries `[offset, length]` of the data after the JSON → each one's bytes, `None` for
    /// one that isn't there (the data ends before it). Errors: the compressed data is damaged, or
    /// the entries are larger than [`MAX_BLOB`] each or [`MAX_BLOBS`] together. Entries are read
    /// in the order of their offsets, each straight into its own buffer.
    pub(crate) fn entries(self, wanted: &[[u64; 2]]) -> Result<Vec<Option<Vec<u8>>>, FormatError> {
        let mut out: Vec<Option<Vec<u8>>> = vec![None; wanted.len()];
        let in_memory = |tail: &[u8], out: &mut Vec<Option<Vec<u8>>>| {
            for (slot, &[offset, len]) in out.iter_mut().zip(wanted) {
                *slot = range(offset, len).and_then(|(a, b)| tail.get(a..b)).map(<[u8]>::to_vec);
            }
        };
        match self.tail {
            Tail::Slice(tail) => in_memory(tail, &mut out),
            Tail::Whole(at) => {
                let buf = match &self.json {
                    Json::Whole { buf, .. } => buf.as_slice(),
                    _ => &[],
                };
                in_memory(buf.get(at..).unwrap_or_default(), &mut out);
            }
            Tail::None => in_memory(&[], &mut out),
            Tail::Stream { file, at, read, rest } => {
                let mut total = 0u64;
                for &[_, len] in wanted {
                    if len > MAX_BLOB {
                        return Err(FormatError::TooBig);
                    }
                    total = total.saturating_add(len);
                }
                if total > MAX_BLOBS {
                    return Err(FormatError::TooBig);
                }
                drop(self.json);
                let mut order: Vec<usize> = (0..wanted.len()).collect();
                order.sort_by_key(|&i| wanted.get(i).map_or(0, |w| w[0]));
                let mut s = Stream { r: Box::new(io::Cursor::new(read).chain(rest)), pos: 0, file, at };
                for i in order {
                    let Some(&[offset, len]) = wanted.get(i) else { continue };
                    if let Some(slot) = out.get_mut(i) {
                        *slot = s.entry(offset, len)?;
                    }
                }
                // To the end, so the stream's checksum is verified as before.
                s.skip(MAX_BLOBS)?;
            }
        }
        Ok(out)
    }
}

/// `offset..offset + len` as indices, when they fit.
fn range(offset: u64, len: u64) -> Option<(usize, usize)> {
    let a = usize::try_from(offset).ok()?;
    Some((a, a.checked_add(usize::try_from(len).ok()?)?))
}

/// The data after a compressed file's JSON, being unpacked.
struct Stream<'a> {
    r: Box<dyn Read + 'a>,
    /// Where `r` is in the data after the JSON.
    pos: u64,
    file: &'a [u8],
    /// Where the data after the JSON starts in the unpacked stream.
    at: u64,
}

impl Stream<'_> {
    /// Unpack and drop up to `n` bytes → how many there were.
    fn skip(&mut self, n: u64) -> Result<u64, FormatError> {
        let skipped = io::copy(&mut (&mut self.r).take(n), &mut io::sink()).map_err(damaged)?;
        self.pos += skipped;
        Ok(skipped)
    }

    /// The entry at `offset`, `len` bytes long (`None`: the data ends before it does).
    fn entry(&mut self, offset: u64, len: u64) -> Result<Option<Vec<u8>>, FormatError> {
        if offset < self.pos {
            // Entries out of order or overlapping (only hand-made files): unpack again from the
            // start.
            let mut dec = GzDecoder::new(self.file);
            let skipped = io::copy(&mut (&mut dec).take(self.at), &mut io::sink()).map_err(damaged)?;
            if skipped < self.at {
                return Ok(None);
            }
            self.r = Box::new(dec);
            self.pos = 0;
        }
        let gap = offset - self.pos;
        if self.skip(gap)? < gap {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        let n = usize::try_from(len).map_err(|_| FormatError::TooBig)?;
        // The length is the file's word: reserve up to a bound, and let real data grow the rest.
        bytes.try_reserve_exact(n.min(RESERVE_MAX)).map_err(|_| FormatError::TooBig)?;
        let got = (&mut self.r).take(len).read_to_end(&mut bytes).map_err(damaged)?;
        self.pos += got as u64;
        Ok((got as u64 == len).then_some(bytes))
    }
}
