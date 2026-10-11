//! Window › Type & Tables › Conditional Text: conditions, applying them to text, showing and
//! hiding them.

use std::sync::Arc;

use designcraft_doc::{Condition, Document, Story};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, has_text, ok, str_param};

/// Indicator colours given to new conditions in turn (light, like the Layers panel's).
const COLORS: [[u8; 3]; 6] = [[79, 153, 255], [255, 79, 79], [79, 255, 79], [255, 179, 79], [179, 79, 255], [79, 230, 230]];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "condition.list", "Conditions", [], None, "{} → [{name, color, visible}]", has_doc, |s, _| {
            Ok(serde_json::to_value(&s.doc()?.doc.conditions).unwrap_or_default())
        }),
        cmd!(
            "condition.new",
            "New Condition…",
            ["Window", "Type & Tables", "Conditional Text"],
            None,
            "{name, color?: [r, g, b], visible?} → {name}",
            has_doc,
            |s, p| {
                let name =
                    str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("condition.new", "`name` required"))?.to_string();
                let color = color_param(p);
                let visible = p.get("visible").and_then(Value::as_bool).unwrap_or(true);
                s.edit(|d, _| {
                    if d.conditions.iter().any(|c| c.name == name) {
                        return Err(bad("condition.new", format!("a condition named `{name}` exists")));
                    }
                    let color = color.unwrap_or(COLORS[d.conditions.len() % COLORS.len()]);
                    d.conditions.push(Condition { name: name.clone(), color, visible });
                    Ok(json!({"name": name}))
                })
            }
        ),
        cmd!("condition.options", "Condition Options…", [], None, "{name, to?: new name, color?: [r, g, b], visible?: bool}", has_doc, |s, p| {
            let name = str_param(p, "name").unwrap_or("").to_string();
            let to = str_param(p, "to").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
            let color = color_param(p);
            let visible = p.get("visible").and_then(Value::as_bool);
            s.edit(|d, _| {
                if let Some(t) = &to
                    && *t != name
                    && d.conditions.iter().any(|c| c.name == *t)
                {
                    return Err(bad("condition.options", format!("a condition named `{t}` exists")));
                }
                let c = d.conditions.iter_mut().find(|c| c.name == name).ok_or_else(|| bad("condition.options", format!("no condition `{name}`")))?;
                if let Some(v) = color {
                    c.color = v;
                }
                if let Some(v) = visible {
                    c.visible = v;
                }
                if let Some(t) = to.filter(|t| *t != name) {
                    c.name = t.clone();
                    rename_everywhere(d, &name, Some(&t));
                }
                ok()
            })
        }),
        cmd!("condition.delete", "Delete Condition", [], None, "{name} — the text it was applied to stays (shown)", has_doc, |s, p| {
            let name = str_param(p, "name").unwrap_or("").to_string();
            s.edit(|d, _| {
                let n = d.conditions.len();
                d.conditions.retain(|c| c.name != name);
                if d.conditions.len() == n {
                    return Err(bad("condition.delete", format!("no condition `{name}`")));
                }
                rename_everywhere(d, &name, None);
                ok()
            })
        }),
        cmd!(
            "condition.apply",
            "Apply Condition",
            [],
            None,
            "{name, on?: bool (default true), only?: bool (remove the others)} — to the selected text; `name: null` with `only` removes all",
            has_text,
            |s, p| {
                let name = str_param(p, "name").map(str::to_string);
                let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
                let only = p.get("only").and_then(Value::as_bool).unwrap_or(false);
                if let Some(n) = &name
                    && !s.doc()?.doc.conditions.iter().any(|c| c.name == *n)
                {
                    return Err(bad("condition.apply", format!("no condition `{n}`")));
                }
                let targets = super::text::format_targets(s);
                s.edit(|d, _| {
                    for t in &targets {
                        let Some(st) = d.text_story_mut(t.story, t.cell) else { continue };
                        if t.range.is_empty() {
                            continue;
                        }
                        st.format_chars(t.range.clone(), |f| {
                            let mut v = if only { Vec::new() } else { f.over.conditions.clone().unwrap_or_default() };
                            if let Some(n) = &name {
                                v.retain(|x| x != n);
                                if on {
                                    v.push(n.clone());
                                }
                            }
                            f.over.conditions = if v.is_empty() { None } else { Some(v) };
                        });
                    }
                    ok()
                })
            }
        ),
    ]
}

fn color_param(p: &Value) -> Option<[u8; 3]> {
    let a = p.get("color")?.as_array()?;
    let c = |i: usize| a.get(i).and_then(Value::as_f64).map(|v| v.clamp(0.0, 255.0) as u8);
    Some([c(0)?, c(1)?, c(2)?])
}

/// Rename (or with `None`, remove) a condition in every story's text, table cells included.
fn rename_everywhere(d: &mut Document, from: &str, to: Option<&str>) {
    fn fix(st: &mut Story, from: &str, to: Option<&str>) {
        let len = st.text.len();
        if st.chars.iter().any(|r| r.format.over.conditions.as_ref().is_some_and(|v| v.iter().any(|x| x == from))) {
            st.format_chars(0..len, |f| {
                if let Some(v) = &mut f.over.conditions {
                    match to {
                        Some(t) => v.iter_mut().filter(|x| *x == from).for_each(|x| *x = t.to_string()),
                        None => v.retain(|x| x != from),
                    }
                    if v.is_empty() {
                        f.over.conditions = None;
                    }
                }
            });
        }
        for t in st.tables.values_mut() {
            for c in &mut Arc::make_mut(t).cells {
                fix(&mut c.text, from, to);
            }
        }
    }
    for st in d.stories.values_mut() {
        fix(Arc::make_mut(st), from, to);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn hidden_condition_removes_text_from_the_layout() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("frame.create", &json!({"rect": [72, 72, 400, 300], "content": "text", "text": "Price: $10 / €9"})).unwrap();
        let sid = designcraft_doc::StoryId(r["story"].as_u64().unwrap());
        s.execute("condition.new", &json!({"name": "EU"})).unwrap();
        let eu = "Price: $10 / ".len();
        s.execute("text.select", &json!({"story": r["story"], "anchor": eu, "focus": "Price: $10 / €9".len()})).unwrap();
        s.execute("condition.apply", &json!({"name": "EU"})).unwrap();
        let end_x = |s: &Session| s.cache.get(&s.doc().unwrap().doc, sid, None).frames[0].lines[0].end_x;
        let shown = end_x(&s);
        s.execute("condition.options", &json!({"name": "EU", "visible": false})).unwrap();
        let hidden = end_x(&s);
        assert!(hidden < shown - 5.0, "{shown} → {hidden}");
        // Every byte still has a glyph (caret positions survive).
        let cs = s.cache.get(&s.doc().unwrap().doc, sid, None);
        assert!(cs.frames[0].lines[0].glyphs.iter().any(|g| g.byte >= eu && !g.visible));
        let html = s.execute("file.exportHtml", &json!({})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(html.contains("$10") && !html.contains("€9"), "hidden text isn't exported");
        let back = designcraft_idml::import_idml(&designcraft_idml::export_idml(&s.doc().unwrap().doc)).unwrap();
        assert_eq!(back.conditions, s.doc().unwrap().doc.conditions, "IDML Condition");
        let bs = back.stories.values().find(|st| st.text.contains("€9")).unwrap();
        assert!(bs.chars.iter().any(|r| r.format.over.conditions.as_deref() == Some(&["EU".to_string()][..])), "AppliedConditions");
        s.execute("condition.options", &json!({"name": "EU", "to": "Europe"})).unwrap();
        let runs = &s.doc().unwrap().doc.stories[&sid].chars;
        assert!(runs.iter().any(|r| r.format.over.conditions.as_deref() == Some(&["Europe".to_string()][..])));
        s.execute("condition.delete", &json!({"name": "Europe"})).unwrap();
        assert!((end_x(&s) - shown).abs() < 0.01, "text shows again");
        assert!(s.doc().unwrap().doc.stories[&sid].chars.iter().all(|r| r.format.over.conditions.is_none()));
    }
}
