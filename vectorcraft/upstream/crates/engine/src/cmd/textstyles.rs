//! Character Styles and Paragraph Styles panels.
//!
//! A style is a named, partial set of attributes ([`TextStyleDef`]); text using it records the
//! name in `CharStyle::style_name` / `ParaStyle::style_name`. Applying a style sets its attributes
//! (Clear Overrides first resets to the Normal style). Redefining a style updates its users but keeps
//! their local overrides: an attribute changes only where it still had the old style's value.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{CharStyle, Document, NodeId, NodeKind, ParaStyle, TextStyleDef};

use super::typecmd::{refresh_bounds, text_targets};
use super::*;

pub const NORMAL_CHAR: &str = "[Normal Character Style]";
pub const NORMAL_PARA: &str = "[Normal Paragraph Style]";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Char,
    Para,
}

impl Kind {
    fn cmd(self, verb: &str) -> String {
        format!("{}.{verb}", if self == Kind::Char { "charStyle" } else { "paraStyle" })
    }
    fn normal(self) -> &'static str {
        if self == Kind::Char { NORMAL_CHAR } else { NORMAL_PARA }
    }
    fn defs(self, d: &Document) -> &Vec<TextStyleDef> {
        if self == Kind::Char { &d.char_styles } else { &d.para_styles }
    }
    fn defs_mut(self, d: &mut Document) -> &mut Vec<TextStyleDef> {
        if self == Kind::Char { &mut d.char_styles } else { &mut d.para_styles }
    }
    /// Attributes of style `name` (the Normal style is built in: empty unless redefined).
    fn attrs(self, d: &Document, name: &str) -> Option<Map<String, Value>> {
        match self.defs(d).iter().find(|s| s.name == name) {
            Some(s) => Some(s.attrs.clone()),
            None if name == self.normal() => Some(Map::new()),
            None => None,
        }
    }
}

/// One command spec (the `cmd!` macro wants literals; these ids are built with `concat!`).
const fn spec(id: &'static str, label: &'static str, menu: &'static [&'static str], params: &'static str, run: Run, journal: bool) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut: None, params, enabled: has_doc, run, journal }
}

macro_rules! style_cmds {
    ($kind:expr, $p:literal, $menu:literal, $what:literal) => {
        vec![
            spec(concat!($p, ".list"), concat!($what, " Styles"), &[], "{} → {styles: [{name, attrs, uses}]}", |s, _| list(s, $kind), false),
            spec(
                concat!($p, ".new"),
                concat!("New ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name?, attrs?: {…}} (default attrs: the selected text's) → {name}",
                |s, p| new(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".apply"),
                concat!("Apply ", $what, " Style"),
                &[],
                "{name, clearOverrides?, ids?|id + start?/end? (a Type tool range: character styles style it, paragraph styles apply to the paragraphs it touches)} → {count}",
                |s, p| apply(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".redefine"),
                concat!("Redefine ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name, id?, start?} from the selected text; users keep their overrides",
                |s, p| redefine(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".setAttrs"),
                concat!($what, " Style Options"),
                &[],
                "{name, attrs: {…}} replace the style's attributes; users keep their overrides",
                |s, p| set_attrs(s, p, $kind),
                true,
            ),
            spec(
                concat!($p, ".duplicate"),
                concat!("Duplicate ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name} → {name}",
                |s, p| duplicate(s, p, $kind),
                true,
            ),
            spec(concat!($p, ".rename"), concat!("Rename ", $what, " Style"), &[], "{name, to}", |s, p| rename(s, p, $kind), true),
            spec(
                concat!($p, ".delete"),
                concat!("Delete ", $what, " Style"),
                &["Window", "Type", $menu],
                "{name} (text keeps its look)",
                |s, p| delete(s, p, $kind),
                true,
            ),
        ]
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = style_cmds!(Kind::Char, "charStyle", "Character Styles", "Character");
    v.extend(style_cmds!(Kind::Para, "paraStyle", "Paragraph Styles", "Paragraph"));
    v
}

// ---------- attribute plumbing ----------

/// `v` as a JSON map with one key per attribute: Auto alignment, which files save as its
/// direction's alignment plus `justify_auto` (see `vectorcraft_doc::ParaStyle`), is `"justify":
/// "Auto"`, so a style's `justify` replaces it.
fn to_map<T: Serialize>(v: &T) -> Option<Map<String, Value>> {
    let Ok(Value::Object(mut m)) = serde_json::to_value(v) else { return None };
    if m.remove("justify_auto") == Some(Value::Bool(true)) {
        m.insert("justify".into(), Value::from("Auto"));
    }
    Some(m)
}

/// A style's attributes as a JSON map, without the style name.
fn to_attrs<T: Serialize>(v: &T) -> Map<String, Value> {
    let mut m = to_map(v).unwrap_or_default();
    m.remove("style_name");
    m
}

/// Every attribute name `T` reads, including those its JSON leaves out at their defaults (Auto
/// leading and kerning, no mojikumi…): the field names serde's derive hands `deserialize_struct`.
/// Not `justify_auto`, which [`to_map`] turns into `"justify": "Auto"`.
fn attr_names<T: DeserializeOwned>() -> Vec<&'static str> {
    use serde::de::{Error as _, Visitor, value::Error};

    struct Names<'a>(&'a mut &'static [&'static str]);
    impl<'de> serde::Deserializer<'de> for Names<'_> {
        type Error = Error;
        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> std::result::Result<V::Value, Error> {
            Err(Error::custom("not a struct"))
        }
        fn deserialize_struct<V: Visitor<'de>>(self, _: &'static str, fields: &'static [&'static str], _: V) -> std::result::Result<V::Value, Error> {
            *self.0 = fields;
            Err(Error::custom("field names only"))
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit
            unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
        }
    }

    let mut fields: &'static [&'static str] = &[];
    // Always an error: the probe only records the names.
    let _ = T::deserialize(Names(&mut fields));
    fields.iter().copied().filter(|&f| f != "justify_auto").collect()
}

/// `base` with `attrs` written over it (unknown or ill-typed attributes are an error).
fn with_attrs<T: Serialize + DeserializeOwned>(base: &T, attrs: &Map<String, Value>, cmd: &str) -> Result<T> {
    let mut m = to_attrs(base);
    if let Some(full) = to_map(base) {
        m.insert("style_name".into(), full.get("style_name").cloned().unwrap_or(Value::Null));
    }
    let names = attr_names::<T>();
    for (k, v) in attrs {
        // Not only the keys `base` writes: those at their defaults (Auto leading…) are left out.
        if !m.contains_key(k) && !names.contains(&k.as_str()) {
            return Err(bad(cmd, format!("unknown attribute `{k}`")));
        }
        m.insert(k.clone(), v.clone());
    }
    serde_json::from_value(Value::Object(m)).map_err(|e| bad(cmd, e.to_string()))
}

/// `cur` after its style changed from `old` to `new`: attributes still equal to the old style's
/// value (or not set by it) take the new value; overrides stay. Unchanged if the style doesn't
/// round-trip through JSON.
fn restyle<T: Serialize + DeserializeOwned + Clone>(cur: &T, old: &Map<String, Value>, new: &Map<String, Value>) -> T {
    let Some(mut m) = to_map(cur) else { return cur.clone() };
    for (k, v) in new {
        if old.get(k).is_none_or(|o| m.get(k) == Some(o)) {
            m.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(m)).unwrap_or_else(|_| cur.clone())
}

/// Every text object id in the document.
fn all_text(d: &Document) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            v.push(n.id)
        }
    });
    v
}

fn text_mut(d: &mut Document, id: NodeId) -> Option<&mut vectorcraft_doc::TextObject> {
    match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Text(t)) => Some(t),
        _ => None,
    }
}

/// How many runs (char) or paragraphs (para) use `name`.
fn uses(d: &Document, kind: Kind, name: &str) -> usize {
    let target = (name != kind.normal()).then_some(name);
    let mut n = 0;
    d.walk(|node| {
        if let NodeKind::Text(t) = &node.kind {
            n += match kind {
                Kind::Char => t.runs.iter().filter(|r| r.style.style_name.as_deref() == target).count(),
                Kind::Para => (0..t.paragraph_count()).filter(|&i| t.para_at(i).style_name.as_deref() == target).count(),
            };
        }
    });
    n
}

/// Update every user of `name` from `old` to `new` attributes (overrides kept).
fn update_users(d: &mut Document, kind: Kind, name: &str, old: &Map<String, Value>, new: &Map<String, Value>) {
    let target = (name != kind.normal()).then_some(name.to_string());
    for id in all_text(d) {
        let Some(t) = text_mut(d, id) else { continue };
        let mut changed = false;
        match kind {
            Kind::Char => {
                for r in &mut t.runs {
                    if r.style.style_name == target {
                        r.style = restyle(&r.style, old, new);
                        changed = true;
                    }
                }
            }
            Kind::Para => {
                for pa in t.para_styles_mut() {
                    if pa.style_name == target {
                        *pa = restyle(pa, old, new);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            refresh_bounds(t);
        }
    }
}

/// The selected text's attributes: the Type tool range's start (`id`, `start`) or the first
/// selected text object's first run.
fn selection_attrs(s: &Session, p: &Value, kind: Kind) -> Result<Option<Map<String, Value>>> {
    let d = &s.doc()?.doc;
    let id = match id_param(p, "id") {
        Some(id) => Some(id),
        None => text_targets(s, &json!({}), "").ok().and_then(|v| v.first().copied()),
    };
    let Some(NodeKind::Text(t)) = id.and_then(|i| d.node(i)).map(|n| &n.kind) else { return Ok(None) };
    let at = p.get("start").and_then(Value::as_u64).map_or(0, |v| usize::try_from(v).unwrap_or(usize::MAX));
    Ok(Some(match kind {
        Kind::Char => to_attrs(&vectorcraft_text::edit::style_at(&t.runs, at)),
        // The paragraph at `start`.
        Kind::Para => to_attrs(t.para_at(t.paragraphs_in(at, at).start)),
    }))
}

/// The selected text's character (`para`: paragraph) attributes, every one of them: what a
/// library keeps of a text style. `None` without text selected.
pub(crate) fn selected_style_attrs(s: &Session, para: bool) -> Result<Option<Map<String, Value>>> {
    selection_attrs(s, &json!({}), if para { Kind::Para } else { Kind::Char })
}

/// Whether the document has a character (`para`: paragraph) style `name`, and its attributes.
pub(crate) fn style_attrs(d: &Document, para: bool, name: &str) -> Option<Map<String, Value>> {
    if para { Kind::Para } else { Kind::Char }.attrs(d, name)
}

fn name_param<'a>(p: &'a Value, kind: Kind, verb: &str) -> Result<&'a str> {
    str_param(p, "name").ok_or_else(|| bad(&kind.cmd(verb), "missing `name`"))
}

fn attrs_param(p: &Value, kind: Kind, verb: &str) -> Result<Option<Map<String, Value>>> {
    match p.get("attrs") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => {
            // Validate against the attribute set, and store values in their canonical serialized
            // form (so comparing a style with its text's attributes is exact).
            let c = kind.cmd(verb);
            let full = match kind {
                Kind::Char => to_attrs(&with_attrs(&CharStyle::default(), m, &c)?),
                Kind::Para => to_attrs(&with_attrs(&ParaStyle::default(), m, &c)?),
            };
            Ok(Some(m.keys().filter_map(|k| full.get(k).map(|v| (k.clone(), v.clone()))).collect()))
        }
        Some(_) => Err(bad(&kind.cmd(verb), "attrs must be an object")),
    }
}

/// The attributes of a new style made from nothing (no `attrs`, no text selected): while the
/// interface is in Japanese, new type's em box top-to-top leading (paragraph styles) and em box
/// centre alignment (character styles); else none (the defaults).
fn new_style_attrs(s: &Session, kind: Kind) -> Map<String, Value> {
    if !s.japanese_interface() {
        return Map::new();
    }
    let (set, default) = match kind {
        Kind::Char => {
            let st = CharStyle { char_align: vectorcraft_doc::CharAlign::EmBoxCenter, ..CharStyle::default() };
            (to_attrs(&st), to_attrs(&CharStyle::default()))
        }
        Kind::Para => {
            let st = ParaStyle { leading_model: vectorcraft_doc::LeadingModel::EmBoxTop, ..ParaStyle::default() };
            (to_attrs(&st), to_attrs(&ParaStyle::default()))
        }
    };
    set.into_iter().filter(|(k, v)| default.get(k) != Some(v)).collect()
}

// ---------- commands ----------

fn list(s: &mut Session, kind: Kind) -> Result<Value> {
    let d = &s.doc()?.doc;
    let mut names = vec![kind.normal().to_string()];
    names.extend(kind.defs(d).iter().map(|s| s.name.clone()).filter(|n| n != kind.normal()));
    let styles: Vec<Value> = names.iter().map(|n| json!({ "name": n, "attrs": kind.attrs(d, n), "uses": uses(d, kind, n) })).collect();
    Ok(json!({ "styles": styles }))
}

fn new(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("new");
    let attrs = match attrs_param(p, kind, "new")? {
        Some(a) => a,
        None => match selection_attrs(s, p, kind)? {
            Some(a) => a,
            None => {
                let a = new_style_attrs(s, kind);
                s.note_journal("attrs", Value::Object(a.clone()));
                a
            }
        },
    };
    let d = &s.doc()?.doc;
    let base = if kind == Kind::Char { "Character Style" } else { "Paragraph Style" };
    let name = match str_param(p, "name") {
        Some(n) => n.to_string(),
        None => (1..).map(|i| format!("{base} {i}")).find(|n| kind.attrs(d, n).is_none()).unwrap_or_else(|| base.to_string()),
    };
    if name.trim().is_empty() || kind.attrs(d, &name).is_some() {
        return Err(bad(&c, format!("a style named `{name}` already exists")));
    }
    let n2 = name.clone();
    s.edit(&format!("New {base}"), |d, _| {
        kind.defs_mut(d).push(TextStyleDef { name: n2, attrs });
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn apply(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("apply");
    let name = name_param(p, kind, "apply")?.to_string();
    let d = &s.doc()?.doc;
    let attrs = kind.attrs(d, &name).ok_or_else(|| bad(&c, format!("no style `{name}`")))?;
    let normal = kind.attrs(d, kind.normal()).unwrap_or_default();
    let clear = bool_or(p, "clearOverrides", false);
    let style_name = (name != kind.normal()).then(|| name.clone());
    // A Type tool range or whole text objects.
    let text_range = super::typecmd::TextRange::parse(p, &c)?;
    let range = id_param(p, "id").filter(|_| text_range.is_some());
    let ids = match range.or_else(|| id_param(p, "id")) {
        Some(id) => vec![id],
        None => text_targets(s, p, &c)?,
    };
    // Characters: Normal then the style over the defaults (clear) or over the current attributes.
    let char_fn = |cur: &CharStyle| -> Result<CharStyle> {
        let base = if clear { with_attrs(&CharStyle::default(), &normal, &c)? } else { cur.clone() };
        let mut st = with_attrs(&base, &attrs, &c)?;
        st.style_name = style_name.clone();
        Ok(st)
    };
    let para_fn = |cur: &ParaStyle| -> Result<ParaStyle> {
        let base = if clear { with_attrs(&ParaStyle::default(), &normal, &c)? } else { cur.clone() };
        let mut st = with_attrs(&base, &attrs, &c)?;
        st.style_name = style_name.clone();
        Ok(st)
    };
    let count = ids.len();
    s.edit(&format!("Apply {name}"), |d, _| {
        for id in &ids {
            let Some(t) = text_mut(d, *id) else { continue };
            match kind {
                Kind::Char => {
                    let len = vectorcraft_text::edit::runs_len(&t.runs);
                    let (a, b) = match range {
                        Some(_) => {
                            let g = |k: &str, def: usize| p.get(k).and_then(Value::as_u64).map_or(def, |v| (v as usize).min(len));
                            (g("start", 0), g("end", len))
                        }
                        None => (0, len),
                    };
                    let mut err = None;
                    vectorcraft_text::edit::style_range(&mut t.runs, a.min(b), a.max(b), |st| match char_fn(st) {
                        Ok(n) => *st = n,
                        Err(e) => err = Some(e),
                    });
                    if len == 0 {
                        for r in &mut t.runs {
                            r.style = char_fn(&r.style)?;
                        }
                    }
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
                Kind::Para => {
                    let mut v = t.paragraph_styles();
                    let span = match (range, text_range) {
                        (Some(_), Some(r)) => r.paras(t),
                        _ => 0..v.len(),
                    };
                    for st in v.iter_mut().take(span.end).skip(span.start) {
                        *st = para_fn(st)?;
                    }
                    t.set_paragraph_styles(v);
                }
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "count": count }))
}

fn redefine_to(s: &mut Session, kind: Kind, name: &str, new_attrs: Map<String, Value>, verb: &str) -> Result<Value> {
    let d = &s.doc()?.doc;
    let old = kind.attrs(d, name).ok_or_else(|| bad(&kind.cmd(verb), format!("no style `{name}`")))?;
    let name = name.to_string();
    s.edit(&format!("Redefine {name}"), |d, _| {
        match kind.defs_mut(d).iter_mut().find(|s| s.name == name) {
            Some(def) => def.attrs = new_attrs.clone(),
            None => kind.defs_mut(d).push(TextStyleDef { name: name.clone(), attrs: new_attrs.clone() }),
        }
        update_users(d, kind, &name, &old, &new_attrs);
        Ok(())
    })?;
    ok()
}

fn redefine(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "redefine")?.to_string();
    let attrs = selection_attrs(s, p, kind)?.ok_or_else(|| bad(&kind.cmd("redefine"), "select text to redefine the style from"))?;
    redefine_to(s, kind, &name, attrs, "redefine")
}

fn set_attrs(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "setAttrs")?.to_string();
    let attrs = attrs_param(p, kind, "setAttrs")?.ok_or_else(|| bad(&kind.cmd("setAttrs"), "missing `attrs`"))?;
    redefine_to(s, kind, &name, attrs, "setAttrs")
}

fn duplicate(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let name = name_param(p, kind, "duplicate")?.to_string();
    let d = &s.doc()?.doc;
    let attrs = kind.attrs(d, &name).ok_or_else(|| bad(&kind.cmd("duplicate"), format!("no style `{name}`")))?;
    let copy = (1..)
        .map(|i| if i == 1 { format!("{name} copy") } else { format!("{name} copy {i}") })
        .find(|n| kind.attrs(d, n).is_none())
        .unwrap_or_else(|| format!("{name} copy"));
    new(s, &json!({ "name": copy, "attrs": attrs }), kind)
}

fn rename(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("rename");
    let name = name_param(p, kind, "rename")?.to_string();
    let to = str_param(p, "to").ok_or_else(|| bad(&c, "missing `to`"))?.trim().to_string();
    let d = &s.doc()?.doc;
    if name == kind.normal() || !kind.defs(d).iter().any(|s| s.name == name) {
        return Err(bad(&c, format!("`{name}` can't be renamed")));
    }
    if to.is_empty() || kind.attrs(d, &to).is_some() {
        return Err(bad(&c, format!("`{to}` is empty or already used")));
    }
    s.edit("Rename Style", |d, _| {
        if let Some(def) = kind.defs_mut(d).iter_mut().find(|s| s.name == name) {
            def.name = to.clone();
        }
        for id in all_text(d) {
            let Some(t) = text_mut(d, id) else { continue };
            let fix = |n: &mut Option<String>| {
                if n.as_deref() == Some(name.as_str()) {
                    *n = Some(to.clone());
                }
            };
            match kind {
                Kind::Char => t.runs.iter_mut().for_each(|r| fix(&mut r.style.style_name)),
                Kind::Para => t.para_styles_mut().for_each(|pa| fix(&mut pa.style_name)),
            }
        }
        Ok(())
    })?;
    ok()
}

fn delete(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let c = kind.cmd("delete");
    let name = name_param(p, kind, "delete")?.to_string();
    if name == kind.normal() || !kind.defs(&s.doc()?.doc).iter().any(|s| s.name == name) {
        return Err(bad(&c, format!("`{name}` can't be deleted")));
    }
    s.edit("Delete Style", |d, _| {
        kind.defs_mut(d).retain(|s| s.name != name);
        for id in all_text(d) {
            let Some(t) = text_mut(d, id) else { continue };
            let fix = |n: &mut Option<String>| {
                if n.as_deref() == Some(name.as_str()) {
                    *n = None;
                }
            };
            match kind {
                Kind::Char => t.runs.iter_mut().for_each(|r| fix(&mut r.style.style_name)),
                Kind::Para => t.para_styles_mut().for_each(|pa| fix(&mut pa.style_name)),
            }
        }
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_text(text: &str) -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": text})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        (s, id)
    }

    fn runs(s: &Session, id: u64) -> Vec<vectorcraft_doc::TextRun> {
        match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn char_style_new_apply_range_redefine_keeps_overrides() {
        let (mut s, id) = session_with_text("Hello world");
        s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0, "underline": true}})).unwrap();
        // Style "world" only (a Type tool range).
        s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11})).unwrap();
        let r = runs(&s, id);
        assert_eq!(r.len(), 2);
        assert_eq!((r[1].text.as_str(), r[1].style.size, r[1].style.underline), ("world", 24.0, true));
        assert_eq!(r[1].style.style_name.as_deref(), Some("Emphasis"));
        assert_eq!(r[0].style.style_name, None);
        // A local override on the styled run survives a redefinition; other attributes update.
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 6, "end": 11, "size": 30.0})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Emphasis", "attrs": {"size": 18.0, "underline": false, "tracking": 50.0}})).unwrap();
        let r = runs(&s, id);
        assert_eq!((r[1].style.size, r[1].style.underline, r[1].style.tracking), (30.0, false, 50.0));
        // Clear Overrides: back to exactly the style's attributes.
        s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11, "clearOverrides": true})).unwrap();
        assert_eq!(runs(&s, id)[1].style.size, 18.0);
        let list = s.execute("charStyle.list", &json!({})).unwrap();
        assert_eq!(list["styles"][0]["name"], NORMAL_CHAR);
        assert_eq!(list["styles"][1]["uses"], 1);
    }

    #[test]
    fn new_from_selection_rename_delete_and_undo() {
        let (mut s, id) = session_with_text("Title");
        s.execute("text.setStyle", &json!({"size": 40})).unwrap();
        let name = s.execute("charStyle.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
        assert_eq!(name, "Character Style 1");
        assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs["size"], 40.0);
        s.execute("charStyle.apply", &json!({"name": name})).unwrap();
        s.execute("charStyle.rename", &json!({"name": name, "to": "Heading"})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
        assert!(s.execute("charStyle.rename", &json!({"name": NORMAL_CHAR, "to": "x"})).is_err());
        s.execute("charStyle.duplicate", &json!({"name": "Heading"})).unwrap();
        s.execute("charStyle.delete", &json!({"name": "Heading"})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name, None);
        assert_eq!(runs(&s, id)[0].style.size, 40.0, "deleting keeps the look");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
        assert!(s.execute("charStyle.new", &json!({"name": "Bad", "attrs": {"nope": 1}})).is_err());
        // Attributes are stored canonically (a hand-written colour matches the text's afterwards).
        s.execute(
            "charStyle.new",
            &json!({"name": "Pink", "attrs": {"fill": {"type": "solid", "color": {"model": "rgb", "r": 0.9, "g": 0.2, "b": 0.4}}}}),
        )
        .unwrap();
        s.execute("charStyle.apply", &json!({"name": "Pink"})).unwrap();
        let fill = serde_json::to_value(&runs(&s, id)[0].style.fill).unwrap();
        assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Pink").unwrap().attrs["fill"], fill);
    }

    /// #1001: attributes their JSON leaves out at the defaults (Auto leading and kerning, no
    /// mojikumi) are still attributes a style can set, and a style captured with one applies to
    /// text at the default, is duplicated and comes back out of a library.
    #[test]
    fn styles_take_attributes_left_out_at_their_defaults() {
        let (mut s, id) = session_with_text("A");
        s.execute("charStyle.new", &json!({"name": "Kerned", "attrs": {"kerning": 40, "leading": 30}})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Kerned", "attrs": {"kerning": 20, "small_caps": 70}})).unwrap();
        s.execute("paraStyle.new", &json!({"name": "Punctuation", "attrs": {"mojikumi": "lineEndHalf"}})).unwrap();
        s.execute("paraStyle.setAttrs", &json!({"name": "Punctuation", "attrs": {"direction": "rightToLeft"}})).unwrap();
        for bad in [json!({"nope": 1}), json!({"justify_auto": true})] {
            assert!(s.execute("paraStyle.new", &json!({"name": "Bad", "attrs": bad})).is_err(), "{bad}");
        }
        // Captured from text with explicit leading, applied to fresh Auto-leading text.
        s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
        s.execute("charStyle.new", &json!({"name": "Spaced"})).unwrap();
        let fresh = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(runs(&s, fresh)[0].style.leading, None);
        for clear in [false, true] {
            s.execute("charStyle.apply", &json!({"name": "Spaced", "id": fresh, "clearOverrides": clear})).unwrap();
            assert_eq!(runs(&s, fresh)[0].style.leading, Some(30.0));
        }
        let copy = s.execute("charStyle.duplicate", &json!({"name": "Spaced"})).unwrap()["name"].as_str().unwrap().to_string();
        assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == copy).unwrap().attrs["leading"], 30.0);
        s.execute("charStyle.apply", &json!({"name": "Kerned", "id": fresh})).unwrap();
        let st = runs(&s, fresh)[0].style.clone();
        assert_eq!((st.kerning, st.small_caps), (Some(20.0), Some(70.0)));
        s.execute("paraStyle.apply", &json!({"name": "Punctuation", "id": id})).unwrap();
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
        assert_eq!((t.para.mojikumi, t.para.direction), (vectorcraft_doc::Mojikumi::LineEndHalf, Some(vectorcraft_doc::ParaDirection::RightToLeft)));
        // A library keeps the captured attributes and makes the style again where it's used.
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let lib = s.execute("library.add", &json!({"kind": "charStyle", "name": "Lib Spaced"})).unwrap();
        let other = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "C"})).unwrap()["id"].as_u64().unwrap();
        s.execute("library.use", &json!({"library": lib["library"], "kind": "charStyle", "item": lib["name"]})).unwrap();
        assert_eq!(runs(&s, other)[0].style.leading, Some(30.0));
    }

    #[test]
    fn paragraph_styles_and_survive_save() {
        let (mut s, id) = session_with_text("Body");
        s.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center", "space_after": 6.0}})).unwrap();
        s.execute("paraStyle.apply", &json!({"name": "Centered"})).unwrap();
        let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para.clone(),
            _ => panic!(),
        };
        assert_eq!((para(&s).justify, para(&s).space_after), (vectorcraft_doc::Justify::Center, 6.0));
        // The Normal style can be redefined too (stored once redefined).
        s.execute("paraStyle.redefine", &json!({"name": NORMAL_PARA})).unwrap();
        assert_eq!(s.execute("paraStyle.list", &json!({})).unwrap()["styles"][0]["attrs"]["justify"], "Center");
        let d = s.doc().unwrap().doc.clone();
        let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
        assert_eq!(back.para_styles, d.para_styles);
    }
}
