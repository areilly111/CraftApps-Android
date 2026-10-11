//! Layer Options on screen: Preview off draws a layer in outline, Dim Images fades its images;
//! exports (which leave templates out) draw both as they are.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::shapes;

use super::*;

/// A red square (10..40) and a 30×30 red image at (50, 10), on one layer set up by `layer`.
fn doc(layer: impl FnOnce(&mut NodeKind)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let img = image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let l = d.layers[0].id;
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let sq = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), red);
    d.insert(Some(l), 0, sq).unwrap();
    let mut im = Node::new(
        NodeId(0),
        NodeKind::Image(ImageObject {
            key: "red".into(),
            width: 30,
            height: 30,
            xf: Affine::translate((50.0, 10.0)),
            link: None,
            placement: Default::default(),
        }),
    );
    im.id = d.alloc_id();
    d.insert(Some(l), 9, im).unwrap();
    layer(&mut d.node_mut(l).unwrap().kind);
    d
}

fn render(d: &Document, export: bool) -> Rendered {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), skip_templates: export, ..Default::default() };
    Renderer::new().render(d, 100, 100, Affine::IDENTITY, &opts)
}

#[test]
fn preview_off_draws_the_layer_in_outline_on_screen_only() {
    let d = doc(|k| {
        if let NodeKind::Layer { preview, .. } = k {
            *preview = false;
        }
    });
    assert_eq!(render(&d, false).pixel(25, 25), [255, 255, 255, 255], "no fill in outline");
    assert_eq!(render(&d, true).pixel(25, 25), [255, 0, 0, 255], "exports draw it as it is");
}

#[test]
fn dim_images_fades_only_the_images() {
    let plain = render(&doc(|_| {}), false);
    assert_eq!(plain.pixel(65, 25), [255, 0, 0, 255]);
    let d = doc(|k| {
        if let NodeKind::Layer { dim_images, .. } = k {
            *dim_images = Some(50);
        }
    });
    let img = render(&d, false);
    let [r, g, b, _] = img.pixel(65, 25);
    assert!(r == 255 && (120..=135).contains(&g) && g == b, "half way to white: {:?}", img.pixel(65, 25));
    assert_eq!(img.pixel(25, 25), [255, 0, 0, 255], "vector art stays");
    assert_eq!(render(&d, true).pixel(65, 25), [255, 0, 0, 255], "exports are not dimmed");
}

/// A guide path draws on screen only: exports leave it out, as they leave templates out (and as
/// the SVG, PSD and PDF exports and printing already do).
#[test]
fn guides_draw_on_screen_only() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let mut g = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(70.0, 70.0, 90.0, 90.0)), Appearance::default());
    if let NodeKind::Path { guide, .. } = &mut g.kind {
        *guide = true;
    }
    d.insert(Some(l), 0, g).unwrap();
    let drawn = |export: bool| {
        let r = render(&d, export);
        (60..100).flat_map(|y| (60..100).map(move |x| (x, y))).filter(|&(x, y)| r.pixel(x, y) != [255, 255, 255, 255]).count()
    };
    assert!(drawn(false) > 0, "on screen the guide shows");
    assert_eq!(drawn(true), 0, "exports leave it out");
}
