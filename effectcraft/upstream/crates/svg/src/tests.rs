use super::*;

const BASIC: &[u8] = include_bytes!("../tests/fixtures/basic-shapes.svg");
const PATHS: &[u8] = include_bytes!("../tests/fixtures/paths-gradients.svg");

fn px(img: &effectcraft_raster::Image, x: u32, y: u32) -> [f32; 4] {
    img.data[img.idx(x, y)]
}

fn near(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() <= tol)
}

#[test]
fn basic_shapes_structure() {
    let d = parse(BASIC).unwrap();
    assert_eq!((d.width, d.height), (200.0, 100.0));
    assert_eq!(d.pixel_size(), (200, 100));
    // viewBox 400×200 into 200×100.
    assert!((d.root.transform.as_coeffs()[0] - 0.5).abs() < 1e-12);
    let shapes = d.flatten();
    assert_eq!(shapes.len(), 6);
    let (m, o, s) = shapes[1];
    assert_eq!(s.name, "box");
    assert!(matches!(s.geom, Geom::Rect { w, h, rx, .. } if w == 120.0 && h == 80.0 && rx == 12.0));
    assert!((o - 0.8).abs() < 1e-12);
    // translate(20 20) then the viewBox scale.
    let p = m * kurbo::Point::new(10.0, 10.0);
    assert!((p.x - 15.0).abs() < 1e-9 && (p.y - 15.0).abs() < 1e-9);
    let st = s.stroke.as_ref().unwrap();
    assert_eq!(st.width, 6.0);
    assert_eq!(st.paint, Paint::Color([1.0, 1.0, 1.0]));
    assert!(matches!(shapes[2].2.geom, Geom::Ellipse { rx, ry, .. } if rx == 30.0 && ry == 30.0));
    assert_eq!(shapes[3].2.fill_opacity, 0.75);
    assert!(shapes[4].2.fill.is_none());
    assert_eq!(shapes[4].2.stroke.as_ref().unwrap().join, Join::Round);
    assert_eq!(shapes[5].2.stroke.as_ref().unwrap().cap, Cap::Round);
}

#[test]
fn basic_shapes_raster() {
    let d = parse(BASIC).unwrap();
    let img = rasterize(&d, 200, 100, 1.0);
    let bg = [0x20 as f32 / 255.0, 0x28 as f32 / 255.0, 0x30 as f32 / 255.0, 1.0];
    assert!(near(px(&img, 2, 2), bg, 1e-3));
    // Inside the box: tomato at group opacity 0.8 over the background.
    let tomato = [1.0, 99.0 / 255.0, 71.0 / 255.0];
    let want = [0, 1, 2].map(|i| tomato[i] * 0.8 + bg[i] * 0.2);
    let got = px(&img, 45, 35);
    assert!(near(got, [want[0], want[1], want[2], 1.0], 2e-3), "{got:?} vs {want:?}");
    // Scaled ×2: the same point lands at double the coordinates, edges stay sharp.
    let big = rasterize(&d, 400, 200, 2.0);
    assert!(near(px(&big, 90, 70), got, 2e-3));
}

#[test]
fn paths_gradients_css_use() {
    let d = parse(PATHS).unwrap();
    assert_eq!(d.pixel_size(), (160, 120));
    let shapes = d.flatten();
    assert_eq!(shapes.len(), 6, "{:?}", shapes.iter().map(|s| &s.2.name).collect::<Vec<_>>());
    // CSS class → gradient fill.
    let Some(Paint::Gradient(g)) = &shapes[0].2.fill else { panic!("gradient expected") };
    assert!(g.bbox_units);
    assert_eq!(g.stops.len(), 2);
    assert_eq!(shapes[1].2.fill_rule, FillRule::EvenOdd);
    // #wave: id selector beats nothing; stroke from CSS.
    assert!(shapes[2].2.fill.is_none());
    assert_eq!(shapes[2].2.stroke.as_ref().unwrap().width, 5.0);
    // use → the referenced path with the use's fill, translated.
    assert_eq!(shapes[4].2.fill, Some(Paint::Color([128.0 / 255.0, 0.0, 128.0 / 255.0])));
    let p = shapes[4].0 * kurbo::Point::new(0.0, 0.0);
    assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 110.0).abs() < 1e-9);
    assert_eq!(shapes[5].2.stroke.as_ref().unwrap().dash, Some((vec![4.0, 2.0], 0.0)));

    let img = rasterize(&d, 160, 120, 1.0);
    // Vertical gradient: yellow at the top, red at the bottom.
    let top = px(&img, 40, 11);
    let bottom = px(&img, 40, 48);
    assert!(top[1] > 0.7 && bottom[1] < 0.3, "{top:?} {bottom:?}");
    // Even-odd hole.
    assert_eq!(px(&img, 120, 35)[3], 0.0);
    assert!(px(&img, 95, 15)[3] > 0.99);
    // Radial gradient: white centre.
    let c = px(&img, 40, 90);
    assert!(c[0] > 0.9 && c[3] > 0.9, "{c:?}");
}

#[test]
fn malformed_input() {
    assert!(parse(b"<html/>").is_err());
    assert!(parse(b"not xml").is_err());
    let d = parse(b"<svg xmlns='http://www.w3.org/2000/svg'><path d='M0 0 L 10'/><rect width='-5' height='5'/><text>hi</text></svg>").unwrap();
    assert_eq!(d.pixel_size(), (300, 150));
    assert_eq!(d.skipped, vec!["text".to_string()]);
    assert!(looks_like_svg(BASIC));
    assert!(!looks_like_svg(b"\x89PNG"));
}

#[test]
fn transforms() {
    let m = parse_transform("translate(10,20) rotate(90) scale(2 3)");
    let p = m * kurbo::Point::new(1.0, 1.0);
    assert!((p.x - 7.0).abs() < 1e-9 && (p.y - 22.0).abs() < 1e-9, "{p:?}");
    let m = parse_transform("matrix(1 0 0 1 5 6) skewX(45)");
    let p = m * kurbo::Point::new(0.0, 1.0);
    assert!((p.x - 6.0).abs() < 1e-9 && (p.y - 7.0).abs() < 1e-9);
}

#[test]
fn deep_nesting_errors_instead_of_overflowing_the_stack() {
    // 100 000 nested groups overflowed the XML parser's stack and aborted the process.
    let deep = |n: usize| format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}<rect width=\"5\" height=\"5\"/>{}</svg>", "<g>".repeat(n), "</g>".repeat(n));
    assert!(matches!(crate::parse(deep(100_000).as_bytes()), Err(crate::Error::Xml(_))));
    assert!(crate::parse(deep(100).as_bytes()).is_ok());
    // Comments, CDATA, quoted '>' and self-closing tags don't count.
    let s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><!-- {} --><![CDATA[{}]]><g title=\"a>b\">{}</g></svg>",
        "<g>".repeat(1000),
        "<g>".repeat(1000),
        "<rect/>".repeat(1000)
    );
    assert_eq!(crate::xml_depth(&s), 2);
    assert!(crate::parse(s.as_bytes()).is_ok());
}
