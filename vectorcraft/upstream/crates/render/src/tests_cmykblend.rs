//! Blending in CMYK documents ([`crate::ink`]): blend modes and opacity composite inks.

use super::*;
use vectorcraft_color::blend::MASK_LUM;
use vectorcraft_color::{BlendMode as Blend, Color, Paint};
use vectorcraft_doc::{Appearance, ColorMode, ImageBlob, ImageObject, Node, OpacityMask};
use vectorcraft_geom::shapes;

fn rect(d: &mut Document, r: Rect, c: Color) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

fn add(d: &mut Document, n: Node) {
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// `below` over the whole 20 pt page and `top` (10 pt in the middle) with `mode` and `opacity`.
fn doc(mode: ColorMode, below: Option<Color>, top: Color, blend: Blend, opacity: f32) -> Document {
    let mut d = Document::new_with_mode(20.0, 20.0, mode);
    if let Some(b) = below {
        let n = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), b);
        add(&mut d, n);
    }
    let mut n = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), top);
    (n.blend, n.opacity) = (blend, opacity);
    add(&mut d, n);
    d
}

/// The centre pixel, rendered on white.
fn centre(d: &Document) -> [u8; 4] {
    Renderer::new().render_region(d, d.artboards[0].rect, 1.0, true).pixel(10, 10)
}

/// Screen colour of inks `cmyk`.
fn shown(cmyk: [f32; 4]) -> [u8; 4] {
    Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3]).to_rgba8(1.0)
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

const CYAN: Color = Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 };
const MAGENTA: Color = Color::Cmyk { c: 0.0, m: 1.0, y: 0.0, k: 0.0 };

#[test]
fn cyan_multiplied_over_magenta_prints_both_inks() {
    let got = centre(&doc(ColorMode::Cmyk, Some(MAGENTA), CYAN, Blend::Multiply, 1.0));
    let want = shown([1.0, 1.0, 0.0, 0.0]);
    assert!(near(got, want, 1), "{got:?} ≠ C100 M100 {want:?}");
    // Multiplying the screen colours instead gives a visibly different blue.
    let (c, m) = (CYAN.to_rgb(), MAGENTA.to_rgb());
    let screen = [0, 1, 2].map(|i| (c[i] * m[i] * 255.0).round() as i32);
    assert!((0..3).any(|i| (screen[i] - want[i] as i32).abs() > 10), "{screen:?} vs {want:?}");
}

#[test]
fn opacity_mixes_inks() {
    let got = centre(&doc(ColorMode::Cmyk, None, CYAN, Blend::Normal, 0.5));
    assert!(near(got, shown([0.5, 0.0, 0.0, 0.0]), 2), "{got:?}");
}

#[test]
fn black_blends_on_its_own_plane() {
    let k = |k| Color::cmyk(0.0, 0.0, 0.0, k);
    // Multiply adds black ink: K50 over K50 leaves 25% of the paper.
    let got = centre(&doc(ColorMode::Cmyk, Some(k(0.5)), k(0.5), Blend::Multiply, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.75]), 2), "{got:?}");
    // Luminosity takes the source's black, Color keeps the backdrop's.
    let got = centre(&doc(ColorMode::Cmyk, Some(CYAN), k(0.6), Blend::Luminosity, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.6]), 3), "{got:?}");
    // (Over a backdrop of black ink alone the C, M, Y plane is paper: its luminance takes the
    // source's colour out.)
    let got = centre(&doc(ColorMode::Cmyk, Some(k(0.5)), CYAN, Blend::Color, 1.0));
    assert!(near(got, shown([0.0, 0.0, 0.0, 0.5]), 2), "{got:?}");
}

#[test]
fn rgb_documents_and_opaque_cmyk_documents_keep_screen_colours() {
    let rgb = |c: &Color| {
        let [r, g, b] = c.to_rgb();
        Color::rgb(r, g, b)
    };
    // RGB documents multiply screen colours.
    let got = centre(&doc(ColorMode::Rgb, Some(rgb(&MAGENTA)), rgb(&CYAN), Blend::Multiply, 1.0));
    let (c, m) = (CYAN.to_rgb(), MAGENTA.to_rgb());
    let want = [0, 1, 2].map(|i| (c[i] * m[i] * 255.0).round() as u8);
    assert!(near(got, [want[0], want[1], want[2], 255], 1), "{got:?} ≠ {want:?}");
    // A CMYK document without transparency paints each colour as it is.
    let d = doc(ColorMode::Cmyk, Some(MAGENTA), CYAN, Blend::Normal, 1.0);
    assert!(!ink::blends_in_cmyk(&d, &RenderOptions::default()));
    assert_eq!(centre(&d), CYAN.to_rgba8(1.0));
}

#[test]
fn opacity_masks_take_screen_luminance() {
    let mut d = Document::new_with_mode(20.0, 20.0, ColorMode::Cmyk);
    let mut n = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let art = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), CYAN);
    n.mask = Some(Box::new(OpacityMask::new(art, true)));
    add(&mut d, n);
    let [r, g, b] = CYAN.to_rgb();
    let cover = MASK_LUM[0] * r + MASK_LUM[1] * g + MASK_LUM[2] * b;
    let got = centre(&d);
    assert!(near(got, shown([0.0, 0.0, 0.0, cover]), 3), "{got:?} ≠ K{cover}");
}

#[test]
fn placed_images_are_separated_not_blackened() {
    let mut d = doc(ColorMode::Cmyk, None, CYAN, Blend::Normal, 0.5);
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let n = Node::new(
        d.alloc_id(),
        NodeKind::Image(ImageObject { key: "red".into(), width: 4, height: 4, xf: Affine::IDENTITY, link: None, placement: Default::default() }),
    );
    add(&mut d, n);
    let got = Renderer::new().render_region(&d, d.artboards[0].rect, 1.0, true).pixel(1, 1);
    let cms = vectorcraft_color::cms::active();
    let [r, g, b] = cms.cmyk_to_srgb(cms.srgb_to_cmyk([1.0, 0.0, 0.0], cms.settings().intent), false).map(|v| (v * 255.0).round() as u8);
    assert!(near(got, [r, g, b, 255], 4), "{got:?} ≠ {:?}", [r, g, b]);
}
