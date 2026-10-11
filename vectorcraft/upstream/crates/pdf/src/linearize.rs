//! Save PDF › Optimize for fast web view: a linearised file (ISO 32000-1 Annex F), whose first page
//! shows while the rest of it downloads.
//!
//! [`prepare`] writes the export's file (one cross-reference table) again with its objects in
//! linearised order and numbering. First, with the highest numbers: the linearization dictionary,
//! the catalog with what opening the document needs, the hint stream and the first page (its page
//! object, then every object it uses, numbered on from it). Then, numbered from 1: each other page
//! with the objects only it uses, the objects several pages share, and the rest (the page tree,
//! document information, metadata, editing data, thumbnails). Encryption runs on that file: it
//! keeps the order and the numbers and adds its dictionary as the last object. [`write`] then lays
//! the file out: the linearization dictionary, the first page's cross-reference table and trailer,
//! the catalog part (with the encryption dictionary), the hint stream (the page offset and shared
//! object hint tables, encrypted like any stream), the first page, the other pages, the shared
//! objects, the rest and the main cross-reference table.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::PdfError;
use crate::encrypt::Cipher;
use crate::encrypt::{Lexer, Obj};
use crate::patch::Xref;
use crate::syntax::{Indirect, indirect, ref_spans};

fn failed(why: &str) -> PdfError {
    PdfError::Write(format!("can't linearise the PDF: {why}"))
}

/// Catalog entries whose objects opening the document needs: they go with the catalog.
const OPEN_KEYS: [&[u8]; 5] = [b"ViewerPreferences", b"Threads", b"OpenAction", b"AcroForm", b"OCProperties"];
/// Keys whose objects aren't part of what a page uses: its parent (the page tree) and its
/// thumbnail (shown without the page).
const NOT_USED: [&[u8]; 2] = [b"Parent", b"Thumb"];
/// Deepest page tree read.
const MAX_TREE_DEPTH: usize = 64;
/// Width of the numbers [`write`] fills in once the layout is known (offsets below 10 GB).
const WIDTH: usize = 10;

/// The linearised order of a [`prepare`]d file's objects, by part (object numbers).
pub(crate) struct Plan {
    /// The linearization dictionary: the first object of the first page's cross-reference table.
    first: u32,
    hint: u32,
    /// The catalog and the objects opening the document needs.
    open: Vec<u32>,
    /// The first page: its page object, then every object it uses.
    first_page: Vec<u32>,
    /// Each other page: its page object, then the objects only it uses.
    pages: Vec<Vec<u32>>,
    /// The objects several pages (not the first) use.
    shared: Vec<u32>,
    /// The rest.
    other: Vec<u32>,
    /// For each page but the first, the shared object hint table entries of the objects it uses
    /// from the first page and from [`Self::shared`].
    shared_ids: Vec<Vec<u32>>,
}

/// A written file (one cross-reference table, as the export writes it) read for rewriting.
pub(crate) struct Parsed<'a> {
    pub xref: Xref,
    pub objects: HashMap<u32, Indirect<'a>>,
    /// The objects' numbers and offsets, in file order.
    pub order: Vec<(u32, usize)>,
    pub trailer: Obj<'a>,
    /// The catalog.
    pub root: u32,
    /// The page objects, in order.
    pub pages: Vec<u32>,
    /// The page tree's nodes.
    pub nodes: HashSet<u32>,
}

/// Read `pdf` → its objects, catalog and pages, or why it can't be.
pub(crate) fn parse(pdf: &[u8]) -> Result<Parsed<'_>, &'static str> {
    let xref = Xref::read(pdf).ok_or("no cross-reference table")?;
    let mut order = xref.objects(pdf);
    order.sort_by_key(|e| e.1);
    let mut objects = HashMap::with_capacity(order.len());
    for &(n, off) in &order {
        let o = indirect(pdf, off).filter(|o| o.num == n && o.generation == 0).ok_or("an object can't be read")?;
        objects.insert(n, o);
    }
    let trailer = Lexer::at(pdf, xref.trailer_at()).object(0).ok_or("bad trailer")?;
    let Some(&Obj::Ref(root, _)) = trailer.get(b"Root") else { return Err("no catalog") };
    let catalog = &objects.get(&root).ok_or("no catalog")?.value;
    let Some(&Obj::Ref(tree, _)) = catalog.get(b"Pages") else { return Err("no pages") };
    let (mut pages, mut nodes) = (vec![], HashSet::new());
    page_tree(&objects, tree, 0, &mut pages, &mut nodes)?;
    if pages.is_empty() {
        return Err("no pages");
    }
    Ok(Parsed { xref, objects, order, trailer, root, pages, nodes })
}

/// The pages under page tree node `n`, in order, and the tree's nodes.
pub(crate) fn page_tree(
    objects: &HashMap<u32, Indirect<'_>>,
    n: u32,
    depth: usize,
    pages: &mut Vec<u32>,
    nodes: &mut HashSet<u32>,
) -> Result<(), &'static str> {
    let node = objects.get(&n).ok_or("a page is missing")?;
    match node.value.name(b"Type") {
        Some(b"Page") if !pages.contains(&n) => pages.push(n),
        Some(b"Pages") if depth < MAX_TREE_DEPTH && nodes.insert(n) => {
            let Some(Obj::Array(kids)) = node.value.get(b"Kids") else { return Err("a page tree node has no kids") };
            for kid in kids {
                let Obj::Ref(k, _) = kid else { return Err("a page tree kid isn't a reference") };
                page_tree(objects, *k, depth + 1, pages, nodes)?;
            }
        }
        _ => return Err("bad page tree"),
    }
    Ok(())
}

/// The objects reached from `starts` (themselves included), breadth first, not entering those
/// `stop` names.
fn reach(objects: &HashMap<u32, Indirect<'_>>, starts: &[u32], stop: impl Fn(u32) -> bool) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut queue: VecDeque<u32> = starts.iter().copied().filter(|n| objects.contains_key(n) && seen.insert(*n)).collect();
    let mut out = vec![];
    let mut links = vec![];
    while let Some(n) = queue.pop_front() {
        out.push(n);
        links.clear();
        if let Some(o) = objects.get(&n) {
            o.value.refs(&NOT_USED, &mut links);
        }
        for &m in &links {
            if objects.contains_key(&m) && !stop(m) && seen.insert(m) {
                queue.push_back(m);
            }
        }
    }
    out
}

/// `pdf` (as the export writes it: one cross-reference table, no encryption) with its objects in
/// linearised order and numbering, a placeholder linearization dictionary and hint stream, and the
/// plan [`write`] lays it out with.
pub(crate) fn prepare(pdf: &[u8]) -> Result<(Vec<u8>, Plan), PdfError> {
    let Parsed { objects, order: entries, trailer, root, pages, nodes, .. } = parse(pdf).map_err(failed)?;
    if trailer.get(b"Encrypt").is_some() || trailer.get(b"Prev").is_some() {
        return Err(failed("the file was updated or encrypted already"));
    }
    let catalog = &objects.get(&root).ok_or_else(|| failed("no catalog"))?.value;
    let Some(&first_page_object) = pages.first() else { return Err(failed("no pages")) };
    let page_set: HashSet<u32> = pages.iter().copied().collect();
    let stop = |n: u32| n == root || page_set.contains(&n) || nodes.contains(&n);

    // The catalog and what opening the document needs.
    let mut starts = vec![];
    for key in OPEN_KEYS {
        if let Some(v) = catalog.get(key) {
            v.refs(&NOT_USED, &mut starts);
        }
    }
    starts.retain(|n| !stop(*n));
    let open: Vec<u32> = std::iter::once(root).chain(reach(&objects, &starts, stop)).collect();
    let mut taken: HashSet<u32> = open.iter().copied().collect();
    // What each page uses (pages don't enter other pages).
    let uses: Vec<Vec<u32>> = pages.iter().map(|&p| reach(&objects, &[p], |n| n != p && stop(n))).collect();
    let first_page: Vec<u32> = uses.first().map_or(&[][..], Vec::as_slice).iter().copied().filter(|n| !taken.contains(n)).collect();
    taken.extend(&first_page);
    let rest = uses.get(1..).unwrap_or_default();
    let mut users: HashMap<u32, usize> = HashMap::new();
    for n in rest.iter().flatten().filter(|n| !taken.contains(n)) {
        *users.entry(*n).or_default() += 1;
    }
    let own: Vec<Vec<u32>> = rest.iter().map(|u| u.iter().copied().filter(|n| users.get(n) == Some(&1)).collect()).collect();
    let mut shared = vec![];
    for &n in rest.iter().flatten() {
        if users.get(&n).is_some_and(|c| *c > 1) && !shared.contains(&n) {
            shared.push(n);
        }
    }
    taken.extend(own.iter().flatten().chain(&shared));
    let other: Vec<u32> = entries.iter().map(|e| e.0).filter(|n| !taken.contains(n)).collect();
    if own.iter().any(|p| p.first().is_none_or(|n| !page_set.contains(n))) || first_page.first() != Some(&first_page_object) {
        return Err(failed("a page object is used by something else"));
    }

    // Their shared object hint table entries: the first page's objects, then the shared ones.
    let index: HashMap<u32, u32> = first_page.iter().chain(&shared).enumerate().map(|(i, n)| (*n, i as u32)).collect();
    let shared_ids: Vec<Vec<u32>> =
        rest.iter().map(|u| u.iter().filter(|n| users.get(n) != Some(&1)).filter_map(|n| index.get(n).copied()).collect()).collect();

    // Numbers: the second part from 1, then the first part.
    let mut numbers = Numbers { next: 1, of: HashMap::with_capacity(objects.len()) };
    let own_new: Vec<Vec<u32>> = own.iter().map(|p| numbers.assign(p)).collect();
    let shared_new = numbers.assign(&shared);
    let other_new = numbers.assign(&other);
    let first = numbers.fresh();
    let open_new = numbers.assign(&open);
    let hint = numbers.fresh();
    let first_page_new = numbers.assign(&first_page);
    let (size, number) = (numbers.next, numbers.of);
    if number.len() != objects.len() {
        return Err(failed("objects were left out"));
    }

    // The file in that order.
    let head = entries.first().and_then(|e| pdf.get(..e.1)).ok_or_else(|| failed("no objects"))?;
    let mut out = Vec::with_capacity(pdf.len() + 256);
    out.extend_from_slice(head);
    let mut offsets = vec![0usize; size as usize];
    let mut put = |num: u32, body: &[u8], out: &mut Vec<u8>| {
        if let Some(slot) = offsets.get_mut(num as usize) {
            *slot = out.len();
        }
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let body = |n: u32| -> Result<Vec<u8>, PdfError> {
        let o = objects.get(&n).ok_or_else(|| failed("an object is missing"))?;
        // Up to `endobj`, without the white space before it.
        let end = o.endobj - pdf.get(..o.endobj).map_or(0, |b| b.iter().rev().take_while(|c| c.is_ascii_whitespace()).count());
        renumber(pdf, o.value_span.0, end.max(o.value_span.1), o.value_span.1, &number)
    };
    put(first, b"<</Linearized 1>>", &mut out);
    for (&n, &new) in open.iter().zip(&open_new) {
        put(new, &body(n)?, &mut out);
    }
    put(hint, b"<</Length 0>>\nstream\n\nendstream", &mut out);
    let order = first_page.iter().zip(&first_page_new).chain(own.iter().flatten().zip(own_new.iter().flatten()));
    for (&n, &new) in order.chain(shared.iter().zip(&shared_new)).chain(other.iter().zip(&other_new)) {
        put(new, &body(n)?, &mut out);
    }
    let table = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f\r\n").as_bytes());
    for off in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    let Obj::Dict { start, end, .. } = &trailer else { return Err(failed("bad trailer")) };
    let Some(Obj::Int { start: size_start, end: size_end, .. }) = trailer.get(b"Size") else { return Err(failed("the trailer has no size")) };
    let mut dict = renumber(pdf, *start, *size_start, *size_start, &number)?;
    dict.extend_from_slice(size.to_string().as_bytes());
    dict.extend(renumber(pdf, *size_end, *end, *end, &number)?);
    out.extend_from_slice(b"trailer\n");
    out.extend(dict);
    out.extend_from_slice(format!("\nstartxref\n{table}\n%%EOF\n").as_bytes());
    let plan = Plan { first, hint, open: open_new, first_page: first_page_new, pages: own_new, shared: shared_new, other: other_new, shared_ids };
    Ok((out, plan))
}

/// New object numbers, given in turn.
struct Numbers {
    next: u32,
    /// Old number → new.
    of: HashMap<u32, u32>,
}

impl Numbers {
    /// A number for a new object.
    fn fresh(&mut self) -> u32 {
        self.next += 1;
        self.next - 1
    }

    /// Numbers for the objects `list` names, in its order.
    fn assign(&mut self, list: &[u32]) -> Vec<u32> {
        list.iter()
            .map(|n| {
                let new = self.fresh();
                self.of.insert(*n, new);
                new
            })
            .collect()
    }
}

/// `pdf[start..end]` with the references in `start..refs_end` renumbered as `number`
/// says (references to objects that aren't there become `null`).
fn renumber(pdf: &[u8], start: usize, end: usize, refs_end: usize, number: &HashMap<u32, u32>) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(end.saturating_sub(start) + 16);
    let mut at = start;
    for (s, e, n) in ref_spans(pdf, start, refs_end) {
        out.extend_from_slice(pdf.get(at..s).ok_or_else(|| failed("bad reference"))?);
        match number.get(&n) {
            Some(new) => out.extend_from_slice(format!("{new} 0 R").as_bytes()),
            None => out.extend_from_slice(b"null"),
        }
        at = e;
    }
    out.extend_from_slice(pdf.get(at..end).ok_or_else(|| failed("bad object"))?);
    Ok(out)
}

/// Bits written most significant first; [`Self::align`] pads to a byte.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    byte: u8,
    used: u32,
}

impl Bits {
    fn put(&mut self, value: u64, bits: u32) {
        for i in (0..bits).rev() {
            self.byte = self.byte << 1 | u8::from(value >> i & 1 == 1);
            self.used += 1;
            if self.used == 8 {
                self.out.push(self.byte);
                (self.byte, self.used) = (0, 0);
            }
        }
    }

    fn align(&mut self) {
        if self.used > 0 {
            self.put(0, 8 - self.used);
        }
    }

    /// One item of every entry, then the padding to a byte (each item of a hint table starts on
    /// one).
    fn column(&mut self, values: impl IntoIterator<Item = u64>, bits: u32) {
        for v in values {
            self.put(v, bits);
        }
        self.align();
    }
}

/// Bits needed to write `v`.
fn bits(v: u64) -> u32 {
    64 - v.leading_zeros()
}

/// A 32-bit hint table value.
fn u32_of(v: usize) -> Result<u64, PdfError> {
    u32::try_from(v).map(u64::from).map_err(|_| failed("the file is too large"))
}

/// The page offset and shared object hint tables → (the hint stream's data, where the shared
/// object table starts), from where each object starts and how long it is, the hint stream left
/// out.
fn hint_tables(plan: &Plan, at: &HashMap<u32, (usize, usize)>) -> Result<(Vec<u8>, usize), PdfError> {
    let len = |n: &u32| at.get(n).map_or(0, |a| a.1);
    let pages: Vec<&[u32]> = std::iter::once(plan.first_page.as_slice()).chain(plan.pages.iter().map(Vec::as_slice)).collect();
    let counts: Vec<u64> = pages.iter().map(|p| p.len() as u64).collect();
    let lengths = pages.iter().map(|p| u32_of(p.iter().map(len).sum())).collect::<Result<Vec<u64>, _>>()?;
    let nshared: Vec<u64> = std::iter::once(0).chain(plan.shared_ids.iter().map(|s| s.len() as u64)).collect();
    let span = |v: &[u64]| {
        let (min, max) = (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(0));
        (min, bits(max - min))
    };
    let ((min_count, count_bits), (min_len, len_bits)) = (span(&counts), span(&lengths));
    let entries = plan.first_page.len() + plan.shared.len();
    let id_bits = bits(entries.saturating_sub(1) as u64);
    let first_at = plan.first_page.first().and_then(|n| at.get(n)).map_or(0, |a| a.0);
    let mut b = Bits::default();
    // The page offset hint table: its header, then each item for every page.
    b.put(min_count, 32);
    b.put(u32_of(first_at)?, 32);
    b.put(u64::from(count_bits), 16);
    b.put(min_len, 32);
    b.put(u64::from(len_bits), 16);
    // Content streams' offsets and lengths: the page's.
    b.put(0, 32);
    b.put(0, 16);
    b.put(min_len, 32);
    b.put(u64::from(len_bits), 16);
    b.put(u64::from(bits(nshared.iter().copied().max().unwrap_or(0))), 16);
    b.put(u64::from(id_bits), 16);
    // No fractional positions of shared objects.
    b.put(0, 16);
    b.put(1, 16);
    b.column(counts.iter().map(|c| c - min_count), count_bits);
    b.column(lengths.iter().map(|l| l - min_len), len_bits);
    b.column(nshared.iter().copied(), bits(nshared.iter().copied().max().unwrap_or(0)));
    b.column(plan.shared_ids.iter().flatten().map(|i| u64::from(*i)), id_bits);
    b.column(lengths.iter().map(|l| l - min_len), len_bits);
    // The shared object hint table: one object a group, the first page's objects first.
    let shared_at = b.out.len();
    let groups = plan.first_page.iter().chain(&plan.shared).map(|n| u32_of(len(n))).collect::<Result<Vec<u64>, _>>()?;
    let (min_group, group_bits) = span(&groups);
    let first_shared = plan.shared.first().and_then(|n| Some((*n, at.get(n)?.0))).unwrap_or((0, 0));
    b.put(u64::from(first_shared.0), 32);
    b.put(u32_of(first_shared.1)?, 32);
    b.put(plan.first_page.len() as u64, 32);
    b.put(entries as u64, 32);
    b.put(0, 16);
    b.put(min_group, 32);
    b.put(u64::from(group_bits), 16);
    b.column(groups.iter().map(|g| g - min_group), group_bits);
    // No MD5 signatures.
    b.column(groups.iter().map(|_| 0), 1);
    Ok((b.out, shared_at))
}

/// A number padded with spaces to [`WIDTH`].
fn padded(v: usize) -> String {
    format!("{v:<WIDTH$}")
}

/// Lay out a [`prepare`]d file (encrypted after it, or not) as `plan` says, the hint stream
/// encrypted with `cipher` when the file is.
pub(crate) fn write(pdf: &[u8], plan: &Plan, cipher: Option<&mut Cipher>) -> Result<Vec<u8>, PdfError> {
    let xref = Xref::read(pdf).ok_or_else(|| failed("no cross-reference table"))?;
    let mut entries = xref.objects(pdf);
    entries.sort_by_key(|e| e.1);
    // Each object's bytes run to where the next one starts.
    let ends = entries.iter().skip(1).map(|e| e.1).chain(std::iter::once(xref.at()));
    let bytes: HashMap<u32, &[u8]> = entries.iter().zip(ends).filter_map(|(&(n, off), end)| Some((n, pdf.get(off..end)?))).collect();
    let head = entries.first().and_then(|e| pdf.get(..e.1)).ok_or_else(|| failed("no objects"))?;
    let trailer = Lexer::at(pdf, xref.trailer_at()).object(0).ok_or_else(|| failed("bad trailer"))?;
    let (Obj::Dict { start, end, .. }, Some(Obj::Int { value: size, .. })) = (&trailer, trailer.get(b"Size")) else {
        return Err(failed("bad trailer"));
    };
    let size = u32::try_from(*size).map_err(|_| failed("bad trailer"))?;
    let encrypt = match trailer.get(b"Encrypt") {
        Some(Obj::Ref(n, _)) => Some(*n),
        _ => None,
    };
    // The first part: the linearization dictionary, the catalog part with the encryption
    // dictionary, the hint stream and the first page, which the first page's table covers.
    let open: Vec<u32> = plan.open.iter().copied().chain(encrypt).collect();
    let mut first_part: Vec<u32> = [plan.first, plan.hint].into_iter().chain(open.iter().copied()).chain(plan.first_page.iter().copied()).collect();
    first_part.sort_unstable();
    let rest: Vec<u32> = plan.pages.iter().flatten().chain(&plan.shared).chain(&plan.other).copied().collect();
    let complete = (1..size).all(|n| bytes.contains_key(&n));
    if !complete || !first_part.iter().copied().eq(plan.first..size) || rest.len() + 1 != plan.first as usize {
        return Err(failed("the objects don't match the plan"));
    }
    let len = |n: &u32| bytes.get(n).map_or(0, |b| b.len());

    // The layout without the hint stream (where its tables place the objects).
    let lin_len = linearization(plan, [0; 5]).len();
    let dict = pdf.get(*start..end.saturating_sub(2)).ok_or_else(|| failed("bad trailer"))?;
    let first_table = |prev: usize, at: &dyn Fn(u32) -> usize| {
        let mut t = format!("xref\n{} {}\n", plan.first, size - plan.first).into_bytes();
        for n in plan.first..size {
            t.extend_from_slice(format!("{:010} 00000 n\r\n", at(n)).as_bytes());
        }
        t.extend_from_slice(b"trailer\n");
        t.extend_from_slice(dict);
        t.extend_from_slice(format!("/Prev {}>>\nstartxref\n0\n%%EOF\n", padded(prev)).as_bytes());
        t
    };
    let table_len = first_table(0, &|_| 0).len();
    let mut at: HashMap<u32, (usize, usize)> = HashMap::with_capacity(bytes.len() + 2);
    let mut pos = head.len() + lin_len + table_len;
    let mut place = |list: &mut dyn Iterator<Item = &u32>, pos: &mut usize| {
        for n in list {
            at.insert(*n, (*pos, len(n)));
            *pos += len(n);
        }
    };
    place(&mut open.iter(), &mut pos);
    let hint_at = pos;
    place(&mut plan.first_page.iter(), &mut pos);
    let first_end = pos;
    place(&mut rest.iter(), &mut pos);
    let main_at = pos;

    // The hint stream, and every offset after it moved by its length.
    let (tables, shared_at) = hint_tables(plan, &at)?;
    let data = match cipher {
        Some(c) => c.stream(plan.hint, &tables)?,
        None => tables,
    };
    let mut hint = format!("{} 0 obj\n<</Length {}/S {shared_at}>>\nstream\n", plan.hint, data.len()).into_bytes();
    hint.extend_from_slice(&data);
    hint.extend_from_slice(b"\nendstream\nendobj\n");
    let shift = |off: usize| if off >= hint_at { off + hint.len() } else { off };
    let offset = |n: u32| match n {
        n if n == plan.first => head.len(),
        n if n == plan.hint => hint_at,
        n => at.get(&n).map_or(0, |a| shift(a.0)),
    };
    let main = {
        let mut t = format!("xref\n0 {}\n0000000000 65535 f\r\n", plan.first).into_bytes();
        for n in 1..plan.first {
            t.extend_from_slice(format!("{:010} 00000 n\r\n", offset(n)).as_bytes());
        }
        t
    };
    let main_at = shift(main_at);
    let entries_at = main_at + format!("xref\n0 {}\n", plan.first).len();
    let tail = format!("trailer\n<</Size {}>>\nstartxref\n{}\n%%EOF\n", plan.first, head.len() + lin_len);
    let total = main_at + main.len() + tail.len();
    if total >= 10usize.pow(WIDTH as u32) {
        return Err(failed("the file is too large"));
    }
    let lin = linearization(plan, [total, hint_at, hint.len(), shift(first_end), entries_at - 1]);
    let table = first_table(main_at, &offset);
    if lin.len() != lin_len || table.len() != table_len {
        return Err(failed("the layout moved"));
    }

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(head);
    out.extend_from_slice(&lin);
    out.extend_from_slice(&table);
    let copy = |list: &mut dyn Iterator<Item = &u32>, out: &mut Vec<u8>| {
        for n in list {
            out.extend_from_slice(bytes.get(n).copied().unwrap_or_default());
        }
    };
    copy(&mut open.iter(), &mut out);
    out.extend_from_slice(&hint);
    copy(&mut plan.first_page.iter(), &mut out);
    copy(&mut rest.iter(), &mut out);
    out.extend_from_slice(&main);
    out.extend_from_slice(tail.as_bytes());
    if out.len() != total {
        return Err(failed("the layout moved"));
    }
    Ok(out)
}

/// The linearization dictionary object, its numbers ([file length, hint stream offset and length,
/// end of the first page, offset of the white space before the main table's first entry]) padded
/// to a fixed width.
fn linearization(plan: &Plan, [l, h, h_len, e, t]: [usize; 5]) -> Vec<u8> {
    let o = plan.first_page.first().copied().unwrap_or_default();
    let n = 1 + plan.pages.len();
    format!(
        "{} 0 obj\n<</Linearized 1/L {}/H[{} {}]/O {o}/E {}/N {n}/T {}>>\nendobj\n",
        plan.first,
        padded(l),
        padded(h),
        padded(h_len),
        padded(e),
        padded(t)
    )
    .into_bytes()
}
