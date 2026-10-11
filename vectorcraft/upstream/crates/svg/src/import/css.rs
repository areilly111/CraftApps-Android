//! The style cascade for the `<text>` pre-pass: `<style>` rules, the `style` attribute and
//! presentation attributes.
//!
//! Selectors: type, `*`, `#id`, any number of `.class`es and `[attr]` / `[attr=value]`, combined by
//! descendant (space) and child (`>`) combinators. Rules with other selectors (pseudo-classes,
//! sibling combinators, namespaces) never match, and at-rule blocks (`@media`, `@font-face`…) are
//! skipped. Precedence: `!important`, then the `style` attribute, then specificity, then source
//! order; a presentation attribute applies only when no rule sets the property.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use usvg::roxmltree;

pub(super) type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

/// `name: value` declarations of a block or `style` attribute, with their `!important` flag.
fn parse_decls(s: &str) -> Vec<(String, String, bool)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim();
            let (v, important) = match v.strip_suffix("important").map(str::trim_end).and_then(|v| v.strip_suffix('!')) {
                Some(v) => (v.trim_end(), true),
                None => (v, false),
            };
            Some((k.trim().to_ascii_lowercase(), v.to_string(), important))
        })
        .collect()
}

/// One compound selector (`text.a.b#c[x=y]`).
#[derive(Debug, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
}

impl Compound {
    fn matches(&self, n: XNode) -> bool {
        self.tag.as_deref().is_none_or(|t| n.tag_name().name() == t)
            && self.id.as_deref().is_none_or(|id| n.attribute("id") == Some(id))
            && (self.classes.is_empty() || n.attribute("class").is_some_and(|c| self.classes.iter().all(|k| c.split_whitespace().any(|x| x == k))))
            && self.attrs.iter().all(|(a, v)| n.attribute(a.as_str()).is_some_and(|x| v.as_deref().is_none_or(|v| x == v)))
    }
}

/// A complex selector, rightmost compound first; each compound's flag says whether it must be the
/// child (true) or a descendant (false) of the next one.
#[derive(Debug)]
struct Selector {
    parts: Vec<(Compound, bool)>,
    /// (ids, classes and attributes, types).
    specificity: (u32, u32, u32),
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

impl Selector {
    /// `None` for selectors this cascade doesn't support (they never match).
    fn parse(s: &str) -> Option<Self> {
        fn ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
            let mut out = String::new();
            while let Some(&c) = chars.peek().filter(|c| is_ident(**c)) {
                out.push(c);
                chars.next();
            }
            (!out.is_empty()).then_some(out)
        }
        // Compounds left to right; `links[i]` joins compounds i and i + 1 (true = child).
        let mut comps: Vec<Compound> = vec![];
        let mut links: Vec<bool> = vec![];
        let mut cur: Option<Compound> = None;
        let mut link: Option<bool> = None;
        let mut chars = s.trim().chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '>' {
                chars.next();
                comps.extend(cur.take());
                if comps.is_empty() {
                    return None;
                }
                link = Some(c == '>' || link == Some(true));
                continue;
            }
            let comp = match &mut cur {
                Some(comp) => comp,
                None => {
                    if !comps.is_empty() {
                        links.push(link.take()?);
                    }
                    cur.insert(Compound::default())
                }
            };
            match c {
                '*' => {
                    chars.next();
                }
                '#' => {
                    chars.next();
                    comp.id = Some(ident(&mut chars)?);
                }
                '.' => {
                    chars.next();
                    comp.classes.push(ident(&mut chars)?);
                }
                '[' => {
                    chars.next();
                    let body: String = chars.by_ref().take_while(|c| *c != ']').collect();
                    let (name, value) = match body.split_once('=') {
                        Some((n, v)) => (n.trim(), Some(v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())),
                        None => (body.trim(), None),
                    };
                    if name.is_empty() || !name.chars().all(is_ident) {
                        return None;
                    }
                    comp.attrs.push((name.to_string(), value));
                }
                c if is_ident(c) => comp.tag = Some(ident(&mut chars)?),
                _ => return None,
            }
        }
        // A trailing `>` has nothing to apply to.
        if cur.is_none() && link == Some(true) {
            return None;
        }
        comps.extend(cur);
        if comps.is_empty() {
            return None;
        }
        let mut specificity = (0, 0, 0);
        for c in &comps {
            specificity.0 += c.id.is_some() as u32;
            specificity.1 += (c.classes.len() + c.attrs.len()) as u32;
            specificity.2 += c.tag.is_some() as u32;
        }
        // Rightmost first, each with its link to the compound on its left.
        let parts = comps.into_iter().enumerate().rev().map(|(i, c)| (c, i.checked_sub(1).and_then(|k| links.get(k)).is_some_and(|l| *l))).collect();
        Some(Self { parts, specificity })
    }

    fn matches(&self, n: XNode) -> bool {
        fn go(parts: &[(Compound, bool)], n: XNode) -> bool {
            let Some(((c, child), rest)) = parts.split_first() else { return true };
            if !c.matches(n) {
                return false;
            }
            if rest.is_empty() {
                return true;
            }
            if *child {
                n.parent_element().is_some_and(|p| go(rest, p))
            } else {
                n.ancestors().skip(1).filter(|a| a.is_element()).any(|a| go(rest, a))
            }
        }
        go(&self.parts, n)
    }
}

struct Rule {
    selector: Selector,
    decls: Rc<Vec<(String, String, bool)>>,
}

/// Rules from every `<style>` element, in source order.
fn parse_sheet(css: &str, out: &mut Vec<Rule>) {
    // Drop comments.
    let mut text = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find("*/").map_or("", |j| &rest[i + 2 + j + 2..]);
    }
    text.push_str(rest);
    let mut s = text.as_str();
    loop {
        s = s.trim_start();
        if s.is_empty() {
            break;
        }
        let Some(open) = s.find('{') else { break };
        let head = s[..open].trim();
        if head.starts_with('@') {
            // Statement at-rules end at `;` before any block.
            if let Some(semi) = s.find(';').filter(|&semi| semi < open) {
                s = &s[semi + 1..];
                continue;
            }
            // Skip the (possibly nested) block.
            let mut depth = 0usize;
            let mut end = s.len();
            for (i, c) in s[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            s = &s[end..];
            continue;
        }
        let close = s[open..].find('}').map_or(s.len(), |i| open + i);
        let decls = Rc::new(parse_decls(&s[open + 1..close]));
        for sel in head.split(',') {
            if let Some(selector) = Selector::parse(sel) {
                out.push(Rule { selector, decls: decls.clone() });
            }
        }
        s = s.get(close + 1..).unwrap_or("");
    }
}

/// The cascade over one parsed document.
pub(super) struct Styles {
    rules: Vec<Rule>,
    /// Declared properties of each element queried so far.
    cache: RefCell<HashMap<roxmltree::NodeId, Rc<HashMap<String, String>>>>,
}

impl Styles {
    pub(super) fn new(doc: &roxmltree::Document) -> Self {
        let mut rules = vec![];
        for st in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "style") {
            if st.attribute("type").is_some_and(|t| !t.trim().is_empty() && t.trim() != "text/css") {
                continue;
            }
            let css: String = st.children().filter_map(|c| c.text()).collect();
            parse_sheet(&css, &mut rules);
        }
        Self { rules, cache: RefCell::default() }
    }

    /// Properties set on `n` by rules and its `style` attribute, after the cascade.
    fn declared(&self, n: XNode) -> Rc<HashMap<String, String>> {
        if let Some(m) = self.cache.borrow().get(&n.id()) {
            return m.clone();
        }
        // (important, inline, specificity, order) → highest wins.
        type Key = (bool, bool, (u32, u32, u32), usize);
        let mut best: HashMap<String, (Key, String)> = HashMap::new();
        let mut put = |name: &str, value: &str, key: Key| {
            if best.get(name).is_none_or(|(k, _)| key >= *k) {
                best.insert(name.to_string(), (key, value.to_string()));
            }
        };
        let mut order = 0;
        for r in self.rules.iter().filter(|r| r.selector.matches(n)) {
            for (k, v, imp) in r.decls.iter() {
                order += 1;
                put(k, v, (*imp, false, r.selector.specificity, order));
            }
        }
        if let Some(st) = n.attribute("style") {
            for (k, v, imp) in parse_decls(st) {
                order += 1;
                put(&k, &v, (imp, true, (0, 0, 0), order));
            }
        }
        let m: Rc<HashMap<String, String>> = Rc::new(best.into_iter().map(|(k, (_, v))| (k, v)).collect());
        self.cache.borrow_mut().insert(n.id(), m.clone());
        m
    }

    /// A property specified on this element (rules and `style` beat the presentation attribute).
    pub(super) fn own(&self, n: XNode, name: &str) -> Option<String> {
        self.declared(n).get(name).cloned().or_else(|| n.attribute(name).map(|v| v.trim().to_string()))
    }

    /// An inherited property: the nearest specified value other than `inherit`.
    pub(super) fn prop(&self, n: XNode, name: &str) -> Option<String> {
        n.ancestors().filter(|a| a.is_element()).find_map(|a| self.own(a, name).filter(|v| v != "inherit"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_doc(svg: &str, f: impl FnOnce(&roxmltree::Document, &Styles)) {
        let doc = roxmltree::Document::parse(svg).unwrap();
        let st = Styles::new(&doc);
        f(&doc, &st);
    }

    fn by_id<'a, 'i>(doc: &'a roxmltree::Document<'i>, id: &str) -> XNode<'a, 'i> {
        doc.descendants().find(|n| n.attribute("id") == Some(id)).unwrap()
    }

    #[test]
    fn selectors_and_precedence() {
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            /* comment { fill: red } */
            @media print { text { fill: red } }
            @import url(x.css);
            text { fill: #111; font-size: 9px }
            g text { fill: #222 }
            g > text.a { fill: #333 }
            .a.b { stroke: blue }
            #t2 { fill: #444 }
            [data-k=v] { font-weight: bold }
            text:first-child, text ~ text { fill: red }
            .imp { fill: #555 !important }
            </style>
            <text id="t0">x</text>
            <g><text id="t1" class="a b" fill="green">x</text>
               <g><text id="t2" class="a" style="fill:#666">x</text></g>
               <text id="t3" class="a imp" style="fill:#777" data-k="v">x</text></g>
            </svg>"#,
            |doc, st| {
                assert_eq!(st.own(by_id(doc, "t0"), "fill").as_deref(), Some("#111"));
                assert_eq!(st.own(by_id(doc, "t1"), "fill").as_deref(), Some("#333"), "child + class beats descendant");
                assert_eq!(st.own(by_id(doc, "t1"), "stroke").as_deref(), Some("blue"), "multiple classes");
                assert_eq!(st.own(by_id(doc, "t2"), "fill").as_deref(), Some("#666"), "style attribute beats #id");
                assert_eq!(st.own(by_id(doc, "t3"), "fill").as_deref(), Some("#555"), "!important beats style");
                assert_eq!(st.own(by_id(doc, "t3"), "font-weight").as_deref(), Some("bold"));
                assert_eq!(st.own(by_id(doc, "t2"), "stroke"), None, "`.a.b` needs both classes");
                assert_eq!(st.prop(by_id(doc, "t2"), "font-size").as_deref(), Some("9px"));
            },
        );
    }

    #[test]
    fn presentation_attributes_lose_to_rules() {
        with_doc(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>#a { fill: blue }</style><text id="a" fill="red" stroke="red">x</text></svg>"#,
            |doc, st| {
                assert_eq!(st.own(by_id(doc, "a"), "fill").as_deref(), Some("blue"));
                assert_eq!(st.own(by_id(doc, "a"), "stroke").as_deref(), Some("red"));
            },
        );
    }

    #[test]
    fn unsupported_selectors_are_dropped() {
        for s in ["a + b", "a ~ b", "a:hover", "svg|text", "> a", "a >", ""] {
            assert!(Selector::parse(s).is_none(), "{s}");
        }
        let s = Selector::parse("g  >  text.x#y").unwrap();
        assert_eq!(s.specificity, (1, 1, 2));
        assert_eq!(s.parts.len(), 2);
        assert!(s.parts[0].1, "text must be a child of g");
    }
}
