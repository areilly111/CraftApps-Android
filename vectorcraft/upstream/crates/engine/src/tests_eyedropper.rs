//! Eyedropper Options and modes (M3.52): the Picks Up / Applies trees, type attributes, Alt-click
//! (reverse) and Shift+Alt-click (append), image pixels, colour models and the preference.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ImageBlob, ImageObject, Justify, LineCap, Node, NodeKind};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn click(s: &mut Session, x: f64, y: f64, mods: Mods) {
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y).with_mods(mods), ViewInfo::default()).unwrap();
}

/// A red-filled source with a 5 pt round-capped blue stroke at 50 % opacity, and a selected
/// target with the default white fill and a 2 pt black stroke.
fn source_and_target(s: &mut Session) -> (NodeId, NodeId) {
    let src = rect(s, 300.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 5, "cap": "round"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let dst = rect(s, 10.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    (src, dst)
}

#[test]
fn picking_up_without_weight_keeps_the_target_weight() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    s.execute("eyedropper.setOptions", &json!({"pickUp": {"appearance": {"stroke": {"weight": false}}}})).unwrap();
    let weight = s.paint.stroke_width;
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let n = node(&s, dst);
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.paint.color().unwrap().to_hex(), st.width, st.cap), ("#0000ff".into(), 2.0, LineCap::Round));
    assert_eq!(n.appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    assert_eq!(n.opacity, 0.5);
    assert_eq!(s.paint.stroke_width, weight, "new art keeps its weight too");
    // Applying no transparency (one call) keeps the target's opacity; undo is one step per copy.
    s.execute("transparency.set", &json!({"opacity": 100, "ids": [dst.0]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("appearance.copyFrom", &json!({"source": src.0, "apply": {"appearance": {"transparency": false}}})).unwrap();
    assert_eq!((node(&s, dst).opacity, s.doc().unwrap().history.undo.len()), (1.0, undo + 1));
    assert!(s.execute("appearance.copyFrom", &json!({"source": src.0, "pickUp": {"nope": true}})).is_err());
}

#[test]
fn every_appearance_attribute_copies_the_whole_stack() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    assert_eq!(node(&s, dst).appearance, node(&s, src).appearance);
    // Without the fill's overprint only the focal attributes go: the target keeps its two items.
    let dst2 = rect(&mut s, 150.0);
    s.execute("appearance.copyFrom", &json!({"source": src.0, "apply": {"appearance": {"fill": {"overprint": false}}}})).unwrap();
    assert_eq!(node(&s, dst2).appearance.items.len(), 2);
}

#[test]
fn type_picks_up_font_and_size() {
    let mut s = session();
    let src = id_of(s.execute("text.create", &json!({"x": 10, "y": 300, "text": "Source"})).unwrap());
    s.execute("text.setStyle", &json!({"font": "Inter", "size": 30, "tracking": 50, "justify": "center", "fill": "#00ff00"})).unwrap();
    let dst = id_of(s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Target"})).unwrap());
    let style = |s: &Session, id| match &node(s, id).kind {
        NodeKind::Text(t) => (t.first_style(), t.para.justify),
        _ => panic!("not type"),
    };
    let width = |s: &Session, id| node(s, id).geometric_bounds().unwrap().width();
    let before = width(&s, dst);
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let (st, justify) = style(&s, dst);
    assert_eq!((st.font_family.as_str(), st.size, st.tracking, justify), ("Inter", 30.0, 50.0, Justify::Center));
    assert_eq!(st.fill.color().unwrap().to_hex(), "#00ff00");
    assert!(width(&s, dst) > before, "laid out again at the new size");
    // Without character or paragraph attributes only the paints go.
    let dst2 = id_of(s.execute("text.create", &json!({"x": 10, "y": 500, "text": "Other"})).unwrap());
    let preserved_alignment = style(&s, dst2).1;
    s.execute("appearance.copyFrom", &json!({"source": src.0, "pickUp": {"character": false, "paragraph": false}})).unwrap();
    let (st, justify) = style(&s, dst2);
    assert_eq!((st.size, justify, st.fill.color().unwrap().to_hex()), (12.0, preserved_alignment, "#00ff00".into()));
}

#[test]
fn alt_click_applies_the_selection_to_the_clicked_object() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    // `dst` is selected; Alt-click `src`: it takes the selection's white fill and 2 pt stroke.
    let defaults = s.paint.clone();
    click(&mut s, 325.0, 35.0, Mods { alt: true, ..Default::default() });
    let n = node(&s, src);
    assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_width()), (Paint::solid(Color::WHITE), 2.0));
    assert_eq!(node(&s, dst).appearance.fill_paint(), Paint::solid(Color::WHITE), "the selection is unchanged");
    assert_eq!(s.paint, defaults, "the defaults stay");
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("appearance.copyFrom", &json!({"source": src.0, "reverse": true})).is_err(), "nothing to copy from");
}

#[test]
fn shift_alt_click_appends_the_appearance() {
    let mut s = session();
    let (_, dst) = source_and_target(&mut s);
    click(&mut s, 325.0, 35.0, Mods { alt: true, shift: true, ..Default::default() });
    let items = node(&s, dst).appearance.items;
    assert_eq!(items.len(), 4, "the source's fill and stroke on top of the target's");
    assert_eq!((items[0].paint().color().unwrap().to_hex(), items[2].paint().color().unwrap().to_hex()), ("#ffffff".into(), "#ff0000".into()));
}

#[test]
fn cmyk_stays_cmyk() {
    let mut s = session();
    let src = rect(&mut s, 300.0);
    let cmyk = Color::cmyk(0.1, 0.5, 0.9, 0.2);
    s.execute("paint.setFill", &json!({"color": {"c": 0.1, "m": 0.5, "y": 0.9, "k": 0.2}})).unwrap();
    let dst = rect(&mut s, 10.0);
    click(&mut s, 325.0, 35.0, Mods { shift: true, ..Default::default() });
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(cmyk), "a sampled colour keeps its model");
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    click(&mut s, 325.0, 35.0, Mods::default());
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(cmyk));
    let _ = src;
}

#[test]
fn clicking_an_image_samples_its_pixel() {
    let mut s = session();
    // Two pixels, orange and teal, 100 pt each, at (300, 0).
    let img = image::RgbaImage::from_raw(2, 1, [[255, 128, 0, 255], [0, 128, 128, 255]].concat()).unwrap();
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let xf = vectorcraft_geom::Affine::translate((300.0, 0.0)) * vectorcraft_geom::Affine::scale(100.0);
        let im = ImageObject { key: "px".into(), width: 2, height: 1, xf, link: None, placement: Default::default() };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(im))).map_err(|e| crate::EngineError::Other(e.to_string()))
    })
    .unwrap();
    let dst = rect(&mut s, 10.0);
    click(&mut s, 450.0, 50.0, Mods::default());
    // Compared as stored (not as displayed through the working RGB space).
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(Color::rgb8(0, 128, 128)));
    s.execute("eyedropper.setOptions", &json!({"sampleSize": 5})).unwrap();
    click(&mut s, 350.0, 50.0, Mods::default());
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(Color::rgb8(128, 128, 64)), "5 x 5 average of both pixels");
}

#[test]
fn the_options_are_a_preference() {
    let mut s = Session::new();
    assert!(s.execute("eyedropper.setOptions", &json!({"sampleSize": 4})).is_err());
    assert!(s.execute("eyedropper.setOptions", &json!({"apply": {"character": 1}})).is_err());
    s.execute("eyedropper.setOptions", &json!({"sampleSize": 3, "apply": {"paragraph": false}})).unwrap();
    let got = s.execute("prefs.get", &json!({"key": "eyedropper"})).unwrap();
    assert_eq!((&got["sampleSize"], &got["apply"]["paragraph"], &got["pickUp"]["paragraph"]), (&json!(3), &json!(false), &json!(true)));
    s.execute("prefs.set", &json!({"key": "eyedropper", "value": {"pickUp": {"appearance": false}}})).unwrap();
    assert!(!s.prefs.eyedropper.pick_up.appearance.fill.color && s.prefs.eyedropper.sample_size == 3);
    // Saved with the preferences; older preferences (no options) load the defaults.
    let back: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back, s.prefs);
    let mut old = Prefs::default().to_json();
    old.as_object_mut().unwrap().remove("eyedropper");
    assert_eq!(serde_json::from_value::<Prefs>(old).unwrap().eyedropper, EyedropperOptions::default());
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.eyedropper, EyedropperOptions::default());
}

#[test]
fn type_takes_the_stroke_options_it_has() {
    let mut s = session();
    let (src, _) = source_and_target(&mut s);
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("stroke.set", &json!({"dash": [4, 2]})).unwrap();
    let dst = id_of(s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Target"})).unwrap());
    s.execute("eyedropper.setOptions", &json!({"apply": {"appearance": {"stroke": {"dash": false}}}})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let NodeKind::Text(t) = &node(&s, dst).kind else { panic!("not type") };
    let st = t.first_style();
    assert_eq!((st.stroke.color().unwrap().to_hex(), st.stroke_width, st.stroke_cap), ("#0000ff".into(), 5.0, LineCap::Round));
    assert!(st.stroke_dash.is_none(), "the dash pattern isn't applied");
}
