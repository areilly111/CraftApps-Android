use super::write::*;
use super::*;

fn gradient(w: u32, h: u32) -> Vec<[f32; 4]> {
    (0..w * h).map(|i| [(i % w) as f32 / (w - 1).max(1) as f32, (i / w) as f32 / (h - 1).max(1) as f32, 0.25, 1.0]).collect()
}

fn close(a: &[[f32; 4]], b: &[[f32; 4]], tol: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (0..4).all(|c| (x[c] - y[c]).abs() <= tol))
}

fn doc_with_layers(depth: u16, mode: ColorMode, rle: bool) -> WDoc {
    let mut d = WDoc::new(40, 30);
    d.depth = depth;
    d.mode = mode;
    d.rle = rle;
    let mut bg = WLayer::pixels("Background", Rect::new(0, 0, 40, 30), gradient(40, 30));
    bg.opacity = 255;
    let mut red = WLayer::solid("Red box", Rect::new(5, 6, 10, 8), [1.0, 0.0, 0.0, 1.0]).with_blend(b"mul ").with_opacity(128);
    red.hidden = true;
    let mut masked = WLayer::solid("Masked", Rect::new(20, 10, 12, 12), [0.0, 0.5, 1.0, 1.0]).with_blend(b"scrn");
    // Mask: left half opaque, right half transparent; default (outside) black.
    let mrect = Rect::new(20, 10, 12, 12);
    masked.mask = Some(WMask { rect: mrect, data: (0..144).map(|i| if i % 12 < 6 { 1.0 } else { 0.0 }).collect(), default_color: 0, disabled: false });
    d.layers = vec![bg, red, masked];
    d
}

#[test]
fn layers_round_trip_8bit_rle() {
    let bytes = write(&doc_with_layers(8, ColorMode::Rgb, true));
    assert!(is_psd(&bytes));
    let p = Psd::parse(bytes).unwrap();
    assert_eq!((p.width, p.height, p.depth, p.mode), (40, 30, 8, ColorMode::Rgb));
    assert_eq!(p.layers.len(), 3);
    let names: Vec<&str> = p.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Red box", "Masked"]);
    assert_eq!(p.layers[1].blend_key(), "mul ");
    assert_eq!(p.layers[1].opacity, 128);
    assert!(p.layers[1].hidden && !p.layers[0].hidden);
    assert_eq!(p.layers[2].blend_key(), "scrn");
    assert_eq!(p.layers[1].rect, Rect::new(5, 6, 10, 8));
    // Pixels (8-bit quantisation).
    let bg = p.layer_pixels(0, false).unwrap();
    assert!(close(&bg.data, &gradient(40, 30), 1.0 / 255.0));
    // Retain layer size vs canvas.
    let red = p.layer_pixels(1, false).unwrap();
    assert_eq!((red.width, red.height), (10, 8));
    let red_c = p.layer_pixels(1, true).unwrap();
    assert_eq!((red_c.width, red_c.height), (40, 30));
    assert_eq!(red_c.data[6 * 40 + 5], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(red_c.data[0][3], 0.0);
    // Layer mask becomes alpha.
    let m = p.layer_pixels(2, false).unwrap();
    assert_eq!(m.data[0][3], 1.0);
    assert_eq!(m.data[11][3], 0.0);
    // Merged composite.
    let c = p.composite().unwrap();
    assert_eq!((c.width, c.height), (40, 30));
    assert!(p.merged_alpha);
}

#[test]
fn layers_round_trip_16bit_raw_and_cmyk_gray() {
    for (depth, mode, rle) in
        [(16, ColorMode::Rgb, false), (16, ColorMode::Rgb, true), (8, ColorMode::Cmyk, true), (16, ColorMode::Cmyk, false), (8, ColorMode::Grayscale, false)]
    {
        let bytes = write(&doc_with_layers(depth, mode, rle));
        let p = Psd::parse(bytes).unwrap_or_else(|e| panic!("{depth} {mode:?} {rle}: {e}"));
        assert_eq!(p.layers.len(), 3, "{depth} {mode:?}");
        assert_eq!(p.depth, depth);
        let bg = p.layer_pixels(0, true).unwrap();
        let tol = if depth == 16 { 1.0 / 30000.0 } else { 1.5 / 255.0 };
        let want: Vec<[f32; 4]> =
            if mode == ColorMode::Grayscale { gradient(40, 30).iter().map(|p| [p[0], p[0], p[0], 1.0]).collect() } else { gradient(40, 30) };
        assert!(close(&bg.data, &want, tol), "{depth} {mode:?} {rle}: {:?} vs {:?}", bg.data[41], want[41]);
        let c = p.composite().unwrap();
        assert_eq!(c.data.len(), 1200);
    }
}

#[test]
fn groups_build_a_tree() {
    let mut d = WDoc::new(16, 16);
    d.layers = vec![
        WLayer::solid("Bottom", Rect::new(0, 0, 16, 16), [1.0; 4]),
        WLayer::group_end(),
        WLayer::solid("Inner A", Rect::new(0, 0, 4, 4), [1.0, 0.0, 0.0, 1.0]),
        WLayer::group_end(),
        WLayer::solid("Deep", Rect::new(2, 2, 4, 4), [0.0, 1.0, 0.0, 1.0]),
        WLayer::group("Nested", false, *b"norm"),
        WLayer::group("Group 1", true, *b"pass"),
        WLayer::solid("Top", Rect::new(8, 8, 4, 4), [0.0, 0.0, 1.0, 1.0]),
    ];
    let p = Psd::parse(write(&d)).unwrap();
    assert_eq!(p.layers[6].section, Section::OpenFolder);
    assert_eq!(p.layers[6].blend_key(), "pass");
    assert_eq!(p.layers[5].section, Section::ClosedFolder);
    let t = p.tree();
    assert_eq!(
        t,
        vec![
            Node::Layer(7),
            Node::Group { layer: 6, children: vec![Node::Group { layer: 5, children: vec![Node::Layer(4)] }, Node::Layer(2)] },
            Node::Layer(0)
        ]
    );
}

#[test]
fn text_effects_fill_and_adjustments() {
    let mut d = WDoc::new(64, 32);
    let fx = Descriptor::new("null").with("masterFXSwitch", DValue::Bool(true)).with(
        "DrSh",
        DValue::Descriptor(
            Descriptor::new("DrSh")
                .with("enab", DValue::Bool(true))
                .with("Md  ", DValue::Enum("BlnM".into(), "Mltp".into()))
                .with("Clr ", DValue::Descriptor(Descriptor::rgb([0.0, 0.0, 1.0])))
                .with("Opct", DValue::UnitFloat("#Prc".into(), 60.0))
                .with("Dstn", DValue::UnitFloat("#Pxl".into(), 7.0)),
        ),
    );
    d.layers = vec![
        WLayer::empty("Fill", vec![solid_color_block([0.0, 1.0, 0.0])]),
        WLayer::empty("Title", vec![text_block("Hello\nWorld", [10.0, 20.0], "Inter-Bold", 18.0, [1.0, 0.0, 0.0], 2)]).with_block(effects_block(&fx)),
        WLayer::empty("Hue", vec![hue_sat_block(30, -20, 10)]),
        WLayer::empty("Lev", vec![levels_block(10, 240, 5, 250, 1.2)]),
        WLayer::empty("Inv", vec![invert_block()]),
        WLayer::empty("BC", vec![brightness_block(20, -10)]),
        WLayer::empty("Exp", vec![exposure_block(1.5, 0.0, 1.0)]),
        WLayer::solid("Vec", Rect::new(0, 0, 64, 32), [1.0; 4]).with_block(vector_mask_block(
            &[vec![[[8.0, 4.0]; 3], [[40.0, 4.0]; 3], [[40.0, 20.0]; 3]]],
            64,
            32,
            false,
        )),
    ];
    let p = Psd::parse(write(&d)).unwrap();
    assert_eq!(p.layers[0].fill, Some([0.0, 1.0, 0.0]));
    let fill = p.layer_pixels(0, false).unwrap();
    assert_eq!((fill.width, fill.height), (64, 32));
    assert_eq!(fill.data[5], [0.0, 1.0, 0.0, 1.0]);
    let t = p.layers[1].text.as_ref().unwrap();
    assert_eq!(t.text, "Hello\nWorld");
    assert_eq!(t.transform[4..], [10.0, 20.0]);
    assert_eq!(t.style.font.as_deref(), Some("Inter-Bold"));
    assert_eq!(t.style.size, Some(18.0));
    assert_eq!(t.style.color, Some([1.0, 0.0, 0.0]));
    assert_eq!(t.style.justification, Some(2));
    let e = p.layers[1].effects.as_ref().unwrap();
    let ds = e.obj("DrSh").unwrap();
    assert_eq!(ds.enum_value("Md  "), Some("Mltp"));
    assert_eq!(ds.num("Dstn"), Some(7.0));
    assert_eq!(p.layers[2].adjustment, Some(Adjustment::HueSaturation { hue: 30.0, saturation: -20.0, lightness: 10.0, colorize: false }));
    assert_eq!(p.layers[3].adjustment, Some(Adjustment::Levels { in_black: 10.0, in_white: 240.0, gamma: 1.2, out_black: 5.0, out_white: 250.0 }));
    assert_eq!(p.layers[4].adjustment, Some(Adjustment::Invert));
    assert_eq!(p.layers[5].adjustment, Some(Adjustment::BrightnessContrast { brightness: 20.0, contrast: -10.0, legacy: true }));
    assert_eq!(p.layers[6].adjustment, Some(Adjustment::Exposure { exposure: 1.5, offset: 0.0, gamma: 1.0 }));
    let vm = p.layers[7].vector_mask.as_ref().unwrap();
    assert_eq!(vm.subpaths.len(), 1);
    assert!(vm.subpaths[0].closed);
    let k = &vm.subpaths[0].knots;
    assert_eq!(k.len(), 3);
    assert!((k[1][1][0] - 40.0).abs() < 1e-4 && (k[2][1][1] - 20.0).abs() < 1e-4);
}

#[test]
fn packbits_round_trip() {
    for src in [vec![], vec![1u8], vec![5; 300], (0..=255).collect::<Vec<u8>>(), vec![1, 1, 2, 3, 3, 3, 4, 5, 5]] {
        let enc = pack_bits(&src);
        let mut out = vec![];
        unpack_bits(&enc, src.len(), &mut out).unwrap();
        assert_eq!(out, src);
    }
}

#[test]
fn truncated_and_garbage_input_errors() {
    let bytes = write(&doc_with_layers(8, ColorMode::Rgb, true));
    for n in [0, 3, 10, 26, 40, 100, bytes.len() / 2] {
        let r = Psd::parse(bytes[..n].to_vec());
        if let Ok(p) = r {
            let _ = p.composite();
        }
    }
    assert!(Psd::parse(b"8BPS\0\x09garbage".to_vec()).is_err());
    assert!(Psd::parse(b"PNG".to_vec()).is_err());
}

#[test]
fn smart_objects_and_embedded_files() {
    let mut inner = WDoc::new(20, 10);
    inner.layers = vec![WLayer::solid("Red", Rect::new(0, 0, 20, 10), [1.0, 0.0, 0.0, 1.0])];
    let inner_bytes = write(&inner);
    let mut d = WDoc::new(64, 48);
    // The content scaled 2× and rotated 90° clockwise: top-left at (40, 4).
    let quad = [[40.0, 4.0], [40.0, 44.0], [20.0, 44.0], [20.0, 4.0]];
    d.layers = vec![
        WLayer::solid("Back", Rect::new(0, 0, 64, 48), [0.0, 0.0, 0.0, 1.0]),
        WLayer::solid("Placed", Rect::new(20, 4, 20, 40), [0.5, 0.5, 0.5, 1.0]).with_block(smart_object_block("uuid-1", quad, [20.0, 10.0])),
    ];
    d.linked = vec![("uuid-1".into(), "inner.psd".into(), *b"8BPS", inner_bytes.clone())];
    let p = Psd::parse(write(&d)).unwrap();
    assert_eq!(p.linked.len(), 1);
    assert_eq!(p.linked[0].name, "inner.psd");
    assert_eq!(p.linked[0].file_type, "8BPS");
    assert_eq!(p.linked_data("uuid-1").unwrap(), &inner_bytes[..]);
    let so = p.layers[1].smart_object.clone().unwrap();
    assert_eq!(so.uuid, "uuid-1");
    assert_eq!(so.quad, quad);
    assert_eq!(so.size, Some([20.0, 10.0]));
    let (c, sx, sy, rot) = so.placement(20.0, 10.0);
    assert_eq!(c, [30.0, 24.0]);
    assert!((sx - 2.0).abs() < 1e-9 && (sy - 2.0).abs() < 1e-9 && (rot - 90.0).abs() < 1e-9, "{sx} {sy} {rot}");
    // The embedded document parses on its own.
    let e = Psd::parse(p.linked_data("uuid-1").unwrap().to_vec()).unwrap();
    assert_eq!((e.width, e.height), (20, 10));
    assert!(p.layers[0].smart_object.is_none());
}

/// Replace the first layer record's bounds (`top, left, bottom, right`) in a written document.
fn patch_layer_rect(bytes: &mut [u8], from: Rect, to: Rect) {
    let enc = |r: Rect| [r.top, r.left, r.bottom, r.right].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let (old, new) = (enc(from), enc(to));
    let at = bytes.windows(16).position(|w| w == old.as_slice()).expect("layer rect in the written file");
    bytes[at..at + 16].copy_from_slice(&new);
}

#[test]
fn extreme_layer_bounds_error_instead_of_overflowing() {
    // A layer spanning the whole i32 range: its width overflowed i32 subtraction.
    let r = Rect::new(2, 3, 4, 5);
    let mut d = WDoc::new(8, 8);
    d.layers = vec![WLayer::solid("L", r, [1.0, 0.0, 0.0, 1.0])];
    let mut bytes = write(&d);
    patch_layer_rect(&mut bytes, r, Rect { top: i32::MIN, left: i32::MIN, bottom: i32::MAX, right: i32::MAX });
    let p = Psd::parse(bytes).unwrap();
    assert_eq!(p.layers[0].rect.width(), u32::MAX);
    assert!(p.layer_pixels(0, false).is_err());
    assert!(p.layer_pixels(0, true).is_err());
}

#[test]
fn huge_layer_bounds_error_instead_of_allocating() {
    // A few bytes claiming a 2-billion-pixel-square layer must not try to allocate it.
    let r = Rect::new(2, 3, 4, 5);
    let mut d = WDoc::new(8, 8);
    d.layers = vec![WLayer::solid("L", r, [1.0, 0.0, 0.0, 1.0])];
    let mut bytes = write(&d);
    patch_layer_rect(&mut bytes, r, Rect { top: 0, left: 0, bottom: i32::MAX, right: i32::MAX });
    let p = Psd::parse(bytes).unwrap();
    assert!(matches!(p.layer_pixels(0, false), Err(Error::Unsupported(_))));
}
