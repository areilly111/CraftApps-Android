//! What renderers keep between frames: decoded images shared by all of them, image mips, and
//! caches swept by every kind of render.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};

use super::*;

/// A `w`×`h` PNG whose pixels depend on `seed` (each test its own image: the store is shared by
/// the tests running alongside).
fn png(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let v = (x * 7 + y * 13) as u8;
        image::Rgba([v ^ seed, v.wrapping_mul(3), seed, 255])
    });
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A document showing `bytes` as image `key` of `w`×`h` pixels, one point a pixel.
fn doc_with_image(key: &str, bytes: Vec<u8>, w: u32, h: u32) -> Document {
    let mut d = Document::new(w as f64, h as f64);
    d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
    let (id, layer) = (d.alloc_id(), d.layers[0].id);
    let im = ImageObject { key: key.into(), width: w, height: h, xf: Affine::IDENTITY, link: None, placement: Default::default() };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    d
}

#[test]
fn renderers_share_one_decode_of_an_image() {
    let bytes = png(16, 16, 101);
    let a = doc_with_image("img", bytes.clone(), 16, 16);
    // Another document holding the same image under another key (its own copy of the bytes).
    let b = doc_with_image("photo", bytes, 16, 16);
    let (mut r1, mut r2) = (Renderer::new(), Renderer::new());
    let (k1, p1) = r1.image_pixmap(&a, "img", false, true).unwrap();
    let (k2, p2) = r2.image_pixmap(&b, "photo", false, true).unwrap();
    assert_eq!(k1, k2);
    assert!(Arc::ptr_eq(&p1, &p2), "decoded once for both renderers");
    // Drawing goes through the store too: the pixels stay the ones decoded.
    let _ = r2.render(&b, 16, 16, Affine::IDENTITY, &RenderOptions::default());
    assert!(Arc::ptr_eq(&r1.image_pixmap(&b, "photo", false, true).unwrap().1, &p1));
    // A document using the same key for other pixels gets its own.
    let c = doc_with_image("img", png(16, 16, 102), 16, 16);
    let (k3, p3) = r1.image_pixmap(&c, "img", false, true).unwrap();
    assert!(k3 != k1 && !Arc::ptr_eq(&p3, &p1));
    assert!(
        r1.render(&c, 16, 16, Affine::IDENTITY, &RenderOptions::default()).pixels
            != r1.render(&a, 16, 16, Affine::IDENTITY, &RenderOptions::default()).pixels
    );
}

/// Thumbnails and single-node renders end their frame as full renders do: entries of nodes no
/// longer drawn are dropped, with the nodes they kept alive.
#[test]
fn node_renders_sweep_stale_cache_entries() {
    let mut d = Document::new(10.0, 10.0);
    let id = d.alloc_id();
    let square = |x: f64| {
        let p = vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 5.0, 5.0));
        Arc::new(Node::path(id, p, vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0)))
    };
    let first = square(0.0);
    let mut r = Renderer::new();
    r.render_node(&d, &first, 10, 10, Affine::IDENTITY).unwrap();
    assert!(Arc::strong_count(&first) > 1, "cached while drawn");
    for i in 0..1100 {
        let n = square(i as f64);
        if i % 2 == 0 {
            r.render_node(&d, &n, 10, 10, Affine::IDENTITY).unwrap();
        } else {
            // A thumbnail of a group caches its member's geometry.
            r.render_node_thumbnail(&d, &Node::group(id, vec![n]), 16, None).unwrap();
        }
    }
    assert_eq!(Arc::strong_count(&first), 1, "swept: the renderer no longer keeps the node");
    assert!(r.geom.len() <= 1025, "{} geometry entries", r.geom.len());
}

/// With `image_mips` off (the default; exports) an image draws from its full pixels whatever the
/// store holds; on, a reduced image draws from a box-filtered half (quarter…) instead, and at
/// more than half size from the full pixels as before.
#[test]
fn image_mips_are_opt_in_and_change_nothing_off() {
    let d = doc_with_image("img", png(64, 64, 103), 64, 64);
    let (off, on) = (RenderOptions::default(), RenderOptions { image_mips: true, ..Default::default() });
    let quarter = Affine::scale(0.25);
    let cold = Renderer::new().render(&d, 16, 16, quarter, &off);
    // Mips made by another renderer don't change what this one draws with them off.
    let mipped = Renderer::new().render(&d, 16, 16, quarter, &on);
    let mut warm = Renderer::new();
    assert!(warm.render(&d, 16, 16, quarter, &off).pixels == cold.pixels);
    assert!(warm.render(&d, 16, 16, quarter, &off).pixels == cold.pixels, "warm");
    // On, the quarter-size image samples the quarter-size mip: close to the full pixels' average
    // but not the same pixels.
    assert!(mipped.pixels != cold.pixels);
    let (key, _) = warm.image_pixmap(&d, "img", false, true).unwrap();
    let mip = images::store().peek(&images::ImageKey { mip: 2, ..key }).expect("quarter mip kept");
    assert_eq!((mip.width(), mip.height()), (16, 16));
    // Above half size, mips on draw the full pixels.
    let near = Affine::scale(0.75);
    assert!(Renderer::new().render(&d, 48, 48, near, &on).pixels == Renderer::new().render(&d, 48, 48, near, &off).pixels);
}

/// A document of `n` point type objects reading `word`, in rows.
fn text_doc(n: usize, word: &str) -> Document {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    let mut d = Document::new(400.0, 400.0);
    let l = d.layers[0].id;
    for i in 0..n {
        let at = vectorcraft_geom::Point::new(10.0 + (i % 8) as f64 * 48.0, 30.0 + (i / 8) as f64 * 30.0);
        let style = CharStyle { size: 18.0, ..Default::default() };
        let mut t = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(at, word, style))));
        t.id = d.alloc_id();
        d.insert(Some(l), 0, t).unwrap();
    }
    d
}

/// The cache keys of `d`'s type objects (their nodes' addresses).
fn text_keys(d: &Document) -> Vec<usize> {
    let mut keys = vec![];
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            keys.push(n as *const Node as usize);
        }
    });
    keys
}

/// More type than the glyph cache's budget holds: the entries kept stay from frame to frame (the
/// rest is laid out again), where emptying the cache when full laid out every object every frame.
#[test]
fn type_over_the_text_budget_keeps_its_entries_between_frames() {
    let d = text_doc(40, "Type");
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let want = r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels;
    assert_eq!(r.texts.len(), 40);
    let all = r.texts.bytes();
    let mut r = Renderer::new();
    r.texts = FrameCache::new(all / 2, 1024);
    let keys = text_keys(&d);
    let kept = |r: &Renderer| keys.iter().filter(|k| r.texts.contains_key(*k)).copied().collect::<Vec<_>>();
    assert_eq!(r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels, want);
    let first = kept(&r);
    assert!(!first.is_empty() && first.len() < 40 && r.texts.bytes() <= all / 2, "{} entries, {} bytes", first.len(), r.texts.bytes());
    for _ in 0..3 {
        assert_eq!(r.render(&d, 400, 400, Affine::IDENTITY, &opts).pixels, want);
        assert_eq!(kept(&r), first, "the same entries stay");
    }
    // Other type: its entries replace the earlier frames' ones, within the budget.
    let e = text_doc(40, "Other");
    r.render(&e, 400, 400, Affine::IDENTITY, &opts);
    assert!(kept(&r).is_empty(), "entries of earlier frames went first");
    assert!(!r.texts.is_empty() && r.texts.bytes() <= all / 2);
}

/// Type no longer drawn leaves the glyph cache after a few frames, and its bytes with it.
#[test]
fn swept_type_entries_give_back_their_bytes() {
    let d = text_doc(12, "Swept");
    let mut r = Renderer::new();
    r.texts = FrameCache::new(lru::TEXT_BUDGET, 0);
    r.render(&d, 400, 400, Affine::IDENTITY, &RenderOptions::default());
    let full = r.texts.bytes();
    assert!(full > 0 && r.texts.len() == 12);
    let empty = Document::new(10.0, 10.0);
    for _ in 0..5 {
        r.render(&empty, 10, 10, Affine::IDENTITY, &RenderOptions::default());
    }
    assert_eq!((r.texts.len(), r.texts.bytes()), (0, 0));
}

/// Each run draws exactly the outlines it drew when kept as a path of its own: its glyphs and
/// decorations in layout order (the whole text's path is kept once, the runs as spans of it).
#[test]
fn text_runs_draw_their_own_glyphs_from_the_shared_outlines() {
    use vectorcraft_doc::text::{CharStyle, TextObject, TextRun};
    let plain = CharStyle { size: 24.0, ..Default::default() };
    let under = CharStyle { underline: true, ..plain.clone() };
    let mut t = TextObject::point(vectorcraft_geom::Point::new(10.0, 40.0), "Ab", plain.clone());
    t.runs.push(TextRun::new("cd", under));
    t.runs.push(TextRun::new("ef", plain));
    let g = text_geom(&t);
    let db = vectorcraft_text::FontDb::global();
    let layout = vectorcraft_text::layout(db, &t);
    let mut want = vec![BezPath::new(); t.runs.len()];
    for gl in &layout.glyphs {
        want[gl.run].extend(gl.outline.iter());
    }
    for (run, bar) in vectorcraft_text::decorations(&layout, db, &t) {
        want[run].extend(bar.iter());
    }
    for (i, w) in want.iter().enumerate() {
        assert_eq!(g.run(i).elements(), w.elements(), "run {i}");
    }
    assert!(g.run(7).elements().is_empty());
}
