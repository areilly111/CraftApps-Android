//! Line Segment family drag tools: Arc, Spiral, Rectangular Grid and Polar Grid.
//!
//! Drag draws (Shift = equal axes / square, Alt = from the centre for arc and grids, Space held
//! moves the shape being drawn); a click without dragging asks the UI for the options dialog. The
//! start point and the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering too.
//! While dragging, ↑/↓ change the spiral's segments, the grid rows or the concentric dividers; ←/→
//! change grid columns / radial dividers.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use crate::guides::{DrawSnap, Leave, square};
use crate::shape::drag_rect;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub struct FamilyTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub closed: bool,
    pub decay: f64,
    pub segments: u32,
    pub clockwise: bool,
    /// Rows / concentric dividers.
    pub rows: u32,
    /// Columns / radial dividers.
    pub columns: u32,
    snap: DrawSnap,
}

impl FamilyTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "spiral" => "spiral",
            "rectangularGrid" => "rectangularGrid",
            "polarGrid" => "polarGrid",
            _ => "arc",
        };
        Self {
            id,
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            closed: false,
            decay: 80.0,
            segments: 10,
            clockwise: true,
            rows: 5,
            columns: 5,
            snap: DrawSnap::default(),
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "spiral" => "Spiral",
            "rectangularGrid" => "Rectangular Grid",
            "polarGrid" => "Polar Grid",
            _ => "Arc",
        }
    }

    /// How the dragged point keeps to the start: a spiral's radius goes anywhere, the arc's and
    /// the grids' corner keeps to a diagonal with Shift.
    fn leave(&self, start: Point, m: Mods) -> Option<Leave> {
        (self.id != "spiral").then(|| Leave::diagonal(start, m.shift))
    }

    /// The command for a drag from `start` to `p`.
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        match self.id {
            "spiral" => {
                let r = p.distance(start).max(0.01);
                (
                    "shape.spiral".into(),
                    json!({"cx": start.x, "cy": start.y, "radius": r, "decay": self.decay, "segments": self.segments, "clockwise": self.clockwise}),
                )
            }
            "rectangularGrid" => {
                let r = drag_rect(start, p, m);
                (
                    "shape.rectangularGrid".into(),
                    json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(), "rows": self.rows, "columns": self.columns}),
                )
            }
            "polarGrid" => {
                let r = drag_rect(start, p, m);
                (
                    "shape.polarGrid".into(),
                    json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(), "concentric": self.rows, "radial": self.columns}),
                )
            }
            _ => {
                let d = if m.shift { square(p - start) } else { p - start };
                let (a, b) = if m.alt { (start - d, start + d) } else { (start, start + d) };
                ("shape.arc".into(), json!({"x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y, "closed": self.closed}))
            }
        }
    }
}

impl Tool for FamilyTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.snap.hover(cx, ev.pos, &[], None);
                vec![]
            }
            PointerKind::Down => {
                let p = self.snap.press(cx, ev.pos, &[], None);
                self.start = Some(p);
                self.last = p;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut s) = self.start else { return vec![] };
                let pos = self.snap.drag(cx, ev.pos, self.leave(s, ev.mods).as_ref());
                let ev = &PointerEvent { pos, ..*ev };
                crate::shape::space_moves(&mut s, &mut self.last, ev, self.began);
                self.start = Some(s);
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                let (c, v) = self.command(s, ev.pos, ev.mods);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                if std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![Action::Dialog(self.id.into(), json!({"x": s.x, "y": s.y}))] }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.id, key) {
            ("spiral", ToolKey::Up) => {
                self.segments = (self.segments + 1).min(1000);
                true
            }
            ("spiral", ToolKey::Down) => {
                self.segments = self.segments.saturating_sub(1).max(2);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Up) => {
                self.rows = (self.rows + 1).min(999);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Down) => {
                self.rows = self.rows.saturating_sub(1);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Right) => {
                self.columns = (self.columns + 1).min(999);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Left) => {
                self.columns = self.columns.saturating_sub(1);
                true
            }
            _ => false,
        };
        if changed {
            let (c, v) = self.command(s, self.last, self.mods);
            vec![Action::Preview(c, v)]
        } else if key == ToolKey::Escape {
            self.start = None;
            self.began = false;
            self.snap.clear();
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            // The size as drawn (from the centre with Alt, square with Shift).
            let r = drag_rect(s, self.last, self.mods);
            let (w, h) = if self.id == "spiral" { ((self.last - s).x.abs(), (self.last - s).y.abs()) } else { (r.width(), r.height()) };
            o.push(Overlay::Measure { p: self.last, text: cx.size_label(w, h) });
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        match self.id {
            "spiral" => json!({"decay": self.decay, "segments": self.segments, "clockwise": self.clockwise}),
            "rectangularGrid" => json!({"rows": self.rows, "columns": self.columns}),
            "polarGrid" => json!({"concentric": self.rows, "radial": self.columns}),
            _ => json!({"closed": self.closed}),
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "closed" => self.closed = v.as_bool().unwrap_or(self.closed),
            "decay" => self.decay = v.as_f64().unwrap_or(self.decay).clamp(5.0, 150.0),
            "segments" => self.segments = v.as_u64().unwrap_or(10).clamp(2, 1000) as u32,
            "clockwise" => self.clockwise = v.as_bool().unwrap_or(self.clockwise),
            "rows" | "concentric" => self.rows = v.as_u64().unwrap_or(5).min(999) as u32,
            "columns" | "radial" => self.columns = v.as_u64().unwrap_or(5).min(999) as u32,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn grid_drag_and_arrows() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = FamilyTool::new("rectangularGrid");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0));
        assert_eq!(a[0], Action::Begin("Rectangular Grid".into()));
        assert_eq!(
            a[1],
            Action::Preview("shape.rectangularGrid".into(), json!({"x": 10.0, "y": 10.0, "width": 50.0, "height": 30.0, "rows": 5, "columns": 5}))
        );
        let a = t.key(&cx, ToolKey::Right, Mods::default());
        assert!(matches!(&a[0], Action::Preview(_, v) if v["columns"] == 6));
        // Space held moves the grid at its size; let go, it grows again from there.
        let space = Mods { space: true, ..Mods::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 50.0).with_mods(space));
        assert!(matches!(&a[..], [Action::Preview(_, v)] if v["x"] == 30.0 && v["y"] == 20.0 && v["width"] == 50.0 && v["height"] == 30.0), "{a:?}");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 60.0));
        assert!(matches!(&a[..], [Action::Preview(_, v)] if v["x"] == 30.0 && v["width"] == 60.0 && v["height"] == 40.0), "{a:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 90.0, 60.0)), vec![Action::Commit]);
    }

    #[test]
    fn click_opens_dialog_and_arc_shift() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = FamilyTool::new("spiral");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)),
            vec![Action::Dialog("spiral".into(), json!({"x": 10.0, "y": 10.0}))]
        );
        let t = FamilyTool::new("arc");
        let (c, v) = t.command(Point::new(0.0, 0.0), Point::new(10.0, 4.0), Mods { shift: true, ..Default::default() });
        assert_eq!(c, "shape.arc");
        assert_eq!(v, json!({"x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0, "closed": false}));
    }

    /// The start and the dragged corner snap to Smart Guides (#506), Shift sliding the corner of
    /// a square grid along its diagonal into line.
    #[test]
    fn corners_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FamilyTool::new("rectangularGrid");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 202.0, 99.0));
        let shift = Mods { shift: true, ..Mods::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 255.0, 147.0).with_mods(shift));
        assert!(
            matches!(&a[..], [Action::Begin(_), Action::Preview(_, v)] if v["x"] == 200.0 && v["y"] == 100.0 && v["width"] == 50.0 && v["height"] == 50.0),
            "{a:?}"
        );
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align")));
    }
}
