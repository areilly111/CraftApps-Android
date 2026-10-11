use super::*;
use vectorcraft_geom::SubPath;

fn square(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)], true))
}

/// A document with paths A, B and C (bottom to top) on its layer, plus a locked path and a guide.
fn doc() -> (Document, Vec<NodeId>, NodeId) {
    let mut d = Document::new(200.0, 200.0);
    let layer = d.layers[0].id;
    let mut ids = vec![];
    for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
        let mut n = Node::path(d.alloc_id(), square(i as f64 * 20.0), Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 1.0));
        n.name = Some(name.into());
        ids.push(d.insert(Some(layer), usize::MAX, n).unwrap());
    }
    let mut locked = Node::path(d.alloc_id(), square(80.0), Appearance::default());
    locked.locked = true;
    d.insert(Some(layer), usize::MAX, locked).unwrap();
    let guide = Node::new(d.alloc_id(), NodeKind::Path { path: square(90.0), rule: FillRule::NonZero, live: None, clipping: false, guide: true });
    d.insert(Some(layer), usize::MAX, guide).unwrap();
    (d, ids, layer)
}

fn names(d: &Document, layer: NodeId) -> Vec<String> {
    d.children(Some(layer)).unwrap().iter().map(|n| n.name.clone().unwrap_or_else(|| "?".into())).collect()
}

fn decode_json(v: Value) -> Result<Vec<OutObject>> {
    decode(v.to_string().as_bytes())
}

#[test]
fn targets_are_selected_paths_in_paint_order() {
    let (mut d, ids, layer) = doc();
    assert_eq!(filter_targets(&d, &[ids[2], ids[0]]), [ids[0], ids[2]]);
    // A selected layer gives its paths, without the locked one and the guide.
    assert_eq!(filter_targets(&d, &[layer]), ids);
    d.node_mut(ids[1]).unwrap().visible = false;
    assert_eq!(filter_targets(&d, &[layer]), [ids[0], ids[2]]);
}

#[test]
fn objects_round_trip_through_the_json_model() {
    let (mut d, ids, _) = doc();
    let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
    g.gradient.stops[0].swatch = Some("White".into());
    d.node_mut(ids[0]).unwrap().appearance.set_stroke(Paint::Gradient(Box::new(g)));
    let input: Value = serde_json::from_slice(&encode_input(&d, &ids)).unwrap();
    let a = &input["objects"][0];
    assert_eq!((a["type"].as_str(), a["name"].as_str(), a["fillRule"].as_str()), (Some("path"), Some("A"), Some("nonzero")));
    assert_eq!(a["fills"][0]["color"], json!({"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}));
    assert_eq!(a["strokes"][0]["paint"]["type"], "gradient");
    assert_eq!(a["bounds"], json!([0.0, 0.0, 10.0, 10.0]));
    // Echoed back unchanged, nothing changes (swatch links included).
    let before = d.clone();
    let out = decode(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(apply_output(&mut d, &ids, out, None).unwrap(), ids);
    assert_eq!(d, before);
}

#[test]
fn outputs_update_add_and_delete_in_place() {
    let (mut d, ids, layer) = doc();
    let [a, b, c] = [ids[0].0, ids[1].0, ids[2].0];
    let new = |name: &str| json!({"name": name, "path": square(5.0), "fills": [{"type": "solid", "color": {"model": "gray", "k": 0.5}}]});
    // A recoloured, B dropped, C kept; new objects around them.
    let out = json!({"objects": [new("n1"), {"id": a, "fills": [null, {"type": "solid", "color": {"model": "rgb", "r": 5, "g": 0, "b": 0}}]}, new("n2"), {"id": c}, new("n3")]});
    let sel = apply_output(&mut d, &ids, decode_json(out).unwrap(), None).unwrap();
    assert_eq!(names(&d, layer), ["n1", "A", "n2", "C", "n3", "?", "?"]);
    assert_eq!(sel.len(), 5);
    assert!(d.node(NodeId(b)).is_none());
    let na = d.node(NodeId(a)).unwrap();
    let fills = paints(&na.appearance, true);
    assert_eq!(fills, [&Paint::None, &Paint::solid(Color::rgb(1.0, 0.0, 0.0))], "a second fill was added; colours are clamped");
    let n1 = d.node(sel[0]).unwrap();
    assert_eq!(n1.appearance.fill_paint(), Paint::solid(Color::gray(0.5)));
    assert!(n1.appearance.stroke().is_none());
}

#[test]
fn new_objects_take_the_place_of_replaced_inputs_or_go_on_top() {
    let (mut d, ids, layer) = doc();
    let out = json!({"objects": [{"name": "X", "path": square(0.0)}]});
    apply_output(&mut d, &ids[1..], decode_json(out.clone()).unwrap(), None).unwrap();
    assert_eq!(names(&d, layer), ["A", "X", "?", "?"], "B and C replaced by X at B's place");
    // No inputs (a generator): on top of the given container.
    let (mut d, _, layer) = doc();
    apply_output(&mut d, &[], decode_json(out).unwrap(), Some(layer)).unwrap();
    assert_eq!(names(&d, layer).last().map(String::as_str), Some("X"));
}

#[test]
fn geometry_type_and_strokes_change() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    let two = PathData::new(square(0.0).subpaths.into_iter().chain(square(2.0).subpaths).collect());
    let blue = json!({"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}});
    let out = json!({"objects": [{"id": a.0, "type": "compound", "path": two, "fillRule": "evenodd", "strokes": [{"paint": blue, "width": 4000}], "opacity": 0.5, "name": ""}]});
    apply_output(&mut d, &[a], decode_json(out).unwrap(), None).unwrap();
    let n = d.node(a).unwrap();
    let NodeKind::Compound { children, rule } = &n.kind else { panic!("not a compound: {:?}", n.kind) };
    assert_eq!((children.len(), *rule), (2, FillRule::EvenOdd));
    assert_eq!(n.appearance.stroke().unwrap().width, MAX_STROKE_WIDTH);
    assert_eq!((n.opacity, n.name.as_deref()), (0.5, None));
    assert_eq!(geometry(n).unwrap().0, two);
    // And back to a single path; strokes removed.
    let out = json!({"objects": [{"id": a.0, "type": "path", "strokes": []}]});
    apply_output(&mut d, &[a], decode_json(out).unwrap(), None).unwrap();
    let n = d.node(a).unwrap();
    assert!(matches!(&n.kind, NodeKind::Path { path, .. } if *path == two));
    assert!(n.appearance.stroke().is_none());
}

#[test]
fn changed_paints_lose_swatch_links_and_patterns_must_exist() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    d.node_mut(a).unwrap().appearance.set_fill(Paint::Solid { color: Color::rgb(1.0, 0.0, 0.0), swatch: Some("Red".into()), tint: 0.5 });
    let linked = json!({"type": "solid", "color": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}, "swatch": "Red", "tint": 0.5});
    apply_output(&mut d, &[a], decode_json(json!({"objects": [{"id": a.0, "fills": [linked]}]})).unwrap(), None).unwrap();
    assert!(matches!(d.node(a).unwrap().appearance.fill_paint(), Paint::Solid { swatch: Some(_), .. }), "unchanged: kept");
    let other = json!({"type": "solid", "color": {"model": "rgb", "r": 0.0, "g": 1.0, "b": 0.0}, "swatch": "Red", "tint": 0.5});
    apply_output(&mut d, &[a], decode_json(json!({"objects": [{"id": a.0, "fills": [other]}]})).unwrap(), None).unwrap();
    assert_eq!(d.node(a).unwrap().appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    let pattern = json!({"objects": [{"id": a.0, "fills": [{"type": "pattern", "pattern": "Dots"}]}]});
    assert!(apply_output(&mut d, &[a], decode_json(pattern.clone()).unwrap(), None).is_err());
    d.patterns.push(serde_json::from_value(json!({"name": "Dots", "tile": {"x0": 0, "y0": 0, "x1": 10, "y1": 10}, "art": []})).unwrap());
    assert!(apply_output(&mut d, &[a], decode_json(pattern).unwrap(), None).is_ok());
}

#[test]
fn bad_outputs_are_refused() {
    let (d, ids, _) = doc();
    let a = ids[0].0;
    let far = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1e9, 0.0)], false));
    for bad in [
        json!([]),
        json!({"objects": 3}),
        json!({"objects": [7]}),
        json!({"objects": [{"id": 999}]}),
        json!({"objects": [{"id": a}, {"id": a}]}),
        json!({"objects": [{"name": "no path"}]}),
        json!({"objects": [{"id": a, "path": far}]}),
        json!({"objects": [{"id": a, "path": {"subpaths": []}}]}),
        json!({"objects": [{"id": a, "type": "text"}]}),
        json!({"objects": [{"id": a, "fillRule": "odd"}]}),
        json!({"objects": [{"id": a, "fills": [{"type": "gradient", "gradient": {"kind": "Linear", "stops": []}}]}]}),
        json!({"objects": [{"id": a, "fills": [{"type": "solid"}]}]}),
        json!({"objects": [{"id": a, "strokes": [3]}]}),
        json!({"objects": [{"id": a, "opacity": "full"}]}),
        json!({"error": "nothing to do"}),
    ] {
        let mut d = d.clone();
        let r = decode_json(bad.clone()).and_then(|out| apply_output(&mut d, &ids, out, None));
        assert!(r.is_err(), "{bad}");
    }
    assert!(decode(b"\xff\xfe").is_err());
}
