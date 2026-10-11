//! A written file's indirect objects read for rewriting it (the linearisation and the page
//! thumbnails), on the lexer of [`crate::encrypt`]: where each object's parts lie, what it refers
//! to, and the references to renumber.

use crate::encrypt::{Lexer, Obj, Tok, int, stream_spans};

impl Obj<'_> {
    /// Every object this one refers to, in file order, leaving out the values of the dictionary
    /// keys in `skip` (`Parent`).
    pub(crate) fn refs(&self, skip: &[&[u8]], out: &mut Vec<u32>) {
        match self {
            Self::Ref(n, _) => out.push(*n),
            Self::Dict { entries, .. } => entries.iter().filter(|(k, _)| !skip.contains(k)).for_each(|(_, v)| v.refs(skip, out)),
            Self::Array(items) => items.iter().for_each(|v| v.refs(skip, out)),
            _ => {}
        }
    }
}

/// An indirect object of a file, with where its parts lie.
pub(crate) struct Indirect<'a> {
    pub num: u32,
    pub generation: u16,
    pub value: Obj<'a>,
    /// Where the value (a stream's dictionary) starts and ends.
    pub value_span: (usize, usize),
    /// Where `endobj` starts.
    pub endobj: usize,
}

/// The indirect object at `off` of `pdf` (a stream's length must be direct); `None` when it can't
/// be read.
pub(crate) fn indirect(pdf: &[u8], off: usize) -> Option<Indirect<'_>> {
    let mut lx = Lexer::at(pdf, off);
    let (num, generation) = lx.header()?;
    let start = lx.clone().next()?.1;
    let value = lx.object(0)?;
    // The value ends before the white space ahead of the next token (`stream` or `endobj`).
    let (next, next_at, _) = lx.clone().next()?;
    let end = next_at - pdf.get(..next_at)?.iter().rev().take_while(|b| b.is_ascii_whitespace()).count();
    let endobj = match next {
        Tok::Word(b"endobj") => next_at,
        _ => {
            let (_, (_, data_end)) = stream_spans(pdf, &mut lx, &value).ok()??;
            let mut lx = Lexer::at(pdf, data_end);
            let (Tok::Word(b"endstream"), ..) = lx.next()? else { return None };
            let (Tok::Word(b"endobj"), at, _) = lx.next()? else { return None };
            at
        }
    };
    Some(Indirect { num, generation, value, value_span: (start, end.max(start)), endobj })
}

/// The references (`n g R`) in `pdf[start..end]` (an object's value: no stream data), outside
/// strings: span and object number.
pub(crate) fn ref_spans(pdf: &[u8], start: usize, end: usize) -> Vec<(usize, usize, u32)> {
    let mut out = vec![];
    let mut lx = Lexer::at(pdf, start);
    // The last two tokens when they are numbers: (start, value).
    let mut last: [Option<(usize, i64)>; 2] = [None, None];
    while let Some((tok, s, e)) = lx.next().filter(|t| t.1 < end) {
        match tok {
            Tok::Word(b"R") => {
                if let [Some((at, n)), Some(_)] = last
                    && let Ok(n) = u32::try_from(n)
                {
                    out.push((at, e, n));
                }
                last = [None, None];
            }
            Tok::Word(w) => last = [last[1], int(w).map(|v| (s, v))],
            _ => last = [None, None],
        }
    }
    out
}
