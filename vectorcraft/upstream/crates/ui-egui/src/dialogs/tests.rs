//! The dialog registry and the shared frame, drawn headlessly.

use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::theme;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp, input: egui::RawInput) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn enter() -> egui::RawInput {
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    egui::RawInput { events: vec![key], ..Default::default() }
}

fn open(app: &mut VectorcraftApp, kind: &str, fields: serde_json::Value) {
    app.ui.dialog = Some(Dialog::new(kind, fields));
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

#[test]
fn kinds_map_to_their_dialogs() {
    for (kind, want) in [
        ("newDocument", DialogKind::NewDocument),
        ("roundedRectangle", DialogKind::Shape),
        ("lineSegment", DialogKind::Shape),
        ("rectangularGrid", DialogKind::Shape),
        ("polarGrid", DialogKind::Shape),
        ("shear", DialogKind::Transform),
        ("splitIntoGrid", DialogKind::PathOp),
        ("artboardOptions", DialogKind::ArtboardOptions),
        ("command", DialogKind::Command),
        ("effect", DialogKind::Effect),
        (crate::unsaved::KIND, DialogKind::SaveChanges),
        ("manageWorkspaces", DialogKind::Workspaces),
        ("findFont", DialogKind::FindFont),
    ] {
        assert_eq!(DialogKind::of(kind), Some(want), "{kind}");
    }
    assert_eq!(DialogKind::of("nope"), None);
    assert!(spec("effect").preview && spec("recolor").preview && !spec("move").preview);
    assert!(spec("preferences").window.is_some() && spec("rectangle").window.is_none());
}

#[test]
fn every_dialog_draws() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let openers: [(&str, serde_json::Value); 8] = [
        ("file.newDialog", json!({})),
        ("file.documentSetup", json!({})),
        ("file.exportForScreens", json!({})),
        ("ui.recolorDialog", json!({})),
        ("ui.paramDialog", json!({"command": "object.path.simplify", "label": "Simplify", "params": {"tolerance": 2, "csv": "a\nb"}})),
        ("effect.dialog", json!({"effect": "distort.roughen"})),
        ("edit.preferences", json!({})),
        ("type.findFont", json!({})),
    ];
    for (id, p) in openers {
        app.run(id, p).unwrap();
        assert!(app.ui.dialog.is_some(), "{id} opened nothing");
        frame(&mut app, Default::default());
        app.ui.dialog = None;
        let _ = app.session.cancel_interaction();
    }
    for kind in [
        "rectangle",
        "roundedRectangle",
        "ellipse",
        "polygon",
        "star",
        "lineSegment",
        "rectangularGrid",
        "polarGrid",
        "rotate",
        "reflect",
        "scale",
        "shear",
        "artboardOptions",
    ] {
        open_tool_dialog(&mut app, kind, json!({"x": 5, "y": 5, "index": 0}));
        assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == kind), "{kind}");
        frame(&mut app, Default::default());
    }
    for (kind, fields) in [
        ("move", json!({"dx": "0 pt", "dy": "0 pt"})),
        ("average", json!({"axis": "both"})),
        (crate::unsaved::KIND, json!({"index": 0, "name": "Untitled-1", "then": "close"})),
        ("newWorkspace", json!({"name": "Mine"})),
        ("shortcuts", json!({})),
        ("unregistered", json!({"value": "1"})),
    ] {
        open(&mut app, kind, fields);
        frame(&mut app, Default::default());
    }
}

#[test]
fn enter_confirms_through_the_dialog_spec() {
    let mut app = app();
    open_tool_dialog(&mut app, "rectangle", json!({"x": 10, "y": 20}));
    frame(&mut app, enter());
    assert!(app.ui.dialog.is_none());
    assert_eq!(objects(&app), 1);
    // An unregistered kind still closes on OK without running anything.
    open(&mut app, "unregistered", json!({}));
    assert_eq!(confirm(&mut app), Ok(serde_json::Value::Null));
    assert!(app.ui.dialog.is_none());
    assert_eq!(confirm(&mut app), Err("no dialog open".into()));
}

#[test]
fn confirm_runs_each_dialogs_command() {
    let mut app = app();
    open(&mut app, "star", json!({"x": 50, "y": 50, "radius1": "40 pt", "radius2": "20 pt", "points": 6}));
    confirm(&mut app).unwrap();
    open(&mut app, "lineSegment", json!({"x": 0, "y": 0, "length": "30 pt", "angle": 90}));
    confirm(&mut app).unwrap();
    assert_eq!(objects(&app), 2);
    app.run("select.all", json!({})).unwrap();
    // Scale with Uniform uses the horizontal value for both axes; Copy keeps the original.
    open(&mut app, "scale", json!({"sx": 200, "sy": 50, "uniform": true, "copy": true}));
    confirm(&mut app).unwrap();
    assert_eq!(objects(&app), 4);
    // The parameter dialog closes before its command runs, then runs it with the edited values.
    app.run("ui.paramDialog", json!({"command": "view.saved.new", "label": "New View", "params": {"name": "Close-up"}})).unwrap();
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().doc.views.last().map(|v| v.name.as_str()), Some("Close-up"));
    // New Document creates the document and closes.
    let docs = app.session.documents().len();
    open(
        &mut app,
        "newDocument",
        json!({"width": "300 pt", "height": "200 pt", "units": "Points", "name": "Card", "artboards": 1, "colorMode": "RGB"}),
    );
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.documents().len(), docs + 1);
}

#[test]
fn effect_preview_is_kept_as_one_undo_step() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
    frame(&mut app, Default::default());
    assert!(app.session.in_interaction(), "the preview runs as an interaction");
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    assert_eq!(app.last_effect.as_ref().map(|e| e.0.as_str()), Some("distort.roughen"));
    assert_ne!(app.session.doc().unwrap().doc, before);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn raster_filter_dialogs_preview_and_apply() {
    for (effect, key, value) in [
        ("blur.radial", "method", "zoom"),
        ("blur.smart", "quality", "high"),
        ("sharpen.unsharpMask", "amount", "120"),
        ("pixelate.colorHalftone", "channel1", "30"),
        ("pixelate.crystallize", "cellSize", "20"),
        ("pixelate.mezzotint", "type", "longLines"),
        ("pixelate.pointillize", "cellSize", "12"),
        ("texture.craquelure", "crackDepth", "9"),
        ("texture.grain", "grainType", "stippled"),
        ("texture.mosaicTiles", "tileSize", "20"),
        ("texture.patchwork", "relief", "20"),
        ("texture.stainedGlass", "borderThickness", "2"),
        ("texture.texturizer", "lightDirection", "bottomLeft"),
    ] {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("effect.dialog", json!({"effect": effect})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        let v: serde_json::Value = value.parse::<f64>().map_or_else(|_| json!(value), |n| json!(n));
        d.fields.insert(key.into(), v.clone());
        frame(&mut app, Default::default());
        assert!(app.session.in_interaction(), "{effect}: previews");
        confirm(&mut app).unwrap();
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        assert_eq!(n.appearance.effects.len(), 1, "{effect}");
        assert_eq!(n.appearance.effects[0].params[key], v, "{effect}");
    }
    // Fields follow the effect's documented order (its dialog's), not the alphabet.
    let doc = vectorcraft_effects::effect_info("blur.smart").unwrap().params;
    let ranks: Vec<usize> = ["radius", "threshold", "quality"].iter().map(|k| super::effect::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{ranks:?}");
    assert_eq!(super::effect::doc_rank(doc, "nope"), usize::MAX);
    let doc = vectorcraft_effects::effect_info("pixelate.colorHalftone").unwrap().params;
    let ranks: Vec<usize> = ["maxRadius", "channel1", "channel2", "channel3", "channel4"].iter().map(|k| super::effect::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[4] < usize::MAX, "{ranks:?}");
    // Labels as the dialogs show them.
    assert_eq!(super::form::humanize("maxRadius"), "Max. Radius:");
    assert_eq!(super::form::humanize("channel3"), "Channel 3:");
    assert_eq!(super::form::humanize("cellSize"), "Cell Size:");
    assert_eq!(super::form::humanize("lightDirection"), "Light Direction:");
    let doc = vectorcraft_effects::effect_info("texture.texturizer").unwrap().params;
    let ranks: Vec<usize> = ["texture", "scaling", "relief", "lightDirection", "invert"].iter().map(|k| super::effect::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[4] < usize::MAX, "{ranks:?}");
}

#[test]
fn effect_dialog_applies_to_its_appearance_item() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.roughen", "item": 1})).unwrap();
    frame(&mut app, Default::default());
    confirm(&mut app).unwrap();
    let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    assert_eq!(n.appearance.items[1].effects().len(), 1, "the effect goes to the stroke");
    assert!(n.appearance.effects.is_empty() && n.appearance.items[0].effects().is_empty());
}

#[test]
fn revolve_dialog_preview_cancel_confirm_and_undo() {
    let mut app = app();
    app.run("path.create", json!({"d":"M150 40 L180 60 L180 140 L150 160"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("effect.dialog", json!({"effect":"threeD.revolve","item":null})).unwrap();
    frame(&mut app, Default::default());
    assert!(app.session.in_interaction());
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("effect.dialog", json!({"effect":"threeD.revolve","item":null})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("angle".into(), json!(180));
    frame(&mut app, Default::default());
    confirm(&mut app).unwrap();
    assert_ne!(app.session.doc().unwrap().doc, before);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

#[test]
fn dialogs_are_as_tall_as_their_content() {
    // The save prompt, shown after a taller dialog, is as tall as its text and buttons: no empty
    // band around the buttons, nothing inherited from the other dialog.
    let mut app = app();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let height = |app: &mut VectorcraftApp, kind: &str, fields: serde_json::Value| {
        open(app, kind, fields);
        for _ in 0..30 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
        }
        ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", kind)))).unwrap().height()
    };
    let tall = height(&mut app, "command", json!({"__command": "object.move", "a": 1, "b": 2, "c": 3, "d": 4, "e": 5, "f": 6, "g": 7, "h": 8}));
    let prompt = height(&mut app, crate::unsaved::KIND, json!({"index": 0, "name": "Untitled-1", "then": "close"}));
    assert!(prompt < 180.0 && prompt < tall, "save prompt {prompt} pt tall (the dialog before it: {tall} pt)");
}

#[test]
fn dialogs_do_not_stretch_to_the_screen() {
    // A dialog is as tall as its content: the same on a short and a tall screen.
    let openers: &[(&str, serde_json::Value)] = &[
        ("ui.newSwatch", json!({})),
        ("ui.newColorGroup", json!({})),
        ("ui.swatchOptions", json!({"name": "White"})),
        ("ui.colorPicker", json!({})),
        ("ui.graphicStyleOptions", json!({})),
        ("ui.brushOptions", json!({"name": "3 pt. Round"})),
        ("ui.brushOptions", json!({"name": "Bristle Round"})),
        ("ui.colorGuideOptions", json!({})),
        ("ui.colorBalanceDialog", json!({})),
        ("ui.saturateDialog", json!({})),
        ("ui.saveSwatchLibrary", json!({})),
        ("ui.flattenTransparencyDialog", json!({})),
        ("ui.expandDialog", json!({})),
        ("ui.spotColors", json!({})),
        ("ui.paramDialog", json!({"command": "object.path.simplify", "label": "Simplify", "params": {"tolerance": 2}})),
        ("effect.dialog", json!({"effect": "distort.roughen"})),
        ("ui.recolorDialog", json!({})),
    ];
    let height = |id: &str, p: &serde_json::Value, screen_h: f32| -> Option<(String, f32)> {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run(id, p.clone()).ok()?;
        let kind = app.ui.dialog.as_ref()?.kind.clone();
        spec(&kind).window.is_none().then_some(())?;
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, screen_h));
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        }
        let h = ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", kind.as_str()))))?.height();
        Some((kind, h))
    };
    let mut checked = 0;
    for (id, p) in openers {
        let (Some((kind, short)), Some((_, tall))) = (height(id, p, 1000.0), height(id, p, 2000.0)) else { continue };
        assert!((short - tall).abs() < 1.0, "{kind} is {short} pt tall on a 1000 pt screen but {tall} pt on a 2000 pt one");
        checked += 1;
    }
    assert!(checked >= 10, "only {checked} dialogs drew in the shared frame");
}

/// The dialog window of `app`'s open dialog after a few frames in a `w` × 900 window.
fn dialog_rect(app: &mut VectorcraftApp, w: f32) -> egui::Rect {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 900.0));
    let kind = app.ui.dialog.as_ref().unwrap().kind.clone();
    for _ in 0..3 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        ctx.run_ui(input, |ui| show(app, ui.ctx())).textures_delta.clear();
    }
    ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", kind.as_str())))).unwrap()
}

#[test]
fn a_form_dialog_is_as_wide_as_its_fields_not_the_window() {
    let mut app = app();
    // Clean Up: three check boxes in the generic parameter dialog.
    let params = json!({"emptyTextPaths": true, "strayPoints": true, "unpaintedObjects": true});
    app.run("ui.paramDialog", json!({"command": "object.path.cleanUp", "label": "Clean Up", "params": params})).unwrap();
    let wide = dialog_rect(&mut app, 1600.0);
    assert!(wide.width() < 500.0, "Clean Up is {:.0} wide in a 1600-point window", wide.width());
    // The same dialog in a narrow window still fits it.
    let narrow = dialog_rect(&mut app, 360.0);
    assert!(narrow.width() <= 360.0, "{:.0} wide in a 360-point window", narrow.width());
}

#[test]
fn menu_parameter_dialogs_are_compact() {
    // #292: Object › Path › Simplify (and its neighbours) spanned the whole window.
    for id in ["object.path.simplify", "object.path.offsetPath", "object.move", "object.rotate", "object.path.splitIntoGrid", "path.average"] {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        crate::menus::invoke(&mut app, id, json!({}));
        assert!(app.ui.dialog.is_some(), "{id} opened no dialog");
        let wide = dialog_rect(&mut app, 1600.0);
        eprintln!("{id}: {:.0} × {:.0} in a 1600-point window", wide.width(), wide.height());
        assert!(wide.width() < 420.0, "{id} is {:.0} wide in a 1600-point window", wide.width());
    }
}

/// A list mixing built-in labels with names translates only the built-in entries: names that
/// happen to be catalog keys ("Black", "Regular") are shown as they are.
#[test]
fn mixed_lists_translate_only_their_built_in_entries() {
    use crate::i18n::{Lang, tr};
    let zh = Lang::from_code("zh-hant").unwrap();
    for key in ["None", "Black", "Regular", "Default"] {
        assert_ne!(tr(zh, key), key, "{key} must be a catalog key for this test");
    }
    let names = ["None", "Black", "Regular", "Default"];
    assert_eq!(shown_names(zh, &names, |k| k == 0 || k == 3), [tr(zh, "None"), "Black", "Regular", tr(zh, "Default")]);
    assert_eq!(shown_names(zh, &names, |_| false), names);
    assert_eq!(shown_names(Lang::EN, &names, |_| true), names);
}

/// Confirmations and plug-in dialogs are headed by the text they are given (callers translate
/// their own templates; a plug-in's name is its own).
#[test]
fn given_headings_are_shown_as_they_are() {
    let d = Dialog::new(confirm::KIND, json!({"message": "Delete “Black”?", "detail": "Regular"}));
    assert_eq!((spec(&d.kind).heading)(&d), "Delete “Black”?");
    let d = Dialog::new(plugin::KIND, json!({"__label": "Black", "__plugin": "x"}));
    assert_eq!((spec(&d.kind).heading)(&d), "Black");
}

/// A plug-in dialog with one whole-number field `seed` in `min..=max`, driven by egui events.
struct PluginIntField {
    ctx: egui::Context,
    d: Dialog,
    specs: Vec<(String, vectorcraft_plugins::ParamSpec)>,
    pos: egui::Pos2,
}

impl PluginIntField {
    fn new(min: i64, max: i64, start: i64) -> Self {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let specs = vec![("seed".into(), vectorcraft_plugins::ParamSpec::Int { min, max, default: 0 })];
        let mut f = PluginIntField { ctx, d: Dialog::new(plugin::KIND, json!({ "seed": start })), specs, pos: egui::Pos2::ZERO };
        fn number_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| number_rect(s, text)),
                _ => None,
            }
        }
        let out = f.run(vec![]);
        f.pos = out.shapes.iter().find_map(|s| number_rect(&s.shape, &start.to_string())).unwrap().center();
        f
    }

    fn run(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        let (d, specs) = (&mut self.d, &self.specs);
        let mut out = self.ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            form::schema_fields(ui, d, specs);
        });
        out.textures_delta.clear();
        out
    }

    /// Clicks the field, sends `events`, presses Enter and returns the field's value.
    fn commit(&mut self, events: Vec<egui::Event>) -> serde_json::Value {
        use egui::{Event, Key, Modifiers, PointerButton};
        let (pos, button) = (self.pos, PointerButton::Primary);
        self.run(vec![Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::default() }]);
        self.run(vec![Event::PointerButton { pos, button, pressed: false, modifiers: Modifiers::default() }]);
        self.run(events);
        self.run(vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::default() }]);
        self.d.fields["seed"].clone()
    }

    /// Replaces the field's text with `text` and commits it.
    fn type_text(&mut self, text: &str) -> serde_json::Value {
        let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        self.commit(vec![
            egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: all },
            egui::Event::Text(text.into()),
        ])
    }
}

#[test]
fn plugin_integer_field_preserves_precision_and_clamps_exactly() {
    let (min, max) = (-9007199254740995_i64, 9007199254740995_i64);
    let mut f = PluginIntField::new(min, max, 0);
    for (text, expected) in [
        ("9007199254740993", 9007199254740993_i64),
        ("9007199254740996", max),
        ("-9007199254740996", min),
        ("invalid", min),
        ("1.5", min),
        ("9223372036854775808", min),
    ] {
        assert_eq!(f.type_text(text), json!(expected), "{text}");
    }
}

#[test]
fn plugin_integer_field_with_small_bounds_keeps_numeric_editing() {
    let mut f = PluginIntField::new(1, 16, 3);
    let up = egui::Event::Key { key: egui::Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    assert_eq!(f.commit(vec![up]), json!(4));
    for (text, expected) in [("7", 7), ("2 + 3", 5), ("17", 16)] {
        assert_eq!(f.type_text(text), json!(expected), "{text}");
    }
}

#[test]
fn a_dialog_opens_with_its_first_field_focused_so_typing_and_enter_apply() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let enter = || egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let bounds = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    // Object › Transform › Rotate…: type 90 over the angle, press Enter.
    crate::menus::invoke(&mut app, "object.rotate", json!({}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("90".into())]);
    frame(&mut app, vec![enter()]);
    assert!(app.ui.dialog.is_none());
    let b = bounds(&app);
    assert!((b.width() - 20.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "{b:?}");
    // Move…: its first field is a distance (with a unit): typing replaces it as well.
    let before = bounds(&app);
    crate::menus::invoke(&mut app, "object.move", json!({}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("15".into())]);
    frame(&mut app, vec![enter()]);
    assert!(app.ui.dialog.is_none());
    let after = bounds(&app);
    assert!((after.x0 - before.x0 - 15.0).abs() < 1e-6 && (after.y0 - before.y0).abs() < 1e-6, "{before:?} → {after:?}");
    // Transform Each, then the Transform effect: Scale › Horizontal, a plain number, comes first.
    let before = bounds(&app);
    crate::menus::invoke(&mut app, "object.transformEach", json!({}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("50".into())]);
    frame(&mut app, vec![enter()]);
    assert!(app.ui.dialog.is_none());
    let after = bounds(&app);
    assert!((after.width() - before.width() / 2.0).abs() < 1e-6 && (after.height() - before.height()).abs() < 1e-6, "{before:?} → {after:?}");
    app.run("effect.dialog", json!({"effect": "distort.transform"})).unwrap();
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("50".into())]);
    frame(&mut app, vec![enter()]);
    assert!(app.ui.dialog.is_none());
    let node = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    assert_eq!(node.appearance.effects.last().map(|e| e.params["scaleH"].clone()), Some(json!(50.0)));
}

/// #1000: a count typed in Object › Create Object Mosaic… reaches the command (a whole number typed
/// in a command's dialog is an integer), so the mosaic has that many columns, not the 10 × 10 grid.
#[test]
fn object_mosaic_makes_the_grid_typed_in_its_dialog() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 40})).unwrap();
    app.run("object.rasterize", json!({"ppi": 72})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let entry = crate::menus::menu_entries(&app).into_iter().find(|e| e.label == "Create Object Mosaic…").expect("menu item");
    let (cmd, params) = crate::menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    crate::menus::invoke(&mut app, &cmd, params);
    // Its first field, Columns, is focused: type 3 over it and press Enter.
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("3".into())]);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(&mut app, vec![enter]);
    assert!(app.ui.dialog.is_none());
    let mut tiles = None;
    app.session.doc().unwrap().doc.walk(|n| {
        if n.name.as_deref() == Some("Object Mosaic") {
            tiles = n.children().map(|c| c.len());
        }
    });
    assert_eq!(tiles, Some(3 * 10));
}

/// #793: Artboard Options lists its fields as Name, Width, Height, not by key.
#[test]
fn artboard_options_lists_name_then_width_and_height() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 612, "height": 792})).unwrap();
    open(&mut app, "artboardOptions", json!({"index": 0, "name": "Artboard 1", "width": 612, "height": 792, "x": 0, "y": 0}));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s} in {text}"));
    assert!(at("Name") < at("Width") && at("Width") < at("Height"), "{text}");
}

#[test]
fn the_transform_effect_dialog_has_its_controls_previews_them_and_shows_them_again() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.transform"})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
    for s in [
        "Transform",
        "Scale",
        "Move",
        "Rotate",
        "Horizontal:",
        "Vertical:",
        "Angle:",
        "Copies:",
        "Reflect X",
        "Reflect Y",
        "Reference Point",
        "Random",
        "Preview",
    ] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    assert!(app.session.in_interaction(), "the preview runs");
    let effect = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].clone();
    // The reference point and Random update the preview.
    let d = app.ui.dialog.as_mut().unwrap();
    for (k, v) in [("moveH", json!(30)), ("copies", json!(3)), ("reference", json!(8))] {
        d.fields.insert(k.into(), v);
    }
    frame(&mut app, Default::default());
    assert_eq!((effect(&app).params["reference"].clone(), effect(&app).params["copies"].clone()), (json!(8), json!(3)));
    app.ui.dialog.as_mut().unwrap().fields.insert("random".into(), json!(true));
    frame(&mut app, Default::default());
    assert_eq!(effect(&app).params["random"], json!(true));
    confirm(&mut app).unwrap();
    assert!(!app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.len(), 1);
    // Editing it from the Appearance panel shows the same values.
    app.run("effect.dialog", json!({"effect": "distort.transform", "index": 0})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("reference", 4.0), d.f64("copies", 0.0), d.f64("moveH", 0.0), d.bool("random")), (8.0, 3.0, 30.0, true));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
    assert!(text.lines().any(|l| l == "Reference Point"), "{text}");
}

#[test]
fn artboard_options_position_moves_art_only_with_the_tool_option_on() {
    for move_art in [false, true] {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("moveArt", &json!(move_art));
        crate::dialogs::artboard_options::open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("x".into(), json!(50));
        // Draw the dialog once and confirm through its registry, as the OK button does.
        frame(&mut app, Default::default());
        let before = app.session.doc().unwrap().doc.clone();
        super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.artboards[0].rect.x0, 50.0);
        assert_eq!(
            app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0,
            if move_art { 60.0 } else { 10.0 }
        );
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, *before);
    }
}

/// Zig Zag's dialog has Size and Ridges per segment sliders, and Relative / Absolute and Smooth /
/// Corner as radio buttons, not dropdowns (#1133).
#[test]
fn zig_zag_has_sliders_and_radio_buttons() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.zigZag"})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    for want in ["Size:", "Relative", "Absolute", "Ridges per segment:", "Points", "Smooth", "Corner"] {
        assert!(lines.contains(&want), "no {want:?} in {lines:?}");
    }
    assert!(!lines.contains(&"Ridges:"), "{lines:?}");
}

#[test]
fn polar_grid_shared_canvas_click_defaults_units_counts_and_history() {
    use vectorcraft_doc::NodeId;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    app.select_tool("polarGrid");
    app.run("tool.setOption", json!({"tool": "polarGrid", "values": {"concentric": 2, "radial": 3}})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 30.0, 40.0), Default::default());
    }
    assert_eq!(DialogKind::of("polarGrid"), Some(DialogKind::Shape));
    let d = app.ui.dialog.as_mut().expect("canvas click opens the shared shape form");
    assert_eq!(d.fields["width"], json!("100 pt"));
    assert_eq!(d.fields["height"], json!("100 pt"));
    assert_eq!((d.fields["concentric"].clone(), d.fields["radial"].clone()), (json!(2), json!(3)));
    d.fields.insert("width".into(), json!("25.4 mm"));
    d.fields.insert("height".into(), json!("0.5 in"));
    frame(&mut app, Default::default());
    let result = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let s = app.session.doc().unwrap();
    let group = s.doc.node(NodeId(result["id"].as_u64().unwrap())).unwrap();
    let children = group.children().unwrap();
    assert_eq!(children.len(), 6);
    assert_eq!(children.iter().filter(|n| n.path_data().unwrap().is_closed()).count(), 3);
    assert_eq!(s.history.undo.len(), undo + 1);
    let made = s.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, made);
}

#[test]
fn polar_grid_shared_cancel_reopen_zero_counts_and_failed_creation() {
    let mut app = app();
    app.run("tool.setOption", json!({"tool": "polarGrid", "values": {"concentric": 2, "radial": 3}})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    let d = app.ui.dialog.as_mut().expect("shared Polar Grid form opens");
    d.fields.insert("concentric".into(), json!(8));
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["concentric"], json!(2));
    for key in ["concentric", "radial"] {
        app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), json!(0));
    }
    let result = confirm(&mut app).unwrap();
    let group = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(result["id"].as_u64().unwrap())).unwrap();
    assert_eq!(group.children().unwrap().len(), 1);
    assert!(group.children().unwrap()[0].path_data().unwrap().is_closed());
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["radial"], json!(0));
    cancel(&mut app);
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"ids": [layer.0], "locked": true})).unwrap();
    let options = app.session.tool_options_of("polarGrid");
    let before = app.session.doc().unwrap().doc.clone();
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    app.ui.dialog.as_mut().unwrap().fields.insert("radial".into(), json!(9));
    assert!(confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.tool_options_of("polarGrid"), options);
    assert_eq!(app.session.doc().unwrap().doc, before);
}
