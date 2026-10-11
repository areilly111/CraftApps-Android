//! Ruler guides on the canvas. Dragged out of a ruler with any tool ([`NewGuide`]), a guide is
//! made where it is released over the canvas: with the Artboard tool, an artboard guide of the
//! active artboard. For the selection tools ([`GuideEdit`]), a press on a guide selects it (Shift
//! adds it to the selected guides or, released without a drag, takes it out); dragging moves the
//! selected guides (Alt copies them). A dragged guide snaps ([`GuideSnap`]): with Shift to the
//! ruler's ticks, else to whole pixels, the grid, with Smart Guides the art's and the artboards'
//! edges, centres and anchors, or with Snap to Point the anchors. Dropped off the canvas onto its
//! ruler, a moved guide is deleted. Hidden or locked guides (View › Guides) can't be picked.

use serde_json::json;
use vectorcraft_geom::Point;

use crate::guides::Targets;
use crate::{Action, Cursor, Overlay, PointerEvent, PointerKind, ToolContext};

/// The ruler guide under `p`: the nearest within the selection tolerance (the newest of equals).
pub fn guide_at(cx: &ToolContext, p: Point) -> Option<usize> {
    if !cx.guides {
        return None;
    }
    let tol = cx.pick_tol();
    let off = |pos: f64, vertical: bool| (if vertical { p.x } else { p.y } - pos).abs();
    cx.doc
        .guides
        .iter()
        .enumerate()
        .filter(|(_, g)| cx.doc.guide_passes(g, p, tol))
        .map(|(i, g)| (i, off(g.pos, g.vertical)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(i, _)| i)
}

/// The pointer for a guide: it moves sideways if vertical, up and down if horizontal.
fn guide_cursor(vertical: bool) -> Cursor {
    if vertical { Cursor::ResizeH } else { Cursor::ResizeV }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    index: usize,
    vertical: bool,
    /// Where the guide was, and where it is now (x of a vertical guide, y of a horizontal one).
    from: f64,
    at: f64,
    start: Point,
    pointer: Point,
    began: bool,
    /// Shift-pressed on a selected guide: released without a drag, it leaves the selection.
    deselect: bool,
}

/// Snapping for a dragged ruler guide, its targets gathered when the drag begins.
#[derive(Default)]
struct GuideSnap {
    /// With Smart Guides the art's and the artboards' edges, centres and anchors, else with Snap
    /// to Point the anchors, and how near (in screen pixels) they pull; none with pixels or the
    /// grid snapping instead, or nothing on.
    targets: Option<(Targets, f64)>,
    /// The target the guide snapped to.
    snapped: Vec<Overlay>,
}

impl GuideSnap {
    fn new(cx: &ToolContext) -> Self {
        let targets = || Targets::collect(cx.doc, &[], None);
        let targets = if cx.snap_to_pixel || cx.snap_to_grid {
            None
        } else if cx.smart_guides {
            Some((targets().styled(cx), cx.snapping_tolerance))
        } else if cx.snap_to_point {
            Some((targets().anchors_only(), cx.snap_tolerance))
        } else {
            None
        };
        Self { targets, snapped: vec![] }
    }

    /// Where a guide dragged to `v` (the x of a vertical one, the y of a horizontal one) goes:
    /// with Shift on the nearest ruler tick, else on whole pixels, the grid or a target.
    fn snap(&mut self, cx: &ToolContext, vertical: bool, v: f64, shift: bool) -> f64 {
        self.snapped.clear();
        if shift {
            return cx.unit.snap_to_ruler_tick(v, cx.zoom);
        }
        if cx.snap_to_pixel {
            return v.round();
        }
        if cx.snap_to_grid {
            return vectorcraft_geom::snap::snap_to_grid(v, cx.grid_step(), 0.0);
        }
        let Some((t, tol)) = &self.targets else { return v };
        let (at, snapped) = t.snap_guide(vertical, v, cx.tol(*tol));
        self.snapped = snapped;
        at
    }

    /// The target the guide at `at` snapped to, and its position by the `pointer`.
    fn overlays(&self, cx: &ToolContext, vertical: bool, at: f64, pointer: Point) -> Vec<Overlay> {
        let mut o = self.snapped.clone();
        if cx.measurement_labels {
            let axis = if vertical { "X" } else { "Y" };
            o.push(Overlay::Measure { p: pointer, text: format!("{axis}: {}", cx.len(at)) });
        }
        o
    }
}

/// A guide dragged out of a ruler (a vertical one out of the left ruler), with any tool: it shows
/// where the pointer is, snapped as a moved guide is, and is made where the button is released
/// over the canvas (`guide.add`, previewed as it goes); taken off the canvas, it goes again.
pub struct NewGuide {
    vertical: bool,
    /// The artboard (index) it belongs to: an artboard guide.
    artboard: Option<usize>,
    snap: GuideSnap,
    /// Where it shows (none off the canvas), and the pointer.
    at: Option<f64>,
    pointer: Point,
}

impl NewGuide {
    /// A vertical guide out of the left ruler, else a horizontal one out of the top ruler; an
    /// artboard guide of `artboard`, if any.
    pub fn new(cx: &ToolContext, vertical: bool, artboard: Option<usize>) -> Self {
        Self { vertical, artboard, snap: GuideSnap::new(cx), at: None, pointer: Point::ZERO }
    }

    /// The pointer at `ev`, over the canvas or not (`on_canvas`): the actions that show the guide
    /// there or take it away, and on release make it.
    pub fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent, on_canvas: bool) -> Vec<Action> {
        let mut out = vec![];
        self.pointer = ev.pos;
        if on_canvas {
            let at = self.snap.snap(cx, self.vertical, if self.vertical { ev.pos.x } else { ev.pos.y }, ev.mods.shift);
            if self.at.is_none() {
                out.push(Action::Begin("New Guide".into()));
            }
            if self.at != Some(at) {
                let mut p = json!({ "vertical": self.vertical, "pos": at });
                if let Some(ab) = self.artboard {
                    p["artboard"] = json!(ab);
                }
                out.push(Action::Preview("guide.add".into(), p));
            }
            self.at = Some(at);
        } else if self.at.take().is_some() {
            self.snap.snapped.clear();
            out.push(Action::Cancel);
        }
        if ev.kind == PointerKind::Up && self.at.take().is_some() {
            out.push(Action::Commit);
        }
        out
    }

    /// While the guide shows: the target it snapped to and its position by the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        self.at.map(|at| self.snap.overlays(cx, self.vertical, at, self.pointer)).unwrap_or_default()
    }
}

/// A selection tool's ruler-guide handling: [`Self::press`] on a press, [`Self::pointer`] for
/// the rest of the gesture.
#[derive(Default)]
pub struct GuideEdit {
    drag: Option<Drag>,
    snap: GuideSnap,
}

impl GuideEdit {
    /// Is a guide pressed or being dragged?
    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }

    /// A press `ev` on a ruler guide: it gets selected and the drag starts. None (and nothing
    /// done) when no guide is there.
    pub fn press(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let index = guide_at(cx, ev.pos)?;
        let g = cx.doc.guides.get(index)?;
        let (shift, selected) = (ev.mods.shift, cx.selection.guides.contains(&index));
        self.drag = Some(Drag {
            index,
            vertical: g.vertical,
            from: g.pos,
            at: g.pos,
            start: ev.pos,
            pointer: ev.pos,
            began: false,
            deselect: shift && selected,
        });
        self.snap = GuideSnap::default();
        Some(match (selected, shift) {
            (true, _) => vec![],
            (false, true) => vec![Action::Exec("guide.select".into(), json!({ "indexes": [index], "toggle": true }))],
            (false, false) => vec![Action::Exec("guide.select".into(), json!({ "indexes": [index] }))],
        })
    }

    /// The drag and release of a guide [`Self::press`] picked; None when no guide is pressed.
    pub fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Option<Vec<Action>> {
        let mut d = self.drag?;
        Some(match ev.kind {
            PointerKind::Drag => {
                let out = self.drag_to(cx, &mut d, ev);
                self.drag = Some(d);
                out
            }
            PointerKind::Up => {
                self.drag = None;
                self.snap = GuideSnap::default();
                if d.began && on_ruler(cx, d.vertical, ev.pos) {
                    // Dropped on its ruler: the guide goes (a copy is just not made).
                    let mut out = vec![Action::Cancel];
                    if !ev.mods.alt {
                        out.push(Action::Exec("guide.remove".into(), json!({ "index": d.index })));
                    }
                    out
                } else if d.began {
                    vec![Action::Commit]
                } else if d.deselect {
                    vec![Action::Exec("guide.select".into(), json!({ "indexes": [d.index], "toggle": true }))]
                } else {
                    vec![]
                }
            }
            // Another press ends the gesture.
            _ => {
                self.drag = None;
                return None;
            }
        })
    }

    fn drag_to(&mut self, cx: &ToolContext, d: &mut Drag, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let mut out = vec![];
        if !d.began {
            if p.distance(d.start) < cx.tol(3.0) {
                return out;
            }
            d.began = true;
            out.push(Action::Begin(if ev.mods.alt { "Copy Guide" } else { "Move Guide" }.into()));
            self.snap = GuideSnap::new(cx);
        }
        let moved = p - d.start;
        let at = self.snap.snap(cx, d.vertical, d.from + if d.vertical { moved.x } else { moved.y }, ev.mods.shift);
        (d.at, d.pointer) = (at, p);
        // The other selected guides follow the pointer: vertical ones across, horizontal ones down.
        let (dx, dy) = if d.vertical { (at - d.from, moved.y) } else { (moved.x, at - d.from) };
        out.push(Action::Preview("guide.move".into(), json!({ "dx": dx, "dy": dy, "copy": ev.mods.alt })));
        out
    }

    /// The pointer over a guide, or while one is dragged.
    pub fn cursor(&self, cx: &ToolContext, p: Point) -> Option<Cursor> {
        if let Some(d) = &self.drag {
            return Some(guide_cursor(d.vertical));
        }
        guide_at(cx, p).and_then(|i| cx.doc.guides.get(i)).map(|g| guide_cursor(g.vertical))
    }

    /// While a guide is dragged: the smart guide it snapped to and its position by the pointer.
    pub fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(d) = self.drag.filter(|d| d.began) else { return vec![] };
        self.snap.overlays(cx, d.vertical, d.at, d.pointer)
    }
}

/// Is `p` off the canvas on the side of the ruler a guide comes from (the top one for a
/// horizontal guide, the left one for a vertical guide)? Never without a window.
fn on_ruler(cx: &ToolContext, vertical: bool, p: Point) -> bool {
    cx.screen.and_then(|s| s.to_screen(p)).is_some_and(|(x, y)| if vertical { x < 0.0 } else { y < 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{Mods, ScreenFrame};
    use vectorcraft_doc::{Guide, Selection};
    use vectorcraft_geom::Vec2;

    fn ev(kind: PointerKind, x: f64, y: f64, mods: Mods) -> PointerEvent {
        PointerEvent::new(kind, x, y).with_mods(mods)
    }

    fn guides_doc() -> vectorcraft_doc::Document {
        let (mut d, _) = doc_with_rect();
        d.guides = vec![Guide::new(true, 300.0), Guide::new(false, 400.0)];
        d
    }

    #[test]
    fn picks_the_guide_within_the_tolerance_unless_hidden_or_locked() {
        let (d, s, p) = (guides_doc(), Selection::default(), paint());
        let c = cx(&d, &s, &p);
        assert_eq!(guide_at(&c, Point::new(302.0, 50.0)), Some(0));
        assert_eq!(guide_at(&c, Point::new(50.0, 398.0)), Some(1));
        // Where they cross, the nearer one.
        assert_eq!(guide_at(&c, Point::new(301.0, 398.0)), Some(0));
        assert_eq!(guide_at(&c, Point::new(305.0, 50.0)), None, "beyond the 3 px tolerance");
        let hidden = ToolContext { guides: false, ..cx(&d, &s, &p) };
        assert_eq!(guide_at(&hidden, Point::new(300.0, 50.0)), None);
    }

    #[test]
    fn a_drag_moves_the_selected_guides_in_one_step() {
        let d = guides_doc();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut g = GuideEdit::default();
        let m = Mods::default();
        assert_eq!(g.press(&c, &ev(PointerKind::Down, 300.0, 50.0, m)), Some(vec![Action::Exec("guide.select".into(), json!({"indexes": [0]}))]));
        // The tool sees the guide selected from now on.
        let mut s = s;
        s.set_guides([0]);
        let c = cx(&d, &s, &p);
        let a = g.pointer(&c, &ev(PointerKind::Drag, 340.0, 60.0, m)).unwrap();
        assert_eq!(a[0], Action::Begin("Move Guide".into()));
        assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 40.0, "dy": 10.0, "copy": false})));
        assert!(matches!(g.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == "X: 340.00 pt"));
        assert_eq!(g.cursor(&c, Point::ZERO), Some(Cursor::ResizeH));
        assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 340.0, 60.0, m)), Some(vec![Action::Commit]));
        assert!(!g.busy() && g.pointer(&c, &ev(PointerKind::Drag, 0.0, 0.0, m)).is_none());
    }

    #[test]
    fn the_dragged_guide_snaps_to_art_ruler_ticks_and_the_grid() {
        let d = guides_doc();
        let mut s = Selection::default();
        s.set_guides([0]);
        let p = paint();
        let drag_to = |c: &ToolContext, x: f64, mods: Mods| {
            let mut g = GuideEdit::default();
            g.press(c, &ev(PointerKind::Down, 300.0, 50.0, Mods::default()));
            let a = g.pointer(c, &ev(PointerKind::Drag, x, 50.0, mods)).unwrap();
            let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
            (300.0 + v["dx"].as_f64().unwrap(), g.overlays(c))
        };
        // Smart Guides: onto the rect's right edge (x = 200), labelled.
        let (x, ov) = drag_to(&cx(&d, &s, &p), 203.0, Mods::default());
        assert_eq!(x, 200.0);
        assert!(matches!(&ov[0], Overlay::Label { .. }));
        let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
        assert_eq!(drag_to(&off, 203.0, Mods::default()).0, 203.0);
        // Shift: the ruler's ticks (labels every 50 pt at 100 %, ticks every 5 pt).
        assert_eq!(drag_to(&off, 223.0, Mods { shift: true, ..Default::default() }).0, 225.0);
        let zoomed = ToolContext { zoom: 5.0, ..cx(&d, &s, &p) };
        assert_eq!(drag_to(&zoomed, 223.4, Mods { shift: true, ..Default::default() }).0, 223.0, "every point at 500 %");
        let grid = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        assert_eq!(drag_to(&grid, 226.0, Mods::default()).0, 225.0, "the grid's 9 pt subdivisions");
    }

    /// #451: a guide dragged out of a ruler lands on the art's corners, side midpoints (its
    /// centre's line) and anchors, and on the artboard's edges and centre, labelled.
    #[test]
    fn a_new_guide_snaps_to_the_art_and_the_artboard() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let m = Mods::default();
        let drag = |vertical: bool, v: f64| {
            let mut g = NewGuide::new(&c, vertical, None);
            let at = if vertical { Point::new(v, 50.0) } else { Point::new(50.0, v) };
            let a = g.pointer(&c, &ev(PointerKind::Drag, at.x, at.y, m), true);
            let Some(Action::Preview(cmd, p)) = a.last() else { panic!("{a:?}") };
            assert_eq!((a[0].clone(), cmd.as_str(), &p["vertical"]), (Action::Begin("New Guide".into()), "guide.add", &json!(vertical)));
            let label = g.overlays(&c).iter().find_map(|o| match o {
                Overlay::Label { text, .. } => Some(text.clone()),
                _ => None,
            });
            (p["pos"].as_f64().unwrap(), label)
        };
        assert_eq!(drag(true, 152.0), (150.0, Some("center".into())), "the line through the top and bottom midpoints");
        assert_eq!(drag(false, 103.0), (100.0, Some("path".into())), "the top edge, through two corners");
        assert_eq!(drag(true, 497.0), (500.0, Some("artboard".into())), "the artboard's right edge");
        assert_eq!(drag(false, 248.0), (250.0, Some("center".into())), "the artboard's centre");
        assert_eq!(drag(true, 320.0), (320.0, None), "nothing within reach");
    }

    /// Preferences › Smart Guides (#394) reach a guide dragged out of a ruler: Snapping
    /// Tolerance sets how far a target pulls it, Anchor/Path Labels and Measurement Labels what
    /// shows by it.
    #[test]
    fn a_new_guide_follows_the_smart_guide_preferences() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let drag = |c: &ToolContext| {
            let mut g = NewGuide::new(c, true, None);
            let a = g.pointer(c, &ev(PointerKind::Drag, 497.0, 50.0, Mods::default()), true);
            let Some(Action::Preview(_, p)) = a.last() else { panic!("{a:?}") };
            (p["pos"].as_f64().unwrap(), g.overlays(c))
        };
        let (at, ov) = drag(&ToolContext { snapping_tolerance: 2.0, ..cx(&d, &s, &p) });
        assert_eq!((at, ov.len()), (497.0, 1), "3 px off is beyond 2: only the readout {ov:?}");
        let (at, ov) = drag(&ToolContext { anchor_path_labels: false, measurement_labels: false, ..cx(&d, &s, &p) });
        assert_eq!((at, ov), (500.0, vec![]), "onto the artboard's edge, nothing said");
    }

    #[test]
    fn a_new_guide_is_made_on_release_over_the_canvas_only() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = ToolContext { smart_guides: false, snap_to_point: false, ..cx(&d, &s, &p) };
        let m = Mods::default();
        let mut g = NewGuide::new(&c, false, Some(0));
        let preview = |pos: f64| Action::Preview("guide.add".into(), json!({"vertical": false, "pos": pos, "artboard": 0}));
        // Off the canvas (still over the ruler): nothing yet.
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, -5.0, m), false), vec![]);
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 30.0, m), true), vec![Action::Begin("New Guide".into()), preview(30.0)]);
        assert!(matches!(g.overlays(&c).last(), Some(Overlay::Measure { text, .. }) if text == "Y: 30.00 pt"));
        // The same place again: no new preview.
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 90.0, 30.0, m), true), vec![]);
        // Back onto the ruler it goes, and comes back over the canvas.
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, -5.0, m), false), vec![Action::Cancel]);
        assert!(g.overlays(&c).is_empty());
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 60.0, m), true), vec![Action::Begin("New Guide".into()), preview(60.0)]);
        // Shift: the ruler's ticks.
        assert_eq!(g.pointer(&c, &ev(PointerKind::Drag, 40.0, 63.0, Mods { shift: true, ..m }), true), vec![preview(65.0)]);
        assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 40.0, 65.0, m), true), vec![Action::Commit]);
        // Released off the canvas: none.
        let mut g = NewGuide::new(&c, true, None);
        g.pointer(&c, &ev(PointerKind::Drag, 30.0, 40.0, m), true);
        assert_eq!(g.pointer(&c, &ev(PointerKind::Up, -5.0, 40.0, m), false), vec![Action::Cancel]);
    }

    /// With Smart Guides off, Snap to Point pulls a dragged guide onto the anchors (within its
    /// distance), not the edges' midpoints.
    #[test]
    fn with_smart_guides_off_snap_to_point_pulls_guides_onto_anchors() {
        let d = guides_doc();
        let mut s = Selection::default();
        s.set_guides([0]);
        let p = paint();
        let off = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
        let moved_to = |c: &ToolContext, x: f64| {
            let mut g = GuideEdit::default();
            g.press(c, &ev(PointerKind::Down, 300.0, 50.0, Mods::default()));
            let a = g.pointer(c, &ev(PointerKind::Drag, x, 50.0, Mods::default())).unwrap();
            let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
            300.0 + v["dx"].as_f64().unwrap()
        };
        assert_eq!(moved_to(&off, 201.5), 200.0);
        assert_eq!(moved_to(&off, 151.0), 151.0, "the centre is no anchor");
        assert_eq!(moved_to(&off, 203.0), 203.0, "beyond the 2 px Snap to Point distance");
        let neither = ToolContext { snap_to_point: false, ..off };
        assert_eq!(moved_to(&neither, 201.5), 201.5);
        let mut g = NewGuide::new(&off, true, None);
        let a = g.pointer(&off, &ev(PointerKind::Drag, 98.5, 20.0, Mods::default()), true);
        assert_eq!(a.last(), Some(&Action::Preview("guide.add".into(), json!({"vertical": true, "pos": 100.0}))));
    }

    /// #451: an artboard guide runs across its artboard only: it is picked there, and pulls a
    /// point with Snap to Point there.
    #[test]
    fn an_artboard_guide_is_picked_and_snapped_to_across_its_artboard_only() {
        let (mut d, _) = doc_with_rect();
        let id = d.next_artboard_id();
        d.artboards.push(vectorcraft_doc::Artboard {
            id,
            name: "B".into(),
            rect: vectorcraft_geom::Rect::new(600.0, 0.0, 900.0, 300.0),
            show_center_mark: false,
            show_cross_hairs: false,
        });
        d.guides = vec![Guide { artboard: Some(id), ..Guide::new(false, 150.0) }];
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        assert_eq!(guide_at(&c, Point::new(700.0, 151.0)), Some(0));
        assert_eq!(guide_at(&c, Point::new(602.0 - 4.0, 151.0)), Some(0), "within the tolerance of its end");
        assert_eq!(guide_at(&c, Point::new(300.0, 151.0)), None, "not across the first artboard");
        let t = Targets::snap_to_point(&ToolContext { smart_guides: false, ..c }, &[]).unwrap();
        assert_eq!(t.snap_point(Point::new(700.0, 151.5), 2.0).0, Point::new(700.0, 150.0));
        assert_eq!(t.snap_point(Point::new(300.0, 151.5), 2.0).0, Point::new(300.0, 151.5));
    }

    #[test]
    fn shift_click_toggles_and_alt_drag_copies() {
        let d = guides_doc();
        let mut s = Selection::default();
        s.set_guides([0, 1]);
        let p = paint();
        let c = cx(&d, &s, &p);
        let (shift, alt) = (Mods { shift: true, ..Default::default() }, Mods { alt: true, ..Default::default() });
        let mut g = GuideEdit::default();
        assert_eq!(g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, shift)), Some(vec![]));
        let deselect = Action::Exec("guide.select".into(), json!({"indexes": [1], "toggle": true}));
        assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 50.0, 400.0, shift)), Some(vec![deselect]));
        g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, alt));
        let a = g.pointer(&c, &ev(PointerKind::Drag, 50.0, 420.0, alt)).unwrap();
        assert_eq!(a[0], Action::Begin("Copy Guide".into()));
        assert_eq!(a[1], Action::Preview("guide.move".into(), json!({"dx": 0.0, "dy": 20.0, "copy": true})));
    }

    #[test]
    fn dropped_on_its_ruler_the_guide_is_deleted() {
        let d = guides_doc();
        let p = paint();
        let mut s = Selection::default();
        s.set_guides([1]);
        // The window shows the document from (0, 0) at 100 %: above y = 0 is the top ruler.
        let screen = ScreenFrame { origin: Point::ZERO, right: Vec2::new(1.0, 0.0), down: Vec2::new(0.0, 1.0), size: (800.0, 600.0) };
        let c = ToolContext { screen: Some(screen), ..cx(&d, &s, &p) };
        let m = Mods::default();
        let gesture = |to: Point, mods: Mods| {
            let mut g = GuideEdit::default();
            g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, m));
            g.pointer(&c, &ev(PointerKind::Drag, to.x, to.y, mods));
            g.pointer(&c, &ev(PointerKind::Up, to.x, to.y, mods)).unwrap()
        };
        assert_eq!(gesture(Point::new(50.0, -6.0), m), vec![Action::Cancel, Action::Exec("guide.remove".into(), json!({"index": 1}))]);
        assert_eq!(gesture(Point::new(50.0, -6.0), Mods { alt: true, ..m }), vec![Action::Cancel], "a copy is just not made");
        // Off the left side: not its ruler.
        assert_eq!(gesture(Point::new(-6.0, 300.0), m), vec![Action::Commit]);
        // Headless (no window): no ruler to drop it on.
        let mut g = GuideEdit::default();
        let c = cx(&d, &s, &p);
        g.press(&c, &ev(PointerKind::Down, 50.0, 400.0, m));
        g.pointer(&c, &ev(PointerKind::Drag, 50.0, -6.0, m));
        assert_eq!(g.pointer(&c, &ev(PointerKind::Up, 50.0, -6.0, m)), Some(vec![Action::Commit]));
    }
}
