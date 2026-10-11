//! Paints SVG can't express: freeform gradients fill and stroke objects as an `<image>` of their
//! colour field clipped to the shape (or the stroke's outline), and the SVG reads back looking as
//! the canvas paints it.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::freeform::spread_scale;
use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, ImageMode, export_full, import};
use vectorcraft_testkit::format::base64_decode;
use vectorcraft_testkit::raster::{assert_similar, render_artboard};

/// Red at the top left, blue at the bottom right, a half-transparent green at the top right.
fn freeform() -> Paint {
    let p = |x, y, c, opacity| FreeformPoint { opacity, spread: 0.1, ..FreeformPoint::new(Point::new(x, y), c) };
    let f = Freeform {
        points: vec![
            p(30.0, 30.0, Color::rgb(1.0, 0.0, 0.0), 1.0),
            p(170.0, 130.0, Color::rgb(0.0, 0.0, 1.0), 1.0),
            p(170.0, 30.0, Color::rgb(0.0, 1.0, 0.0), 0.5),
        ],
        ..Default::default()
    };
    let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
    g.freeform = Some(f);
    Paint::Gradient(Box::new(g))
}

/// A 200 × 160 document holding `n`.
fn doc_with(mut n: Node) -> Document {
    let mut d = Document::new(200.0, 160.0);
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The data of the one embedded PNG.
fn png_of(svg: &str) -> image::RgbaImage {
    let data = svg.split("data:image/png;base64,").nth(1).expect("an embedded PNG").split('"').next().unwrap();
    image::load_from_memory(&base64_decode(data).unwrap()).unwrap().to_rgba8()
}

#[test]
fn a_freeform_fill_is_an_image_clipped_to_the_shape() {
    let shape = shapes::ellipse(Rect::new(20.0, 20.0, 180.0, 140.0));
    let d = doc_with(Node::path(NodeId(0), shape, Appearance::basic(freeform(), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions::default(), None);
    let svg = &o.svg;
    assert!(!svg.contains("linearGradient"), "{svg}");
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
    assert!(svg.contains("<clipPath") && svg.contains("<g clip-path=\"url(#clip-path-1)\">"), "{svg}");
    assert!(o.warnings.iter().any(|w| w.contains("freeform")), "{:?}", o.warnings);
    // The image is the colour field over the shape's box, straight alpha: the green point is
    // half transparent.
    let png = png_of(svg);
    let (w, h) = png.dimensions();
    assert!((8..=256).contains(&w) && (8..=256).contains(&h), "{w}×{h}");
    let b = Rect::new(20.0, 20.0, 180.0, 140.0);
    let Paint::Gradient(g) = freeform() else { panic!("a gradient") };
    let field = g.freeform.unwrap().field(spread_scale(b));
    for (x, y) in [(2, 2), (w / 2, h / 2), (w - 3, 2), (w - 3, h - 3)] {
        let at = Point::new(b.x0 + (x as f64 + 0.5) * b.width() / w as f64, b.y0 + (y as f64 + 0.5) * b.height() / h as f64);
        let ([r, g, bl], a) = field.sample(at);
        let want = [r, g, bl, a].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i32);
        let got = png.get_pixel(x, y).0.map(i32::from);
        assert!(want.iter().zip(got).all(|(w, g)| (w - g).abs() <= 1), "({x}, {y}): {got:?} vs {want:?}");
    }
    // Read back, it looks as the canvas paints the gradient.
    let back = import(svg).unwrap();
    let mut images = 0;
    back.walk(|n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 1, "the image comes back (in its clip group)");
    assert_similar(&render_artboard(&d), &render_artboard(&back), 12.0, 0.01);
}

#[test]
fn a_freeform_stroke_is_an_image_clipped_to_the_stroke_outline() {
    let shape = shapes::rectangle(Rect::new(40.0, 40.0, 160.0, 120.0));
    let mut ap = Appearance::basic(Paint::solid(Color::WHITE), freeform(), 16.0);
    if let Some(AppearanceItem::Stroke(s)) = ap.items.get_mut(1) {
        s.opacity = 0.8;
    }
    let d = doc_with(Node::path(NodeId(0), shape, ap));
    let svg = export_full(&d, &ExportOptions::default(), None).svg;
    assert_eq!(svg.matches("<image ").count(), 1, "{svg}");
    assert!(svg.contains("opacity=\"0.8\"") && !svg.contains("linearGradient"), "{svg}");
    let back = import(&svg).unwrap();
    assert_similar(&render_artboard(&d), &render_artboard(&back), 12.0, 0.01);
}

#[test]
fn linked_images_write_the_gradient_next_to_the_svg_and_characters_stay_linear() {
    let d = doc_with(Node::path(NodeId(0), shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 120.0)), Appearance::basic(freeform(), Paint::None, 0.0)));
    let o = export_full(&d, &ExportOptions { images: ImageMode::Link, ..Default::default() }, None);
    assert_eq!(o.linked.len(), 1);
    assert!(o.linked[0].name.ends_with(".png") && o.svg.contains(&format!("xlink:href=\"{}\"", o.linked[0].name)), "{}", o.svg);
    // Characters keep a (linear) gradient paint, with a warning.
    let style = vectorcraft_doc::CharStyle { size: 30.0, fill: freeform(), ..Default::default() };
    let t = vectorcraft_doc::TextObject::point(Point::new(20.0, 60.0), "Hi", style);
    let o = export_full(&doc_with(Node::new(NodeId(0), NodeKind::Text(Box::new(t)))), &ExportOptions::default(), None);
    assert!(o.svg.contains("linearGradient") && !o.svg.contains("<image"), "{}", o.svg);
    assert!(o.warnings.iter().any(|w| w.contains("characters")), "{:?}", o.warnings);
}
