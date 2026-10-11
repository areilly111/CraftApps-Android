//! Brushed strokes and symbol-instance staining in the renderer.
//!
//! A stroke with `brush: Some(name)` is painted as the filled art `vectorcraft_brush::stroke_pieces`
//! generates. Pieces are cached per (node, stroke item) and validated by the node's path, the
//! stroke layer and the brush definition (an `Arc` from the per-frame parsed library), so edits to
//! any of them regenerate the art and nothing else does.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_brush::Brush;
use vectorcraft_doc::{AppearanceItem, Document, Node, StrokeLayer};
use vectorcraft_geom::{BezPath, Rect};
use vello_cpu::RenderContext;

use crate::{Frame, Renderer};

struct Entry {
    path: BezPath,
    stroke: StrokeLayer,
    brush: Arc<Brush>,
    pieces: Arc<Vec<Node>>,
    stamp: u64,
}

/// Parsed brush library and generated brush art.
#[derive(Default)]
pub(crate) struct BrushCache {
    /// The document value the library was parsed from (`None` = defaults).
    src: Option<Option<vectorcraft_brush::serde_json::Value>>,
    /// (frame stamp, document address) the library was last validated for.
    checked: (u64, usize),
    lib: HashMap<String, Arc<Brush>>,
    entries: HashMap<(u64, usize), Entry>,
}

impl BrushCache {
    fn library(&mut self, doc: &Document, stamp: u64) {
        let key = (stamp, doc as *const Document as usize);
        if self.checked == key {
            return;
        }
        self.checked = key;
        let cur = doc.unknown.get(vectorcraft_brush::DOC_KEY);
        if self.src.as_ref().is_some_and(|s| s.as_ref() == cur) {
            return;
        }
        self.src = Some(cur.cloned());
        self.lib = vectorcraft_brush::library(doc).into_iter().map(|b| (b.name.clone(), Arc::new(b))).collect();
    }
}

/// Cull bounds that allow for brush art reaching past the stroke (scatter, wide nibs).
pub(crate) fn cull_bounds(n: &Node) -> Option<Rect> {
    let b = crate::fx::cull_bounds(n)?;
    if !vectorcraft_brush::has_brush(n) {
        return Some(b);
    }
    let w = n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.width) } else { None }).fold(1.0, f64::max);
    let pad = 16.0 * w + 8.0;
    Some(b.inflate(pad, pad))
}

pub use vectorcraft_brush::instance_art;

impl Renderer {
    /// Paint stroke `st` of `n` (outline `bp`) with its brush. Returns false when the brush is
    /// unknown (the caller then draws a plain stroke).
    pub(crate) fn draw_brush(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, st: &StrokeLayer) -> bool {
        let Some(name) = st.brush.as_deref() else { return false };
        self.brushes.library(f.doc, self.stamp);
        let Some(brush) = self.brushes.lib.get(name).cloned() else { return false };
        let idx = n.appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st))).unwrap_or(usize::MAX);
        let key = (n.id.0, idx);
        let stamp = self.stamp;
        let pieces = match self.brushes.entries.get_mut(&key) {
            Some(e) if Arc::ptr_eq(&e.brush, &brush) && e.stroke == *st && e.path == *bp => {
                e.stamp = stamp;
                e.pieces.clone()
            }
            _ => {
                let pieces = Arc::new(vectorcraft_brush::stroke_pieces(&brush, bp, st));
                if self.brushes.entries.len() > 2048 {
                    self.brushes.entries.retain(|_, e| stamp.saturating_sub(e.stamp) <= 3);
                }
                self.brushes.entries.insert(key, Entry { path: bp.clone(), stroke: st.clone(), brush, pieces: pieces.clone(), stamp });
                pieces
            }
        };
        let mut draw = |r: &mut Self, c: &mut RenderContext, fr: &Frame| {
            for p in pieces.iter() {
                r.draw_node(c, fr, p, true);
            }
        };
        if st.opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal {
            self.group(ctx, f, crate::group::Composite { blend: st.blend, opacity: st.opacity, ..Default::default() }, &mut draw);
        } else {
            draw(self, ctx, f);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::{Affine, Rect, shapes};

    use crate::{RenderOptions, Rendered, Renderer};

    fn doc_with_line(brush: Option<&str>, width: f64) -> Document {
        let mut d = Document::new(200.0, 100.0);
        let id = d.alloc_id();
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), width);
        ap.stroke_mut().unwrap().brush = brush.map(str::to_string);
        let n = Node::path(id, shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()), ap);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    }

    fn render(r: &mut Renderer, d: &Document) -> Rendered {
        r.render(d, 200, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    }

    fn dark(p: [u8; 4]) -> bool {
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 200
    }

    #[test]
    fn calligraphic_brush_paints_wider_than_the_stroke() {
        let mut r = Renderer::new();
        let plain = render(&mut r, &doc_with_line(None, 1.0));
        assert!(!dark(plain.pixel(100, 53)));
        let d = doc_with_line(Some("10 pt. Oval"), 1.0);
        let img = render(&mut r, &d);
        assert!(dark(img.pixel(100, 50)) && dark(img.pixel(100, 52)), "brush art is painted: {:?}", img.pixel(100, 52));
        assert!(!dark(img.pixel(100, 60)));
        // Second frame hits the cache and paints the same.
        let again = render(&mut r, &d);
        assert_eq!(again.pixels, img.pixels);
    }

    #[test]
    fn unknown_brush_falls_back_to_plain_stroke_and_scatter_renders() {
        let mut r = Renderer::new();
        let img = render(&mut r, &doc_with_line(Some("No Such Brush"), 2.0));
        assert!(dark(img.pixel(100, 50)));
        let img = render(&mut r, &doc_with_line(Some("Dots"), 1.0));
        let dark_px = (20..180).filter(|x| dark(img.pixel(*x, 50))).count();
        assert!(dark_px > 20 && dark_px < 150, "dots leave gaps: {dark_px}");
    }

    #[test]
    fn brush_definition_edits_rerender() {
        let mut r = Renderer::new();
        let mut d = doc_with_line(Some("3 pt. Round"), 1.0);
        let a = render(&mut r, &d);
        let mut lib = vectorcraft_brush::library(&d);
        if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut lib[0].kind {
            c.size = 20.0;
        }
        vectorcraft_brush::store(&mut d, &lib);
        let b = render(&mut r, &d);
        assert!(!dark(a.pixel(100, 58)) && dark(b.pixel(100, 58)));
    }

    #[test]
    fn stained_symbol_instance() {
        let mut d = Document::new(100.0, 100.0);
        let art = Node::path(
            vectorcraft_doc::NodeId(0),
            shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: std::sync::Arc::new(art) });
        let id = d.alloc_id();
        let mut inst = Node::new(id, vectorcraft_doc::NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((50.0, 50.0)) });
        inst.appearance.set_fill(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, inst).unwrap();
        let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
        assert_eq!(img.pixel(50, 50), [255, 0, 0, 255]);
    }
}
