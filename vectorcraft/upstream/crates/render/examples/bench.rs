//! `cargo run --release -p vectorcraft-render --example bench` — render timing on synthetic documents.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::{RenderOptions, Renderer};

fn rng(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed % 1_000_000) as f64 / 1_000_000.0
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let mut d = Document::new(1600.0, 1200.0);
    let l = d.layers[0].id;
    let mut seed = 42u64;
    for i in 0..n {
        let x = rng(&mut seed) * 1600.0;
        let y = rng(&mut seed) * 1200.0;
        let r = 3.0 + rng(&mut seed) * 22.0;
        let c = Color::rgb(rng(&mut seed) as f32, rng(&mut seed) as f32, rng(&mut seed) as f32);
        let id = d.alloc_id();
        let mut node = match i % 3 {
            0 => Node::path(
                id,
                shapes::ellipse(Rect::from_center_size(Point::new(x, y), (2.0 * r, 2.0 * r))),
                Appearance::basic(Paint::solid(c), Paint::solid(Color::BLACK), 0.5),
            ),
            1 => Node::path(id, shapes::rectangle(Rect::new(x, y, x + 2.0 * r, y + r)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)),
            _ => Node::path(id, shapes::star(Point::new(x, y), r, r / 2.0, 5, 0.0), Appearance::basic(Paint::None, Paint::solid(c), 2.0)),
        };
        if i % 3 == 1 {
            node.opacity = 0.8;
        }
        d.insert(Some(l), usize::MAX, node).unwrap();
    }
    // Text objects (1 per 20 shapes).
    for i in 0..n / 20 {
        let x = rng(&mut seed) * 1500.0;
        let y = rng(&mut seed) * 1150.0 + 20.0;
        let id = d.alloc_id();
        let t = vectorcraft_doc::TextObject::point(
            Point::new(x, y),
            &format!("Label {i} — VectorCraft"),
            vectorcraft_doc::CharStyle { size: 10.0 + (i % 5) as f64 * 4.0, ..Default::default() },
        );
        d.insert(Some(l), usize::MAX, Node::new(id, vectorcraft_doc::NodeKind::Text(Box::new(t)))).unwrap();
    }
    let mut r = Renderer::new();
    if let Ok(t) = std::env::var("THREADS") {
        r.threads = t.parse().unwrap_or(0);
    }
    let iters: usize = std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let opts = RenderOptions::default();
    for (label, w, h, view) in [
        ("fit 2880x1800", 2880u32, 1800u32, Affine::scale(1.6)),
        ("zoom 400% 2880x1800", 2880, 1800, Affine::scale(6.4) * Affine::translate((-600.0, -400.0))),
        ("thumb 400x300", 400, 300, Affine::scale(0.25)),
    ] {
        r.render(&d, w, h, view, &opts); // warm up
        let t = std::time::Instant::now();
        for _ in 0..iters {
            r.render(&d, w, h, view, &opts);
        }
        println!(
            "{n} objects, {label}: {:.1} ms/frame (drawn {}, culled {})",
            t.elapsed().as_secs_f64() * 1000.0 / iters as f64,
            r.stats.drawn,
            r.stats.culled
        );
    }
}
