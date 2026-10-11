//! Window → Type → Tabs: tab stops of the selected text objects' paragraphs.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TabAlign, TabStop};

use super::typecmd::{TextRange, para_span, refresh_bounds, text_targets};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "text.tabs.set",
            "Set Tab Stops",
            ["Window", "Tabs"],
            None,
            "{stops: [{position: pt, align?: left|center|right|decimal, leader?: \". \", alignOn?: \".\"}], ids?, start?: byte, end?: byte} replace the tab stops of the selected text (with a range: of the paragraphs it touches)",
            has_selection,
            set
        ),
        cmd!(
            query "text.tabs.get",
            "Tab Stops",
            [],
            None,
            "{ids?, start?: byte} → {stops} of the first selected text object (its paragraph at `start`; default the first)",
            has_selection,
            get
        ),
        cmd!("text.tabs.clear", "Clear All Tabs", ["Window", "Tabs"], None, "{ids?, start?: byte, end?: byte}", has_selection, |s, p| apply(
            s,
            p,
            vec![],
            "Clear All Tabs"
        )),
    ]
}

fn parse_stops(p: &Value) -> Result<Vec<TabStop>> {
    const C: &str = "text.tabs.set";
    let arr = p.get("stops").and_then(Value::as_array).ok_or_else(|| bad(C, "missing stops"))?;
    let mut stops = vec![];
    for s in arr {
        let position = s.get("position").and_then(Value::as_f64).ok_or_else(|| bad(C, "each stop needs a position"))?;
        if !(0.0..=100_000.0).contains(&position) {
            return Err(bad(C, "positions must be between 0 and 100000 pt"));
        }
        let align = match s.get("align").and_then(Value::as_str).unwrap_or("left").to_ascii_lowercase().as_str() {
            "left" => TabAlign::Left,
            "center" | "centre" => TabAlign::Center,
            "right" => TabAlign::Right,
            "decimal" => TabAlign::Decimal,
            other => return Err(bad(C, format!("unknown align `{other}`"))),
        };
        let leader = s.get("leader").and_then(Value::as_str).unwrap_or("").chars().take(8).collect();
        let align_on = s.get("alignOn").and_then(Value::as_str).and_then(|a| a.chars().next()).unwrap_or('.');
        stops.push(TabStop { position, align, leader, align_on });
    }
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    stops.dedup_by(|a, b| (a.position - b.position).abs() < 1e-6);
    Ok(stops)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let stops = parse_stops(p)?;
    apply(s, p, stops, "Tab Stops")
}

fn apply(s: &mut Session, p: &Value, stops: Vec<TabStop>, label: &str) -> Result<Value> {
    let ids = text_targets(s, p, "text.tabs.set")?;
    let range = TextRange::parse(p, "text.tabs.set")?;
    let n = stops.len();
    s.edit(label, |d, _| {
        for id in &ids {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                let span = para_span(range, t);
                t.edit_paras(span, |pa| pa.tabs = stops.clone());
                refresh_bounds(t);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "stops": n }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = text_targets(s, p, "text.tabs.get")?;
    let range = TextRange::parse(p, "text.tabs.get")?;
    let st = s.doc()?;
    let tabs = ids.iter().find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => Some(t.para_at(para_span(range, t).map_or(0, |r| r.start)).tabs.clone()),
        _ => None,
    });
    Ok(json!({ "stops": serde_json::to_value(tabs.unwrap_or_default()).unwrap_or(Value::Null) }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn set_get_clear_tabs() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("text.create", &json!({"x": 10, "y": 20, "text": "Item\t9.99"})).unwrap();
        s.execute("text.tabs.set", &json!({"stops": [{"position": 200, "align": "decimal"}, {"position": 80, "leader": "."}]})).unwrap();
        let g = s.execute("text.tabs.get", &json!({})).unwrap();
        assert_eq!(g["stops"][0]["position"], json!(80.0));
        assert_eq!(g["stops"][0]["leader"], json!("."));
        assert_eq!(g["stops"][1]["align"], json!("decimal"));
        assert!(s.execute("text.tabs.set", &json!({"stops": [{"position": 10, "align": "diagonal"}]})).is_err());
        s.execute("text.tabs.clear", &json!({})).unwrap();
        assert_eq!(s.execute("text.tabs.get", &json!({})).unwrap()["stops"], json!([]));
    }
}
