//! The session's journal of executed commands: what the Actions panel records an action from
//! (and what a replay runs), kept within a memory bound.
//!
//! Entries are numbered from the session's first command on, and a number keeps naming the same
//! entry when older ones are let go: [`Journal::len`] counts every entry ever journaled, so a
//! recording that remembers where it started finds its steps by that number. Only the most recent
//! entries are kept (at most [`Journal::MAX_ENTRIES`] and about [`Journal::MAX_BYTES`], always the
//! last [`Journal::MIN_ENTRIES`]): a long session of select-alls on a large document no longer
//! grows the journal by a copy of every id list forever. A recording that must keep everything
//! from its start holds the journal there ([`Journal::hold`]).

use serde_json::Value;

/// One journaled command: its id and params (with the values it noted, see
/// [`crate::Session::note_journal`]).
pub type JournalEntry = (String, Value);

#[derive(Clone, Debug, Default)]
pub struct Journal {
    /// How many entries were let go from the front; the first kept one has this number.
    dropped: usize,
    entries: Vec<JournalEntry>,
    /// Each kept entry's estimated size, and their sum.
    sizes: Vec<usize>,
    bytes: usize,
    /// Keep every entry from this number on ([`Journal::hold`]).
    held: Option<usize>,
}

/// Journals with the same entries (by number) are equal, whatever their history.
impl PartialEq for Journal {
    fn eq(&self, other: &Self) -> bool {
        self.dropped == other.dropped && self.entries == other.entries
    }
}

impl Journal {
    /// Most entries kept (the oldest go first).
    pub const MAX_ENTRIES: usize = 10_000;
    /// About the most memory the kept entries take.
    pub const MAX_BYTES: usize = 32 << 20;
    /// The most recent entries are kept whatever they take.
    pub const MIN_ENTRIES: usize = 64;

    /// Every entry ever journaled, kept or let go: the number the next one gets.
    pub fn len(&self) -> usize {
        self.dropped + self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The number of the oldest entry kept.
    pub fn first_kept(&self) -> usize {
        self.dropped
    }
    /// The kept entries, oldest first.
    pub fn iter(&self) -> std::slice::Iter<'_, JournalEntry> {
        self.entries.iter()
    }
    /// The kept entries from number `start` on (all kept ones when older ones were let go).
    pub fn since(&self, start: usize) -> &[JournalEntry] {
        self.entries.get(start.saturating_sub(self.dropped)..).unwrap_or_default()
    }
    /// Entry number `i`, if kept.
    pub fn get(&self, i: usize) -> Option<&JournalEntry> {
        self.entries.get(i.checked_sub(self.dropped)?)
    }
    pub fn get_mut(&mut self, i: usize) -> Option<&mut JournalEntry> {
        self.entries.get_mut(i.checked_sub(self.dropped)?)
    }
    pub fn last(&self) -> Option<&JournalEntry> {
        self.entries.last()
    }
    pub fn last_mut(&mut self) -> Option<&mut JournalEntry> {
        self.entries.last_mut()
    }
    pub fn push(&mut self, e: JournalEntry) {
        let size = entry_bytes(&e);
        self.entries.push(e);
        self.sizes.push(size);
        self.bytes += size;
        self.trim();
    }
    /// Drop the entries from number `len` on (a cancelled undo group's).
    pub fn truncate(&mut self, len: usize) {
        let keep = len.saturating_sub(self.dropped);
        self.entries.truncate(keep);
        self.sizes.truncate(keep);
        self.bytes = self.sizes.iter().sum();
        self.dropped = self.dropped.min(len);
    }
    /// Keep every entry from number `from` on (a recording in progress), or stop (`None`).
    pub fn hold(&mut self, from: Option<usize>) {
        self.held = from;
        self.trim();
    }

    /// Let the oldest entries go while over the bounds (in batches: a quarter of the entry bound
    /// at a time, so a push doesn't shift the whole list every time).
    fn trim(&mut self) {
        let over_count = self.entries.len() > Self::MAX_ENTRIES + Self::MAX_ENTRIES / 4;
        if !over_count && self.bytes <= Self::MAX_BYTES {
            return;
        }
        // The entries that may go: not the last few, nor any a hold keeps.
        let mut may = self.entries.len().saturating_sub(Self::MIN_ENTRIES);
        if let Some(h) = self.held {
            may = may.min(h.saturating_sub(self.dropped));
        }
        let mut n = 0;
        let mut bytes = self.bytes;
        while n < may && (self.entries.len() - n > Self::MAX_ENTRIES || bytes > Self::MAX_BYTES) {
            bytes -= self.sizes.get(n).copied().unwrap_or(0);
            n += 1;
        }
        if n > 0 {
            self.entries.drain(..n);
            self.sizes.drain(..n);
            self.bytes = bytes;
            self.dropped += n;
        }
    }
}

impl<'a> IntoIterator for &'a Journal {
    type Item = &'a JournalEntry;
    type IntoIter = std::slice::Iter<'a, JournalEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

/// Roughly the heap an entry takes: its id, and its params as `serde_json` holds them.
fn entry_bytes((id, p): &JournalEntry) -> usize {
    fn json(v: &Value) -> usize {
        const VALUE: usize = std::mem::size_of::<Value>();
        match v {
            Value::String(s) => s.capacity(),
            Value::Array(a) => 16 + a.capacity() * VALUE + a.iter().map(json).sum::<usize>(),
            Value::Object(m) => m.iter().map(|(k, v)| 48 + k.capacity() + VALUE + json(v)).sum(),
            _ => 0,
        }
    }
    std::mem::size_of::<JournalEntry>() + id.capacity() + json(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn big(i: usize) -> JournalEntry {
        ("select.set".into(), json!({"ids": (0..50_000u64).collect::<Vec<_>>(), "n": i}))
    }

    #[test]
    fn numbers_survive_letting_entries_go() {
        let mut j = Journal::default();
        for i in 0..200 {
            j.push(big(i));
        }
        assert_eq!(j.len(), 200, "every entry counts");
        assert!(j.first_kept() > 0, "big entries were let go");
        assert!(j.iter().count() >= Journal::MIN_ENTRIES);
        assert_eq!(j.get(199).map(|e| e.1["n"].clone()), Some(json!(199)));
        assert_eq!(j.since(150).len(), 50);
        assert!(j.get(0).is_none());
    }

    #[test]
    fn a_hold_keeps_a_recording_whole() {
        let mut j = Journal::default();
        j.push(("file.new".into(), json!({})));
        let start = j.len();
        j.hold(Some(start));
        for i in 0..200 {
            j.push(big(i));
        }
        assert_eq!(j.since(start).len(), 200, "nothing recorded was let go");
        assert_eq!(j.first_kept(), 1, "only what came before the recording may go");
        j.hold(None);
        j.push(big(200));
        assert!(j.first_kept() > start);
    }

    #[test]
    fn many_small_entries_are_bounded_too() {
        let mut j = Journal::default();
        for i in 0..30_000 {
            j.push(("object.move".into(), json!({"dx": i})));
        }
        assert_eq!(j.len(), 30_000);
        assert!(j.iter().count() <= Journal::MAX_ENTRIES + Journal::MAX_ENTRIES / 4);
        j.truncate(29_990);
        assert_eq!((j.len(), j.last().map(|e| e.1["dx"].clone())), (29_990, Some(json!(29_989))));
    }
}
