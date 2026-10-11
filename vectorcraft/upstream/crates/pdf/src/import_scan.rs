//! What the interpreter doesn't tell the import, read from the file itself: the optional content
//! groups (layers) with their states and nesting, which group each marked-content sequence belongs
//! to, the isolate and knockout flags of transparency groups, and the names of the fonts.
//!
//! hayro-interpret reports a marked-content sequence by its tag alone and a transparency group
//! without its flags, and skips the content of groups that are off. So each page's content is
//! walked here in the interpreter's order (page content, then form XObjects as they are drawn,
//! skipping what it skips): the k-th `BDC`/`BMC` it reports is the k-th one found here (its tag
//! checks that), and the k-th form transparency group is the k-th group found here. To import
//! content that is off as hidden layers, [`all_on`] gives the file an update that turns every
//! group on.

use std::collections::{HashMap, HashSet};

use hayro_interpret::CacheKey;
use hayro_syntax::Pdf;
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::{Array, Dict, FromBytes, MaybeRef, Name, Object, ObjectIdentifier, dict_or_stream};
use hayro_syntax::page::{Page, Resources};

/// The interpreter's limit on nested form XObjects.
const MAX_DEPTH: u32 = 50;
/// The most content operators walked per page (a guard against forms drawn within each other
/// many times over): the art past it can't be told apart by group. A drawing of a million paths
/// takes a few million; the walk reads about 15 million a second, twice as fast as the
/// interpreter draws them.
const MAX_OPS: usize = 64_000_000;
/// The most marked-content sequences noted per page (24 bytes each): past it the rest of the
/// page isn't told apart either.
const MAX_TAGS: usize = 4_000_000;
/// The most groups read.
const MAX_GROUPS: usize = 10_000;
/// The deepest layers nest in the configuration's `/Order`.
pub(crate) const MAX_NESTING: usize = 32;

/// An optional content group: a layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ocg {
    pub name: String,
    /// On in the file's default configuration (shown by viewers).
    pub on: bool,
    /// Printed (its print usage isn't off).
    pub print: bool,
    pub locked: bool,
    /// The group it is listed under in the layer order (a sublayer of it).
    pub parent: Option<usize>,
}

/// The file's optional content groups.
#[derive(Default)]
pub(crate) struct Ocgs {
    pub list: Vec<Ocg>,
    index: HashMap<ObjectIdentifier, usize>,
    /// The groups that are off (as the interpreter decides it).
    off: HashSet<ObjectIdentifier>,
    /// The groups whose view state applies when the file opens (an `/AS` entry for viewing).
    view_auto: HashSet<ObjectIdentifier>,
    /// The groups placed in the layer order.
    placed: HashSet<usize>,
}

impl Ocgs {
    /// The groups listed in the catalog's `/OCProperties`, with their default states.
    pub fn read(pdf: &Pdf) -> Self {
        let mut o = Self::default();
        let xref = pdf.xref();
        let Some(props) = xref.get::<Dict<'_>>(xref.root_id()).and_then(|c| c.get::<Dict<'_>>(b"OCProperties")) else {
            return o;
        };
        let config = props.get::<Dict<'_>>(b"D").unwrap_or_default();
        let refs = |d: &Dict<'_>, key: &[u8]| -> Vec<ObjectIdentifier> {
            d.get::<Array<'_>>(key)
                .map(|a| a.raw_iter().filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).take(MAX_GROUPS).collect())
                .unwrap_or_default()
        };
        let all = refs(&props, b"OCGs");
        if config.get::<Name<'_>>(b"BaseState").is_some_and(|n| n.as_ref() == b"OFF") {
            o.off.extend(all.iter().copied());
        }
        for id in refs(&config, b"ON") {
            o.off.remove(&id);
        }
        o.off.extend(refs(&config, b"OFF"));
        let auto = config.get::<Array<'_>>(b"AS").map(|a| a.iter::<Dict<'_>>().take(MAX_GROUPS).collect::<Vec<_>>()).unwrap_or_default();
        for d in auto.iter().filter(|d| d.get::<Name<'_>>(b"Event").is_some_and(|e| e.as_ref() == b"View")) {
            o.view_auto.extend(refs(d, b"OCGs"));
        }
        let locked: HashSet<_> = refs(&config, b"Locked").into_iter().collect();
        for id in all {
            if let Some(d) = xref.get::<Dict<'_>>(id) {
                o.add(id, &d, locked.contains(&id));
            }
        }
        if let Some(order) = config.get::<Array<'_>>(b"Order") {
            o.nest(&order, None, 0, xref, &mut 0);
        }
        o
    }

    /// Read the layer order `order` (listed under group `parent`): a group followed by an array
    /// lists that array's groups as its sublayers; an array starting with a label lists its
    /// groups under the label, which isn't a group (they go under `parent`). A group listed twice
    /// stays where it is first listed. `seen`: the entries read so far.
    fn nest(&mut self, order: &Array<'_>, parent: Option<usize>, depth: usize, xref: &hayro_syntax::xref::XRef, seen: &mut usize) {
        let mut prev = None;
        for item in order.raw_iter() {
            *seen += 1;
            if *seen > 4 * MAX_GROUPS {
                return;
            }
            let sub = match item {
                MaybeRef::Ref(r) => {
                    let id = ObjectIdentifier::from(r);
                    let sub = xref.get::<Array<'_>>(id);
                    if sub.is_none() {
                        prev = self.place(id, parent);
                    }
                    sub
                }
                MaybeRef::NotRef(o) => o.into_array(),
            };
            if let Some(sub) = sub
                && depth < MAX_NESTING
            {
                let label = matches!(sub.raw_iter().next(), Some(MaybeRef::NotRef(Object::String(_))));
                let under = prev.take().filter(|_| !label).or(parent);
                self.nest(&sub, under, depth + 1, xref, seen);
            }
        }
    }

    /// Put group `id` under `parent` in the layer order → its index, unless it isn't a group or
    /// is already placed. Each group is placed after its parent, so the nesting has no cycles.
    fn place(&mut self, id: ObjectIdentifier, parent: Option<usize>) -> Option<usize> {
        let i = *self.index.get(&id)?;
        let placed = self.placed.insert(i);
        let g = self.list.get_mut(i).filter(|_| placed)?;
        g.parent = parent;
        Some(i)
    }

    /// Note group `id` (its dictionary `d`) → its index.
    fn add(&mut self, id: ObjectIdentifier, d: &Dict<'_>, locked: bool) -> usize {
        if let Some(i) = self.index.get(&id) {
            return *i;
        }
        let name = d.get::<hayro_syntax::object::String<'_>>(b"Name").map(|s| text_string(s.as_bytes())).unwrap_or_default();
        let print = usage(d, b"Print", b"PrintState") != Some(false);
        // A view state applies when the configuration says so on opening; an off one hides the
        // group all the same (it says how the group is meant to be seen).
        let view = usage(d, b"View", b"ViewState");
        let on = match view {
            Some(v) if self.view_auto.contains(&id) => v,
            Some(false) => false,
            _ => !self.off.contains(&id),
        };
        let name = if name.is_empty() { format!("Layer {}", self.list.len() + 1) } else { name };
        self.list.push(Ocg { name, on, print, locked, parent: None });
        self.index.insert(id, self.list.len() - 1);
        self.list.len() - 1
    }

    /// Does the interpreter leave out the art of some group (off in the default configuration)?
    pub fn skips_any(&self) -> bool {
        self.index.keys().any(|id| self.off.contains(id))
    }

    /// The group content marked with `/OC` (`d`: the group or membership dictionary, `id` its
    /// object) belongs to: a membership dictionary's first group.
    fn group_of(&mut self, d: &Dict<'_>, id: ObjectIdentifier, xref: &hayro_syntax::xref::XRef) -> Option<usize> {
        if d.get::<Name<'_>>(b"Type").is_some_and(|t| t.as_ref() == b"OCMD") {
            let first = d.get::<Array<'_>>(b"OCGs").and_then(|a| a.raw_iter().find_map(|i| i.as_obj_ref())).or_else(|| d.get_ref(b"OCGs"))?;
            let id = ObjectIdentifier::from(first);
            return self.note(id, &xref.get::<Dict<'_>>(id)?);
        }
        self.note(id, d)
    }

    /// The index of group `id` (dictionary `d`), noted if new (up to [`MAX_GROUPS`]).
    fn note(&mut self, id: ObjectIdentifier, d: &Dict<'_>) -> Option<usize> {
        match self.index.get(&id) {
            Some(i) => Some(*i),
            None => (self.list.len() < MAX_GROUPS).then(|| self.add(id, d, false)),
        }
    }
}

/// The state (`ON` → true, `OFF` → false) group `d`'s usage `category` gives in `key`.
fn usage(d: &Dict<'_>, category: &[u8], key: &[u8]) -> Option<bool> {
    let state = d.get::<Dict<'_>>(b"Usage")?.get::<Dict<'_>>(category)?.get::<Name<'_>>(key)?;
    match state.as_ref() {
        b"ON" => Some(true),
        b"OFF" => Some(false),
        _ => None,
    }
}

/// A PDF text string: UTF-16 (with its byte order mark), UTF-8 (with one) or PDFDocEncoding
/// (read as Latin-1, which it matches for letters).
pub(crate) fn text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| c as char).collect()
}

/// One page as the interpreter walks it.
#[derive(Default)]
pub(crate) struct Scan {
    /// Each marked-content sequence: its tag ([`tag_key`]) and the group (index into
    /// [`Ocgs::list`]) it marks.
    pub tags: Vec<(u64, Option<usize>)>,
    /// Each form transparency group: isolated, knockout.
    pub groups: Vec<(bool, bool)>,
    /// The walk stopped at [`MAX_OPS`] or [`MAX_TAGS`]: the rest of the page isn't told apart.
    pub cut: bool,
}

/// A marked-content tag as [`Scan::tags`] keeps it: its FNV-1a hash (the tags are only compared,
/// and a page can have millions).
pub(crate) fn tag_key(tag: &[u8]) -> u64 {
    tag.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// [`MAX_OPS`], lowered by tests that need a page past it.
#[cfg(not(test))]
fn max_ops() -> usize {
    MAX_OPS
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_MAX_OPS: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_OPS) };
}

#[cfg(test)]
fn max_ops() -> usize {
    TEST_MAX_OPS.with(std::cell::Cell::get)
}

struct Walker<'w, 'p> {
    ocgs: &'w mut Ocgs,
    /// The groups that are off for the interpreter (none once [`all_on`] turned them on).
    off: HashSet<ObjectIdentifier>,
    xref: &'p hayro_syntax::xref::XRef,
    fonts: &'w mut FontNames,
    visible: Vec<bool>,
    ops: usize,
    max_ops: usize,
    out: Scan,
    /// A form XObject hidden by its own optional content was met.
    hidden_form: bool,
}

impl Walker<'_, '_> {
    fn is_visible(&self) -> bool {
        self.visible.last().copied().unwrap_or(true)
    }

    fn begin(&mut self, on: bool) {
        let v = self.is_visible() && on;
        self.visible.push(v);
    }

    /// Enter optional content `d` (object `id`): a group, or a membership dictionary whose policy
    /// decides from its groups.
    fn begin_oc(&mut self, d: &Dict<'_>, id: Option<ObjectIdentifier>) {
        let membership = id.is_none() || d.get::<Name<'_>>(b"Type").is_some_and(|t| t.as_ref() == b"OCMD");
        let on = match id {
            Some(id) if !membership => !self.off.contains(&id),
            _ => {
                let ids: Vec<ObjectIdentifier> = match d.get::<Array<'_>>(b"OCGs") {
                    Some(a) => a.raw_iter().filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).collect(),
                    None => d.get_ref(b"OCGs").map(ObjectIdentifier::from).into_iter().collect(),
                };
                let on = |i: &ObjectIdentifier| !self.off.contains(i);
                ids.is_empty()
                    || match d.get::<Name<'_>>(b"P").as_deref() {
                        Some(b"AllOn") => ids.iter().all(on),
                        Some(b"AnyOff") => !ids.iter().all(on),
                        Some(b"AllOff") => !ids.iter().any(on),
                        _ => ids.iter().any(on),
                    }
            }
        };
        self.begin(on);
    }

    fn note_fonts(&mut self, res: &Resources<'_>) {
        for (key, _) in res.fonts.entries() {
            if let Some(f) = res.fonts.get::<Dict<'_>>(key.as_ref())
                && let Some(name) = f.get::<Name<'_>>(b"BaseFont")
            {
                self.fonts.entry(f.cache_key()).or_insert_with(|| (String::from_utf8_lossy(name.as_ref()).into_owned(), embeds_glyphs(&f)));
            }
        }
    }

    fn walk(&mut self, mut iter: TypedIter<'_>, res: &Resources<'_>, depth: u32) {
        self.note_fonts(res);
        while let Some(op) = iter.next() {
            self.ops += 1;
            if self.ops > self.max_ops || self.out.tags.len() >= MAX_TAGS {
                self.out.cut = true;
                return;
            }
            match op {
                TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                    let oc = match bdc.1 {
                        Object::Name(n) => {
                            res.properties.get_ref(n.as_ref()).map(|r| (res.properties.get::<Dict<'_>>(n.as_ref()).unwrap_or_default(), r))
                        }
                        o => dict_or_stream(o)
                            .and_then(|(props, _)| props.get_ref(b"OC").map(|r| (props.get::<Dict<'_>>(b"OC").unwrap_or_default(), r))),
                    };
                    let group = match oc {
                        Some((d, r)) => {
                            let id = ObjectIdentifier::from(r);
                            self.begin_oc(&d, Some(id));
                            self.ocgs.group_of(&d, id, self.xref)
                        }
                        None => {
                            self.begin(true);
                            None
                        }
                    };
                    self.out.tags.push((tag_key(bdc.0.as_ref()), group));
                }
                TypedInstruction::BeginMarkedContent(bmc) => {
                    self.begin(true);
                    self.out.tags.push((tag_key(bmc.0.as_ref()), None));
                }
                TypedInstruction::EndMarkedContent(_) => {
                    self.visible.pop();
                }
                TypedInstruction::XObject(x) => {
                    if !self.is_visible() || depth >= MAX_DEPTH {
                        continue;
                    }
                    let Some(s) = res.get_x_object(x.0) else { continue };
                    let d = s.dict();
                    if d.get::<Name<'_>>(b"Subtype").as_deref() != Some(b"Form") || d.get::<[f32; 4]>(b"BBox").is_none() {
                        continue;
                    }
                    let Ok(data) = s.decoded() else { continue };
                    let oc = d.get::<Dict<'_>>(b"OC");
                    if let Some(oc) = &oc {
                        self.begin_oc(oc, d.get_ref(b"OC").map(ObjectIdentifier::from));
                        self.hidden_form |= !self.is_visible();
                    }
                    if self.is_visible() {
                        if let Some(g) = d.get::<Dict<'_>>(b"Group")
                            && self.out.groups.len() < MAX_GROUPS
                        {
                            self.out.groups.push((g.get::<bool>(b"I").unwrap_or(false), g.get::<bool>(b"K").unwrap_or(false)));
                        }
                        let inner = Resources::from_parent(d.get::<Dict<'_>>(b"Resources").unwrap_or_default(), res.clone());
                        self.walk(TypedIter::new(data.as_ref()), &inner, depth + 1);
                    }
                    if oc.is_some() {
                        self.visible.pop();
                    }
                }
                _ => {}
            }
        }
    }
}

/// The fonts of a document by cache key: base font name, and whether the file carries the glyphs
/// ([`embeds_glyphs`]).
pub(crate) type FontNames = HashMap<u128, (String, bool)>;

/// Whether font `f` carries its own glyphs: a font program in its descriptor (a composite font's in
/// its descendant's), or a Type 3 font's glyph procedures. A font that doesn't is drawn with a
/// stand-in, whose glyphs say nothing about which installed face the document means.
fn embeds_glyphs(f: &Dict<'_>) -> bool {
    if f.get::<Name<'_>>(b"Subtype").is_some_and(|s| s.as_ref() == b"Type3") {
        return true;
    }
    // A font program is a stream, so always an indirect object.
    let program = |d: &Dict<'_>| {
        d.get::<Dict<'_>>(b"FontDescriptor").is_some_and(|fd| [&b"FontFile"[..], b"FontFile2", b"FontFile3"].iter().any(|k| fd.get_ref(*k).is_some()))
    };
    program(f) || f.get::<Array<'_>>(b"DescendantFonts").and_then(|a| a.iter::<Dict<'_>>().next()).is_some_and(|d| program(&d))
}

/// Walk `page` as the interpreter will: `all_on` when [`all_on`] turned every group on. Fonts
/// found are noted in `fonts` (their cache key → base font name, and whether the file carries its
/// glyphs).
pub(crate) fn scan_page(page: &Page<'_>, ocgs: &mut Ocgs, all_on: bool, fonts: &mut FontNames) -> Scan {
    walk_page(page, ocgs, all_on, fonts).out
}

fn walk_page<'w, 'p>(page: &Page<'p>, ocgs: &'w mut Ocgs, all_on: bool, fonts: &'w mut FontNames) -> Walker<'w, 'p> {
    let off = if all_on { HashSet::new() } else { ocgs.off.clone() };
    let mut w = Walker { ocgs, off, xref: page.xref(), fonts, visible: vec![], ops: 0, max_ops: max_ops(), out: Scan::default(), hidden_form: false };
    w.walk(page.typed_operations(), page.resources(), 0);
    w
}

/// Does `page` draw a form XObject that is off by its own optional content (not a marked-content
/// sequence)? Its art couldn't be told apart by group once every group is on.
pub(crate) fn hides_forms(page: &Page<'_>, ocgs: &mut Ocgs) -> bool {
    walk_page(page, ocgs, false, &mut HashMap::new()).hidden_form
}

/// `bytes` with an update appended that drops the catalog's `/OCProperties`, so every group
/// draws; `None` when the file's structure can't take one.
pub(crate) fn all_on(bytes: &[u8], pdf: &Pdf) -> Option<Vec<u8>> {
    let root = pdf.xref().root_id();
    let catalog = pdf.xref().get::<Dict<'_>>(root)?;
    let body = catalog.data().strip_suffix(b">>")?;
    let prev = last_xref(bytes)?;
    let trailer = trailer_at(bytes, prev)?;
    let inner = trailer.strip_prefix(b"<<")?.strip_suffix(b">>")?;
    let mut out = bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let at = out.len();
    out.extend(format!("{} {} obj\n", root.obj_number, root.gen_number).bytes());
    out.extend_from_slice(body);
    // A later key wins: the groups' configuration is gone, so the interpreter draws them all.
    out.extend_from_slice(b" /OCProperties null >>\nendobj\n");
    let xref = out.len();
    out.extend(format!("xref\n{} 1\n{at:010} {:05} n \ntrailer\n<<", root.obj_number, root.gen_number).bytes());
    out.extend_from_slice(inner);
    out.extend(format!(" /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    Some(out)
}

/// The offset the file's last `startxref` points to.
fn last_xref(bytes: &[u8]) -> Option<usize> {
    let at = bytes.windows(9).rposition(|w| w == b"startxref")?;
    let rest = bytes.get(at + 9..)?;
    let digits: String = rest.iter().skip_while(|c| c.is_ascii_whitespace()).take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    digits.parse().ok().filter(|&p: &usize| p < bytes.len())
}

/// The trailer dictionary of the cross-reference section at `at` (a table's `trailer`, or a
/// cross-reference stream's dictionary), as written.
fn trailer_at(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let rest = bytes.get(at..)?;
    let from = if rest.trim_ascii_start().starts_with(b"xref") { rest.windows(7).position(|w| w == b"trailer")? } else { 0 };
    let open = from + rest.get(from..)?.windows(2).position(|w| w == b"<<")?;
    let d = Dict::from_bytes(rest.get(open..)?)?;
    Some(d.data())
}
