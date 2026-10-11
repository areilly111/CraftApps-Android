//! On-canvas text editing and per-character styles through commands: run-scoped
//! `layer.setText`, text.insert / delete / setSelection / moveCaret, the text clipboard and the
//! paste-formatting commands, with undo / redo.

use effectcraft_keyframe::{BaselineOption, Justify, Kerning, TextDoc};
use serde_json::json;

use crate::Session;
use crate::commands::text_edit::layer_doc;

fn session(text: &str) -> (Session, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "C", "width": 640, "height": 360, "duration": 4})).unwrap();
    let t = s.execute("layer.newText", json!({"text": text, "size": 60, "justify": "left"})).unwrap()["layer"].as_u64().unwrap();
    (s, t)
}

fn doc(s: &Session, t: u64) -> TextDoc {
    layer_doc(s, effectcraft_project::LayerId(t)).unwrap()
}

fn sel(s: &Session) -> (usize, usize) {
    let e = s.state.text_edit.as_ref().unwrap();
    (e.anchor, e.caret)
}

#[test]
fn set_text_with_a_range_creates_runs_and_undoes() {
    let (mut s, t) = session("Hello World");
    s.execute("layer.setText", json!({"layer": t, "range": [6, 11], "size": 30, "fill": "#ff0000", "kerning": "optical"})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.runs().len(), 2);
    assert_eq!(d.style_at(0).size, 60.0);
    assert_eq!(d.style_at(7).size, 30.0);
    assert_eq!(d.style_at(7).fill, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(d.style_at(7).kerning, Kerning::Optical);
    // Paragraph attributes apply to the paragraphs the range touches; document ones to all.
    s.execute("layer.setText", json!({"layer": t, "range": [0, 1], "justify": "justifyAll", "indentFirst": 20, "spaceAfter": 6, "direction": "rtl"})).unwrap();
    let d = doc(&s, t);
    assert_eq!((d.justify, d.indent_first, d.space_after), (Justify::JustifyAll, 20.0, 6.0));
    // Unknown attributes and bad values are errors (nothing changes).
    assert!(s.execute("layer.setText", json!({"layer": t, "sizee": 3})).is_err());
    assert!(s.execute("layer.setText", json!({"layer": t, "size": "big"})).is_err());
    assert!(s.execute("edit.undo", json!({})).is_ok());
    assert_eq!(doc(&s, t).justify, Justify::Left);
    assert!(s.execute("edit.undo", json!({})).is_ok());
    assert!(doc(&s, t).is_uniform());
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(doc(&s, t).runs().len(), 2);
    // Without a range everything changes and the runs collapse again.
    s.execute("layer.setText", json!({"layer": t, "size": 44, "superscript": true})).unwrap();
    let d = doc(&s, t);
    assert!(d.runs().iter().all(|r| r.style.size == 44.0 && r.style.baseline == BaselineOption::Superscript));
    // Replacing a range's text keeps the formatting around it.
    s.execute("layer.setText", json!({"layer": t, "range": [0, 5], "text": "Howdy", "fauxBold": true})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.text, "Howdy World");
    assert!(d.style_at(2).faux_bold && !d.style_at(7).faux_bold);
}

#[test]
fn typing_deleting_and_caret_movement() {
    let (mut s, t) = session("abc def");
    s.execute("text.edit", json!({"layer": t})).unwrap();
    assert_eq!(sel(&s), (0, 7), "editing starts with everything selected");
    s.execute("text.setSelection", json!({"start": 3})).unwrap();
    s.execute("text.insert", json!({"text": "XY", "merge": "typing-1"})).unwrap();
    s.execute("text.insert", json!({"text": "Z", "merge": "typing-1"})).unwrap();
    assert_eq!(doc(&s, t).text, "abcXYZ def");
    assert_eq!(sel(&s), (6, 6));
    // One undo step for the merged typing.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s, t).text, "abc def");
    assert_eq!(sel(&s), (6, 6));
    s.execute("edit.redo", json!({})).unwrap();
    s.execute("text.setSelection", json!({"start": 6})).unwrap();
    // Backspace / Delete, by character and by word.
    s.execute("text.delete", json!({})).unwrap();
    assert_eq!(doc(&s, t).text, "abcXY def");
    s.execute("text.delete", json!({"direction": "forward", "word": true})).unwrap();
    assert_eq!(doc(&s, t).text, "abcXY");
    s.execute("text.delete", json!({"word": true})).unwrap();
    assert_eq!(doc(&s, t).text, "");
    s.execute("text.insert", json!({"text": "one two\nthree"})).unwrap();
    assert_eq!(sel(&s), (13, 13));
    // Caret movement: words, lines, up / down, extend.
    let mv = |s: &mut Session, to: &str, extend: bool| -> (usize, usize) {
        let r = s.execute("text.moveCaret", json!({"to": to, "extend": extend})).unwrap();
        (r["anchor"].as_u64().unwrap() as usize, r["caret"].as_u64().unwrap() as usize)
    };
    assert_eq!(mv(&mut s, "lineStart", false), (8, 8));
    assert_eq!(mv(&mut s, "up", false).1, 0);
    assert_eq!(mv(&mut s, "wordRight", false), (3, 3));
    assert_eq!(mv(&mut s, "wordRight", true), (3, 7));
    assert_eq!(mv(&mut s, "left", false), (3, 3), "left collapses a selection to its start");
    assert_eq!(mv(&mut s, "lineEnd", false), (7, 7));
    assert_eq!(mv(&mut s, "down", false).1, 13);
    assert_eq!(mv(&mut s, "start", true), (13, 0));
    assert_eq!(mv(&mut s, "paraEnd", false), (7, 7));
    // Select All while editing selects the text, not the layers.
    s.execute("edit.selectAll", json!({})).unwrap();
    assert_eq!(sel(&s), (0, 13));
    // Undo past the insertion clamps the selection.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(sel(&s).0 <= doc(&s, t).char_len() && sel(&s).1 <= doc(&s, t).char_len());
    s.execute("text.endEdit", json!({})).unwrap();
    assert!(s.state.text_edit.is_none());
}

#[test]
fn caret_style_applies_to_the_next_typed_text() {
    let (mut s, t) = session("ab");
    s.execute("text.edit", json!({"layer": t, "caret": 1})).unwrap();
    s.execute("layer.setText", json!({"layer": t, "range": [1, 1], "size": 12, "fill": [0, 1, 0]})).unwrap();
    assert!(doc(&s, t).is_uniform(), "a caret-only change doesn't touch the text");
    s.execute("text.insert", json!({"text": "Q"})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.text, "aQb");
    assert_eq!(d.style_at(1).size, 12.0);
    assert_eq!(d.style_at(1).fill, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(d.style_at(2).size, 60.0);
    // Typing again continues the new style (the character before the caret).
    s.execute("text.insert", json!({"text": "R"})).unwrap();
    assert_eq!(doc(&s, t).style_at(2).size, 12.0);
}

#[test]
fn clipboard_and_paste_formatting() {
    let (mut s, t) = session("red plain");
    s.execute("layer.setText", json!({"layer": t, "range": [0, 3], "fill": "#ff0000", "size": 20})).unwrap();
    s.execute("text.edit", json!({"layer": t, "select": [0, 3]})).unwrap();
    let r = s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(r["text"], "red");
    // Paste keeps the copied formatting.
    s.execute("text.setSelection", json!({"start": 9})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.text, "red plainred");
    assert_eq!(d.style_at(10).fill, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(sel(&s), (12, 12));
    // Different text from the system clipboard pastes in the caret's style.
    s.execute("text.setSelection", json!({"start": 4})).unwrap();
    s.execute("edit.paste", json!({"text": "new "})).unwrap();
    assert_eq!(doc(&s, t).text, "red new plainred");
    assert_eq!(doc(&s, t).style_at(5).size, 60.0);
    // Paste Text and Match Formatting: the copied text in the caret's style.
    s.execute("text.setSelection", json!({"start": 8})).unwrap();
    s.execute("edit.pasteTextMatchFormatting", json!({})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.text, "red new redplainred");
    assert_eq!(d.style_at(9).fill, [1.0; 4]);
    // Paste Text Formatting Only onto a selection.
    s.execute("text.setSelection", json!({"start": 11, "end": 16})).unwrap();
    s.execute("edit.pasteTextFormattingOnly", json!({})).unwrap();
    let d = doc(&s, t);
    assert_eq!(d.style_at(12).fill, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(d.style_at(12).size, 20.0);
    // Cut removes the selection.
    s.execute("text.setSelection", json!({"start": 0, "end": 4})).unwrap();
    assert_eq!(s.execute("edit.cut", json!({})).unwrap()["text"], "red ");
    assert_eq!(doc(&s, t).text, "new redplainred");
    // Not editing: Paste Text Formatting Only restyles the whole selected text layer.
    s.execute("text.endEdit", json!({})).unwrap();
    s.execute("layer.select", json!({"layers": [t]})).unwrap();
    s.execute("edit.pasteTextFormattingOnly", json!({})).unwrap();
    assert!(doc(&s, t).is_uniform());
    assert_eq!(doc(&s, t).size, 20.0);
}

#[test]
fn type_tool_layers_point_box_and_empty_cleanup() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "C", "width": 640, "height": 360, "duration": 4})).unwrap();
    let t = s.execute("layer.newText", json!({"text": "", "position": [100, 200], "justify": "left", "edit": true})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(s.state.text_edit.as_ref().map(|e| e.layer.0), Some(t));
    // Leaving an untouched Type-tool layer deletes it.
    s.execute("text.endEdit", json!({})).unwrap();
    assert!(s.active_comp().unwrap().layers.is_empty());
    let b = s.execute("layer.newText", json!({"text": "", "box": [40, 50, 300, 120], "edit": true})).unwrap()["layer"].as_u64().unwrap();
    s.execute("text.insert", json!({"text": "Boxed"})).unwrap();
    let d = doc(&s, b);
    assert_eq!(d.box_size, Some([300.0, 120.0]));
    assert_eq!(d.box_pos, [-150.0, -60.0]);
    let l = s.active_comp().unwrap().layers[0].clone();
    assert_eq!(l.name, "Boxed", "named after its text");
    let pos = l.props.prop("transform/position").unwrap().value.as_vec3();
    assert_eq!([pos[0], pos[1]], [190.0, 110.0]);
    s.execute("text.endEdit", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), 1);
    // Resizing the box (layer space) is an attribute too.
    s.execute("layer.setText", json!({"layer": b, "box": [-100, -60, 200, 120]})).unwrap();
    assert_eq!(doc(&s, b).box_size, Some([200.0, 120.0]));
    s.execute("layer.setText", json!({"layer": b, "vertical": true})).unwrap();
    assert!(doc(&s, b).vertical);
}

#[test]
fn edit_state_round_trips_through_serde() {
    let (mut s, t) = session("abc");
    s.execute("text.edit", json!({"layer": t, "select": [1, 2]})).unwrap();
    let j = serde_json::to_value(&s.state).unwrap();
    let back: crate::EditorState = serde_json::from_value(j).unwrap();
    assert_eq!(back.text_edit, s.state.text_edit);
}

#[test]
fn opentype_features_per_range_render_and_query() {
    let (mut s, t) = session("Office 1/2");
    s.execute("layer.setText", json!({"layer": t, "range": [7, 10], "fractions": true, "ss01": true})).unwrap();
    let d = doc(&s, t);
    assert!(!d.style_at(0).opentype.fractions);
    assert!(d.style_at(8).opentype.fractions && d.style_at(8).opentype.stylistic_set(1));
    // The laid-out glyphs change (frac turns 1/2 into a fraction).
    let plain = effectcraft_text::layout_doc(&TextDoc { text: "1/2".into(), ..Default::default() });
    let frac = effectcraft_text::layout_doc(&d.slice(7..10));
    let ids = |l: &effectcraft_text::TextLayout| l.glyphs.iter().map(|g| g.gid).collect::<Vec<_>>();
    assert_ne!(ids(&plain), ids(&frac));
    // Undo restores the uniform style.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(doc(&s, t).is_uniform());
    // Font feature query (font by name, or the layer's).
    let f = s.execute("text.fontFeatures", json!({"font": "Noto Serif"})).unwrap();
    assert_eq!(f["options"]["smallCaps"], true);
    assert!(f["features"].as_array().unwrap().iter().any(|t| t == "onum"));
    let f = s.execute("text.fontFeatures", json!({"layer": t})).unwrap();
    assert_eq!(f["family"], "Inter");
    assert!(f["options"]["stylisticSets"].as_array().unwrap().contains(&json!(1)));
    assert_eq!(f["options"]["smallCaps"], false);
}

#[test]
fn fonts_lists_bundled_and_installed_families() {
    let mut s = Session::default();
    let r = s.execute("text.fonts", json!({})).unwrap();
    let fams = r["families"].as_array().unwrap();
    assert_eq!(r["count"].as_u64().unwrap() as usize, fams.len());
    let inter = fams.iter().find(|f| f["family"] == "Inter").unwrap();
    assert_eq!(inter["origin"], "bundled");
    assert!(inter["styles"].as_array().unwrap().contains(&json!("Bold")));
    // Installed fonts are listed without any layer having asked for one.
    use effectcraft_text::fonts::FontSource as _;
    let installed = effectcraft_text::fonts::DirectorySource::system().faces();
    for face in &installed {
        assert!(fams.iter().any(|f| f["family"].as_str().unwrap().eq_ignore_ascii_case(&face.family)), "{} not listed", face.family);
    }
    // Filtered by name; rescanning keeps what was there.
    let r = s.execute("text.fonts", json!({"query": "noto ser", "rescan": true})).unwrap();
    assert_eq!(r["families"][0]["family"], "Noto Serif");
    // The bundled face is Regular; the OS can also provide Bold, Italic, or other faces.
    // Compare against the independent directory inventory rather than assuming an empty OS.
    let mut expected_styles = std::collections::BTreeSet::from(["Regular".to_string()]);
    expected_styles.extend(installed.iter().filter(|face| face.family.eq_ignore_ascii_case("Noto Serif")).map(|face| face.style.clone()));
    let styles: Vec<String> = r["families"][0]["styles"].as_array().unwrap().iter().map(|style| style.as_str().unwrap().to_string()).collect();
    assert_eq!(styles.iter().cloned().collect::<std::collections::BTreeSet<_>>(), expected_styles);
    assert_eq!(styles.len(), expected_styles.len(), "style menu has no duplicates");
    assert!(r["families"].as_array().unwrap().iter().all(|f| f["family"].as_str().unwrap().to_lowercase().contains("noto ser")));
    // The style menus offer a family's own styles.
    assert_eq!(crate::font_styles("noto serif"), styles);
    assert_eq!(crate::font_styles("NOTO SERIF"), styles);
    assert_eq!(crate::font_styles("No Such Family").len(), 5);
}

/// `layer.setText`'s schema lists every OpenType key as optional, so checked execution (control
/// channel, MCP) accepts them and requires none (#520).
#[test]
fn set_text_schema_accepts_every_opentype_key_and_requires_none() {
    let spec = crate::commands::find("layer.setText").unwrap();
    let schema = crate::commands::params_schema(spec);
    assert_eq!(schema["required"], json!([]));
    for key in ["discretionaryLigatures", "ss01", "ss02", "ss20", "swash"] {
        assert!(schema["properties"].get(key).is_some(), "{key} is not in the schema");
    }
    assert!(schema["properties"].get("OpenType").is_none());
    let (mut s, t) = session("Office 1/2");
    s.execute_checked("layer.setText", json!({"layer": t, "discretionaryLigatures": true, "ss02": true, "ss20": true})).unwrap();
    assert!(doc(&s, t).style_at(0).opentype.stylistic_set(20));
}
