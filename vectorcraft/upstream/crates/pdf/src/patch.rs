//! Edits to a PDF the export wrote (uncompressed objects, one cross-reference table starting at
//! object 0): bytes replaced or inserted inside objects, new objects and trailer entries. The
//! cross-reference table is written again to match.

use crate::PdfError;
use crate::lab_spot::{find, rfind, xref_offset};

/// The cross-reference table of a written PDF and its trailer.
pub(crate) struct Xref {
    /// Where the table (`xref`) starts.
    at: usize,
    /// Where its first 20-byte entry starts.
    entries: usize,
    /// Number of entries (the trailer's `/Size`).
    count: usize,
    /// The trailer dictionary, `<<` to `>>` included.
    trailer: (usize, usize),
}

/// Most bytes of a number in PDF syntax read here.
const MAX_DIGITS: usize = 10;

/// The unsigned number `pdf` holds at `at` (after white space), and where it ends.
pub(crate) fn number(pdf: &[u8], mut at: usize) -> Option<(usize, usize)> {
    while pdf.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let digits = pdf.get(at..)?.iter().take(MAX_DIGITS + 1).take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > MAX_DIGITS {
        return None;
    }
    Some((std::str::from_utf8(pdf.get(at..at + digits)?).ok()?.parse().ok()?, at + digits))
}

impl Xref {
    /// The table `startxref` points at; `None` for files written another way.
    pub fn read(pdf: &[u8]) -> Option<Self> {
        let at = xref_offset(pdf)?;
        let (first, after) = number(pdf, at + 4)?;
        let (count, after) = number(pdf, after)?;
        let entries = find(pdf, b"\n", after)? + 1;
        let table_end = entries.checked_add(count.checked_mul(20)?)?;
        if first != 0 || count == 0 || table_end > pdf.len() {
            return None;
        }
        let start = find(pdf, b"<<", find(pdf, b"trailer", table_end)?)?;
        let end = rfind(pdf.get(start..rfind(pdf, b"startxref")?)?, b">>")? + start + 2;
        Some(Self { at, entries, count, trailer: (start, end) })
    }

    /// Number of entries (objects `0..count`).
    pub fn count(&self) -> usize {
        self.count
    }

    /// Where in-use object `n` starts.
    pub fn offset(&self, pdf: &[u8], n: u32) -> Option<usize> {
        let e = self.entries + 20 * usize::try_from(n).ok().filter(|n| *n < self.count)?;
        let entry = pdf.get(e..e + 20)?;
        (entry.get(17) == Some(&b'n')).then_some(())?;
        number(entry, 0).map(|(off, _)| off)
    }

    /// The in-use objects: number and offset, in number order.
    pub fn objects(&self, pdf: &[u8]) -> Vec<(u32, usize)> {
        (1..u32::try_from(self.count).unwrap_or(u32::MAX)).filter_map(|n| Some((n, self.offset(pdf, n)?))).collect()
    }

    /// Where the table (`xref`) starts.
    pub fn at(&self) -> usize {
        self.at
    }

    /// Where the trailer dictionary starts (its `<<`).
    pub fn trailer_at(&self) -> usize {
        self.trailer.0
    }

    /// The object the trailer's `key` (`b"/Root"`) refers to.
    pub fn trailer_ref(&self, pdf: &[u8], key: &[u8]) -> Option<u32> {
        let dict = pdf.get(self.trailer.0..self.trailer.1)?;
        let at = find(dict, key, 0)? + key.len();
        // A key that only starts with `key` (`/Info` in `/InfoX`) isn't it.
        if dict.get(at).is_some_and(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        u32::try_from(number(dict, at)?.0).ok()
    }

    /// The contents of the dictionary object `n` is (just after its `<<`, up to its `endobj`).
    pub fn dict(&self, pdf: &[u8], n: u32) -> Option<(usize, usize)> {
        let off = self.offset(pdf, n)?;
        let (num, after) = number(pdf, off)?;
        let head = pdf.get(after..after + 6)?;
        if num != n as usize || head != b" 0 obj" {
            return None;
        }
        let mut at = after + 6;
        while pdf.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        (pdf.get(at..at + 2)? == b"<<").then_some(())?;
        Some((at + 2, find(pdf, b"endobj", at)?))
    }
}

/// A replacement of bytes `start..end` of the file.
type Edit = (usize, usize, Vec<u8>);

/// The edits of one PDF, applied together by [`Self::apply`].
pub(crate) struct Patch {
    edits: Vec<Edit>,
    /// New objects' bodies, numbered from the table's size on.
    objects: Vec<Vec<u8>>,
    /// Entries added to the trailer dictionary.
    trailer: Vec<u8>,
    size: usize,
}

impl Patch {
    pub fn new(xref: &Xref) -> Self {
        Self { edits: vec![], objects: vec![], trailer: vec![], size: xref.count }
    }

    /// Replace bytes `start..end` (inside the objects) with `with`; `start == end` inserts.
    pub fn replace(&mut self, start: usize, end: usize, with: Vec<u8>) {
        self.edits.push((start, end, with));
    }

    /// Add an object of `body` (a dictionary, array or stream with its dictionary) → its number.
    pub fn add_object(&mut self, body: Vec<u8>) -> u32 {
        self.objects.push(body);
        u32::try_from(self.size + self.objects.len() - 1).unwrap_or(u32::MAX)
    }

    /// Add `entry` (`/Info 12 0 R`) to the trailer dictionary.
    pub fn trailer_entry(&mut self, entry: &[u8]) {
        self.trailer.extend_from_slice(entry);
    }

    /// `pdf` with the edits, new objects and trailer entries, and a cross-reference table that
    /// matches. Refused when an edit overlaps another or reaches past the objects.
    pub fn apply(mut self, pdf: &[u8], xref: &Xref) -> Result<Vec<u8>, PdfError> {
        let bad = |why: &str| PdfError::Write(format!("can't edit the written PDF: {why}"));
        self.edits.sort_by_key(|e| (e.0, e.1));
        if self.edits.windows(2).any(|w| w[0].1 > w[1].0) || self.edits.iter().any(|e| e.0 > e.1 || e.1 > xref.at) {
            return Err(bad("edits overlap"));
        }
        // Where a byte offset of the old file lands in the new one.
        let edits = &self.edits;
        let moved = |off: usize| -> usize {
            edits.iter().filter(|e| e.1 <= off).fold(off as isize, |o, e| o + e.2.len() as isize - (e.1 - e.0) as isize) as usize
        };
        let added: usize = self.edits.iter().map(|e| e.2.len()).sum::<usize>() + self.objects.iter().map(|o| o.len() + 32).sum::<usize>();
        let mut out = Vec::with_capacity(pdf.len() + added + 64);
        let mut at = 0;
        for (start, end, new) in edits.iter() {
            out.extend_from_slice(pdf.get(at..*start).ok_or_else(|| bad("bad edit"))?);
            out.extend_from_slice(new);
            at = *end;
        }
        out.extend_from_slice(pdf.get(at..xref.at).ok_or_else(|| bad("bad edit"))?);
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", self.size + i).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        // The table: the old entries moved (free ones as they are), then the new objects'.
        let table = out.len();
        let size = self.size + self.objects.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        let old = pdf.get(xref.entries..xref.entries + 20 * xref.count).ok_or_else(|| bad("bad cross-reference table"))?;
        for e in old.chunks(20) {
            match number(e, 0) {
                Some((off, _)) if e.get(17) == Some(&b'n') => {
                    out.extend_from_slice(format!("{:010}", moved(off)).as_bytes());
                    out.extend_from_slice(e.get(10..).unwrap_or_default());
                }
                _ => out.extend_from_slice(e),
            }
        }
        let eol = old.get(18..20).unwrap_or(b"\r\n");
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n").as_bytes());
            out.extend_from_slice(eol);
        }
        // The trailer, with its size and the new entries.
        let (ts, te) = xref.trailer;
        let dict = pdf.get(ts..te.saturating_sub(2)).ok_or_else(|| bad("bad trailer"))?;
        let size_at = find(dict, b"/Size", 0).ok_or_else(|| bad("the trailer has no size"))? + 5;
        let (_, size_end) = number(dict, size_at).ok_or_else(|| bad("the trailer has no size"))?;
        let (Some(head), Some(tail)) = (dict.get(..size_at), dict.get(size_end..)) else { return Err(bad("bad trailer")) };
        out.extend_from_slice(b"trailer\n");
        out.extend_from_slice(head);
        out.extend_from_slice(format!(" {size}").as_bytes());
        out.extend_from_slice(tail);
        out.extend_from_slice(&self.trailer);
        out.extend_from_slice(format!(">>\nstartxref\n{table}\n%%EOF").as_bytes());
        Ok(out)
    }
}
