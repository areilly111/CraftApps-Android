//! Layout → Table of Contents: list the paragraphs of chosen styles with their page names, as a
//! story with "TOC Title" / "TOC Level n" styles (right tab with dot leader). Update Table of
//! Contents regenerates the same story after edits.

use designcraft_compose::{ComposeOptions, caret, compose_story};
use designcraft_doc::{Align, Document, ParaAttrs, ParaFormat, ParagraphStyle, SpreadRef, StoryId, TabAlign, TabStop, Toc, TocEntry};
use designcraft_geom::Rect;
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "toc.generate",
            "Table of Contents…",
            ["Layout"],
            None,
            "{entries: [{style, level?: 1}] | styles: [names], title?: \"Contents\", pageNumbers?: true, page?: 1-based page for a new frame (its margins), rect?: [x0,y0,x1,y1] (spread coords)} → {story, entries}",
            has_doc,
            generate
        ),
        cmd!("toc.update", "Update Table of Contents", ["Layout"], None, "{} — regenerate the existing table of contents", has_toc, |s, _| {
            s.edit(|d, _| {
                let toc = d.toc.clone().ok_or_else(|| bad("toc.update", "no table of contents"))?;
                let n = fill(d, &toc, None)?;
                Ok(json!({"story": toc.story.0, "entries": n}))
            })
        }),
        cmd!(query "toc.entries", "Table of Contents Entries", [], None, "{entries | styles} → [{level, text, page}] without changing the document", has_doc, |s, p| {
            let entries = entries_param(p)?;
            let d = &s.doc()?.doc;
            let skip = d.toc.as_ref().map(|t| t.story);
            Ok(json!(collect(d, &entries, skip).into_iter().map(|e| json!({"level": e.level, "text": e.text, "page": e.page})).collect::<Vec<_>>()))
        }),
    ]
}

fn has_toc(s: &Session) -> std::result::Result<(), String> {
    match s.doc() {
        Ok(st) if st.doc.toc.as_ref().is_some_and(|t| st.doc.story(t.story).is_some()) => Ok(()),
        _ => Err("the document has no table of contents".into()),
    }
}

fn entries_param(p: &Value) -> Result<Vec<TocEntry>> {
    if let Some(v) = p.get("entries") {
        return serde_json::from_value(v.clone()).map_err(|e| bad("toc.generate", e.to_string()));
    }
    if let Some(a) = p.get("styles").and_then(Value::as_array) {
        return Ok(a.iter().enumerate().filter_map(|(i, v)| v.as_str().map(|s| TocEntry { style: s.into(), level: (i + 1).min(9) as u8 })).collect());
    }
    Err(bad("toc.generate", "give `entries` or `styles`"))
}

pub struct Found {
    pub level: u8,
    pub text: String,
    pub page: String,
    key: (usize, i64, i64),
}

/// Paragraphs in `entries` styles across the document, in page order.
pub fn collect(d: &Document, entries: &[TocEntry], skip: Option<StoryId>) -> Vec<Found> {
    let mut out = Vec::new();
    for (sid, st) in &d.stories {
        if Some(*sid) == skip || !st.paras.iter().any(|f| entries.iter().any(|e| e.style == f.style)) {
            continue;
        }
        let cs = compose_story(d, *sid, &ComposeOptions::default());
        for (pi, r) in st.para_ranges().into_iter().enumerate() {
            let Some(e) = entries.iter().find(|e| e.style == st.paras[pi].style) else { continue };
            let text: String = st.text[r.clone()]
                .chars()
                .filter(|c| !('\u{E000}'..='\u{E1FF}').contains(c))
                .map(|c| if c == '\t' || c == '\u{2028}' { ' ' } else { c })
                .collect();
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            // Overset paragraphs have no page and are left out (as InDesign does).
            let Some((fi, x, baseline, ..)) = caret(&cs, r.start) else { continue };
            let Some(ft) = cs.frames.get(fi) else { continue };
            if r.start >= ft.range.end && r.start != ft.range.start {
                continue;
            }
            let Some(loc) = d.find(ft.frame) else { continue };
            let SpreadRef::Doc(si) = loc.spread else { continue };
            let Some(item) = d.item(ft.frame) else { continue };
            let b = item.bounds();
            let in_spread = d.spreads[si].page_at_x(b.center().x).unwrap_or(0);
            let abs = d.first_page_of_spread(si) + in_spread;
            out.push(Found {
                level: e.level,
                text,
                page: d.page_name(abs),
                key: (abs, ((b.y0 + baseline) * 10.0) as i64, ((b.x0 + x) * 10.0) as i64),
            });
        }
    }
    out.sort_by_key(|f| f.key);
    out
}

fn ensure_styles(d: &mut Document, levels: u8, width: f64) {
    let mut want = vec![("TOC Title".to_string(), ParaAttrs { space_after: Some(12.0), ..Default::default() }, Some(16.0))];
    for l in 1..=levels.max(1) {
        let indent = (l as f64 - 1.0) * 12.0;
        want.push((
            format!("TOC Level {l}"),
            ParaAttrs {
                left_indent: Some(indent),
                space_before: Some(if l == 1 { 4.0 } else { 0.0 }),
                align: Some(Align::Left),
                tabs: Some(vec![TabStop { position: width.max(36.0), align: TabAlign::Right, leader: ". ".into(), align_on: String::new() }]),
                ..Default::default()
            },
            None,
        ));
    }
    let missing: Vec<_> = want.into_iter().filter(|(n, ..)| d.styles.para(n).is_none()).collect();
    if missing.is_empty() {
        return;
    }
    let st = d.styles_mut();
    for (name, para, size) in missing {
        let chars = designcraft_doc::CharAttrs { size, ..Default::default() };
        st.paragraph.push(ParagraphStyle {
            name,
            based_on: Some(designcraft_doc::story::BASIC_PARAGRAPH.into()),
            next_style: None,
            para,
            chars,
            shortcut: String::new(),
        });
    }
}

/// Rewrite the TOC story; returns the number of entries.
fn fill(d: &mut Document, toc: &Toc, width: Option<f64>) -> Result<usize> {
    let found = collect(d, &toc.entries, Some(toc.story));
    let levels = toc.entries.iter().map(|e| e.level).max().unwrap_or(1);
    let width = width.unwrap_or_else(|| {
        d.story(toc.story).and_then(|st| st.frames.first()).and_then(|f| d.item(*f)).map(|i| i.text_area().width()).unwrap_or(300.0)
    });
    ensure_styles(d, levels, width);
    let mut text = String::new();
    let mut styles = Vec::new();
    if !toc.title.is_empty() {
        text.push_str(&toc.title);
        styles.push("TOC Title".to_string());
    }
    for f in &found {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&f.text);
        if toc.page_numbers {
            text.push('\t');
            text.push_str(&f.page);
        }
        styles.push(format!("TOC Level {}", f.level.max(1)));
    }
    let st = d.story_mut(toc.story).ok_or(designcraft_doc::DocError::NoStory(toc.story))?;
    let len = st.len();
    st.replace(0..len, &text);
    for (pi, r) in st.para_ranges().into_iter().enumerate() {
        if let Some(name) = styles.get(pi) {
            st.format_paras(r.start..r.start, |f| *f = ParaFormat { style: name.clone(), ..Default::default() });
        }
    }
    Ok(found.len())
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let entries = entries_param(p)?;
    if entries.is_empty() {
        return Err(bad("toc.generate", "no entries"));
    }
    let title = str_param(p, "title").unwrap_or("Contents").to_string();
    let page_numbers = p.get("pageNumbers").and_then(Value::as_bool).unwrap_or(true);
    let rect = super::rect_param(p, "rect");
    let page = p.get("page").and_then(Value::as_u64).unwrap_or(1).max(1) as usize - 1;
    s.edit(|d, sel| {
        // Reuse the existing TOC story when there is one; otherwise make a frame.
        let existing = d.toc.as_ref().map(|t| t.story).filter(|sid| d.story(*sid).is_some());
        let (story, width) = match existing {
            Some(sid) => (sid, None),
            None => {
                let (si, pi) = d.page_loc(page).ok_or_else(|| bad("toc.generate", format!("no page {}", page + 1)))?;
                let r: Rect = rect.unwrap_or_else(|| d.spreads[si].pages[pi].margin_rect());
                let layer = d.default_layer();
                let (fid, sid) = d.add_text_frame(SpreadRef::Doc(si), r, layer, "", ParaFormat::default())?;
                *sel = designcraft_doc::Selection::items(vec![fid]);
                let w = d.item(fid).map(|i| i.text_area().width());
                (sid, w)
            }
        };
        let toc = Toc { story, title: title.clone(), entries: entries.clone(), page_numbers };
        let n = fill(d, &toc, width)?;
        d.toc = Some(toc);
        Ok(json!({"story": story.0, "entries": n}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_lists_headings_in_page_order_and_updates() {
        let mut s = Session::new();
        s.execute("file.newSample", &json!({})).unwrap();
        let pages = s.doc().unwrap().doc.page_count();
        s.execute("layout.pages.insert", &json!({"count": 1})).unwrap();
        let r = s.execute("toc.generate", &json!({"styles": ["Headline", "Cover Title"], "page": pages + 1})).unwrap();
        assert!(r["entries"].as_u64().unwrap() >= 2, "{r}");
        let d = &s.doc().unwrap().doc;
        let sid = StoryId(r["story"].as_u64().unwrap());
        let text = d.story(sid).unwrap().text.clone();
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "Contents");
        // Cover title (page 1) comes before the feature headline (page 2+).
        let cover = lines.iter().position(|l| l.starts_with("The Quiet")).unwrap();
        let head = lines.iter().position(|l| l.starts_with("Notes on the Grid")).unwrap();
        assert!(cover < head);
        assert!(lines[head].ends_with("\t2") || lines[head].contains('\t'), "{}", lines[head]);
        assert_eq!(d.story(sid).unwrap().paras[head].style, "TOC Level 1");
        assert!(d.styles.para("TOC Level 2").is_some());
        // Edit a heading, then update.
        let hid = d.stories.iter().find(|(_, st)| st.text.starts_with("Notes on the Grid")).map(|(id, _)| *id).unwrap();
        s.execute("story.replaceRange", &json!({"story": hid.0, "start": 0, "end": 5, "text": "Thoughts"})).unwrap();
        s.execute("toc.update", &json!({})).unwrap();
        let text = s.doc().unwrap().doc.story(sid).unwrap().text.clone();
        assert!(text.contains("Thoughts on the Grid"), "{text}");
        s.doc().unwrap().doc.check().unwrap();
    }
}
