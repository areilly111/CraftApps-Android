//! Type → Text Variables: define, list, delete and insert variables.

use designcraft_doc::vars::{TextVariable, VarKind, var_char};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, str_param};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "variables.list", "Text Variables", [], None, "{} → [{index, name, type, …, value (page 1)}]", has_doc, |s, _| {
            let d = &s.doc()?.doc;
            Ok(Value::Array(
                d.text_variables
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let mut o = serde_json::to_value(v).unwrap_or_default();
                        o["index"] = json!(i);
                        o["value"] = json!(d.variable_value(i, Some(0)));
                        o
                    })
                    .collect(),
            ))
        }),
        cmd!(
            "variables.define",
            "Define Text Variable…",
            ["Type", "Text Variables"],
            None,
            "{name, type: custom|lastPageNumber|chapterNumber|fileName|creationDate|modificationDate|outputDate|runningHeader, text?, format?, style?, use?: firstOnPage|lastOnPage, character?, section?, extension?, before?, after?} — creates or replaces by name",
            has_doc,
            define
        ),
        cmd!(
            "variables.delete",
            "Delete Text Variable",
            [],
            None,
            "{name} — instances become plain text of their current value",
            has_doc,
            |s, p| {
                let name = str_param(p, "name").ok_or_else(|| bad("variables.delete", "missing name"))?.to_string();
                s.edit(|d, _| {
                    let i = d.variable(&name).ok_or_else(|| bad("variables.delete", format!("no variable `{name}`")))?;
                    // Replace instances with their value (first page context), then shift higher indices down.
                    let value = d.variable_value(i, Some(0)).unwrap_or_default();
                    let n = d.text_variables.len();
                    let ids: Vec<_> = d.stories.keys().copied().collect();
                    for sid in ids {
                        let Some(st) = d.story(sid) else { continue };
                        if !st.text.chars().any(|c| designcraft_doc::vars::var_index(c).is_some_and(|k| k >= i)) {
                            continue;
                        }
                        let st = d.story_mut(sid).ok_or(designcraft_doc::DocError::NoStory(sid))?;
                        let target = var_char(i).unwrap_or(' ').to_string();
                        while let Some(pos) = st.text.find(&target) {
                            st.replace(pos..pos + target.len(), &value);
                        }
                        for k in i + 1..n {
                            let (from, to) = (var_char(k).unwrap_or(' ').to_string(), var_char(k - 1).unwrap_or(' ').to_string());
                            while let Some(pos) = st.text.find(&from) {
                                st.replace(pos..pos + from.len(), &to);
                            }
                        }
                    }
                    d.text_variables.remove(i);
                    Ok(Value::Null)
                })
            }
        ),
        cmd!(
            "variables.insert",
            "Insert Variable",
            ["Type", "Text Variables"],
            None,
            "{name} — at the text insertion point",
            super::has_text,
            |s, p| {
                let name = str_param(p, "name").ok_or_else(|| bad("variables.insert", "missing name"))?;
                let i = s.doc()?.doc.variable(name).ok_or_else(|| bad("variables.insert", format!("no variable `{name}`")))?;
                let c = var_char(i).ok_or_else(|| bad("variables.insert", "too many variables"))?;
                s.execute("text.insert", &json!({"text": c.to_string()}))
            }
        ),
    ]
}

fn define(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("variables.define", "missing name"))?.to_string();
    let fmt = || str_param(p, "format").unwrap_or("MM/dd/yy").to_string();
    let kind = match str_param(p, "type").unwrap_or("custom") {
        "custom" => VarKind::Custom { text: str_param(p, "text").unwrap_or("").into() },
        "lastPageNumber" => VarKind::LastPageNumber { section: p.get("section").and_then(Value::as_bool).unwrap_or(false) },
        "chapterNumber" => VarKind::ChapterNumber,
        "fileName" => VarKind::FileName { extension: p.get("extension").and_then(Value::as_bool).unwrap_or(false) },
        "creationDate" => VarKind::CreationDate { format: fmt() },
        "modificationDate" => VarKind::ModificationDate { format: fmt() },
        "outputDate" => VarKind::OutputDate { format: fmt() },
        "runningHeader" => VarKind::RunningHeader {
            style: str_param(p, "style").ok_or_else(|| bad("variables.define", "runningHeader needs `style`"))?.into(),
            use_: if str_param(p, "use") == Some("lastOnPage") { designcraft_doc::vars::Use::LastOnPage } else { Default::default() },
            character: p.get("character").and_then(Value::as_bool).unwrap_or(false),
        },
        t => return Err(bad("variables.define", format!("unknown type `{t}`"))),
    };
    let mut v = TextVariable::new(&name, kind);
    v.before = str_param(p, "before").unwrap_or("").into();
    v.after = str_param(p, "after").unwrap_or("").into();
    s.edit(|d, _| {
        let i = match d.variable(&name) {
            Some(i) => {
                d.text_variables[i] = v.clone();
                i
            }
            None => {
                if d.text_variables.len() >= designcraft_doc::vars::VAR_MAX {
                    return Err(bad("variables.define", "too many variables"));
                }
                d.text_variables.push(v.clone());
                d.text_variables.len() - 1
            }
        };
        Ok(json!({"index": i}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_header_on_parent_page_follows_headings() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"pages": 3, "facingPages": false})).unwrap();
        s.execute("style.paragraph.create", &json!({"name": "Heading"})).ok();
        // Headings on pages 1 and 3 (page 2 carries page 1's).
        for (page, text) in [(0, "Alpha"), (2, "Epsilon")] {
            let sp = s.doc().unwrap().doc.page_loc(page).unwrap().0;
            let r = s.execute("frame.create", &json!({"spread": sp, "rect": [72, 200, 400, 260], "content": "text", "text": text})).unwrap();
            let sid = s.doc().unwrap().doc.item(designcraft_doc::ItemId(r["id"].as_u64().unwrap())).unwrap().text_frame().unwrap().story;
            s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 1})).unwrap();
            s.execute("style.paragraph.apply", &json!({"name": "Heading"})).unwrap();
        }
        s.execute("variables.define", &json!({"name": "Chapter Head", "type": "runningHeader", "style": "Heading"})).unwrap();
        s.execute("variables.define", &json!({"name": "Total", "type": "custom", "text": "of "})).unwrap();
        // A header frame on the parent with "<running header> · <page> / <last page>".
        let r = s
            .execute("frame.create", &json!({"spread": {"kind": "parent", "index": 0}, "rect": [72, 30, 500, 60], "content": "text", "text": ""}))
            .unwrap();
        let fid = designcraft_doc::ItemId(r["id"].as_u64().unwrap());
        let sid = s.doc().unwrap().doc.item(fid).unwrap().text_frame().unwrap().story;
        s.execute("text.select", &json!({"story": sid.0, "anchor": 0, "focus": 0})).unwrap();
        s.execute("variables.insert", &json!({"name": "Chapter Head"})).unwrap();
        s.execute("text.insert", &json!({"text": " · "})).unwrap();
        s.execute("variables.insert", &json!({"name": "Last Page Number"})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        assert!(matches!(d.find(fid).unwrap().spread, designcraft_doc::SpreadRef::Parent(0)));
        // Visible glyphs of the header composed for each page: "Alpha · 3" vs "Epsilon · 3".
        let shown = |page: usize| -> usize {
            let cs = s.cache.get(&d, sid, Some(&d.page_name(page)));
            cs.frames.iter().flat_map(|f| f.lines.iter()).map(|l| l.glyphs.iter().filter(|g| g.visible && g.adv > 0.0).count()).sum()
        };
        let (p1, p2, p3) = (shown(0), shown(1), shown(2));
        assert_eq!(p1, p2, "page 2 carries page 1's header");
        assert_eq!(p3, p1 + 2, "Epsilon is two letters longer than Alpha");
        let idx = s.cache.running_index(&d);
        assert_eq!(idx.value("Heading", false, Default::default(), 0), "Alpha");
        assert_eq!(idx.value("Heading", false, Default::default(), 1), "Alpha");
        assert_eq!(idx.value("Heading", false, Default::default(), 2), "Epsilon");
        let list = s.execute("variables.list", &json!({})).unwrap();
        assert!(list.as_array().unwrap().iter().any(|v| v["name"] == "Last Page Number" && v["value"] == "3"));
        // Deleting a variable flattens its instances to text.
        s.execute("variables.delete", &json!({"name": "Last Page Number"})).unwrap();
        assert!(s.doc().unwrap().doc.story(sid).unwrap().text.ends_with(" · 3"));
    }
}
