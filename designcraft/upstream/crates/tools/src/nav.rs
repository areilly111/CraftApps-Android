//! Hand (H) and Zoom (Z) tools.

use designcraft_geom::Point;
use serde_json::json;

use crate::{Action, Cursor, Mods, PointerEvent, PointerKind, Tool, ToolContext};

/// Hand tool: drag to pan. Alt-press starts Power Zoom: the view zooms out to the spread, the
/// pointer aims a rectangle of the previous view, and releasing zooms back in there.
#[derive(Default)]
pub struct HandTool {
    last: Option<Point>,
    power: bool,
}

impl Tool for HandTool {
    fn id(&self) -> &'static str {
        "hand"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down if ev.mods.alt => {
                self.power = true;
                self.last = Some(ev.pos);
                vec![Action::View(json!({"powerZoom": "start", "at": [ev.pos.x, ev.pos.y]}))]
            }
            PointerKind::Down => {
                self.last = Some(ev.pos);
                vec![]
            }
            PointerKind::Drag if self.power => vec![Action::View(json!({"powerZoom": "move", "at": [ev.pos.x, ev.pos.y]}))],
            PointerKind::Up if self.power => {
                self.power = false;
                self.last = None;
                vec![Action::View(json!({"powerZoom": "end", "at": [ev.pos.x, ev.pos.y]}))]
            }
            PointerKind::Drag => {
                // Canvas positions shift as we pan, so convert the delta to screen pixels once.
                let Some(l) = self.last else { return vec![] };
                let d = (ev.pos - l) * cx.zoom;
                vec![Action::View(json!({"pan": [d.x, d.y]}))]
            }
            PointerKind::Up => {
                self.last = None;
                vec![]
            }
            _ => vec![],
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        if self.last.is_some() { Cursor::HandGrab } else { Cursor::Hand }
    }
    fn busy(&self) -> bool {
        self.last.is_some()
    }
}

/// Zoom tool: click zooms in (Alt: out); dragging left or right zooms out or in continuously
/// around the press point (scrubby zoom).
#[derive(Default)]
pub struct ZoomTool {
    /// Press point (canvas), zoom at the press, whether it has been dragged.
    press: Option<(Point, f64, bool)>,
}

/// Screen pixels of horizontal drag per e-fold of zoom.
const SCRUB_PX: f64 = 150.0;

impl Tool for ZoomTool {
    fn id(&self) -> &'static str {
        "zoom"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.press = Some((ev.pos, cx.zoom, false));
                vec![]
            }
            PointerKind::Drag => {
                let Some((start, z0, _)) = self.press else { return vec![] };
                // The press point stays put on screen, so this is the on-screen distance.
                let dx = (ev.pos.x - start.x) * cx.zoom;
                if dx.abs() < 3.0 && self.press.is_some_and(|p| !p.2) {
                    return vec![];
                }
                self.press = Some((start, z0, true));
                let want = (z0 * (dx / SCRUB_PX).exp()).clamp(0.05, 40.0);
                vec![Action::View(json!({"zoomAt": [start.x, start.y], "factor": want / cx.zoom.max(1e-9)}))]
            }
            PointerKind::Up => {
                let dragged = self.press.take().is_some_and(|p| p.2);
                if dragged {
                    return vec![];
                }
                let f = if ev.mods.alt { 0.5 } else { 2.0 };
                vec![Action::View(json!({"zoomAt": [ev.pos.x, ev.pos.y], "factor": f}))]
            }
            _ => vec![],
        }
    }
    fn busy(&self) -> bool {
        self.press.is_some_and(|p| p.2)
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, m: Mods) -> Cursor {
        if m.alt { Cursor::ZoomOut } else { Cursor::ZoomIn }
    }
}

/// The loaded place cursor: click places at actual size, drag draws the frame, click on an empty
/// frame places into it.
#[derive(Default)]
pub struct PlaceGun {
    start: Option<Point>,
    cur: Point,
}

impl Tool for PlaceGun {
    fn id(&self) -> &'static str {
        "placeGun"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.start = Some(ev.pos);
                self.cur = ev.pos;
                vec![]
            }
            PointerKind::Drag => {
                self.cur = ev.pos;
                vec![]
            }
            PointerKind::Up => {
                let Some(a) = self.start.take() else { return vec![] };
                let Some((sr, sa)) = cx.layout.spread_at(a) else { return vec![] };
                let b = cx.layout.to_spread(sr, ev.pos);
                if (ev.pos - a).hypot() < cx.tol(3.0)
                    && let Some((_, id)) = cx.hit(a)
                    && cx
                        .doc
                        .item(id)
                        .is_some_and(|i| matches!(i.content, designcraft_doc::Content::Unassigned | designcraft_doc::Content::Graphic(_)))
                {
                    return vec![Action::Exec("place.drop".into(), json!({"frame": id.0}))];
                }
                let r = designcraft_geom::Rect::from_points(sa, b);
                vec![Action::Exec(
                    "place.drop".into(),
                    json!({"spread": crate::spread_json(sr), "x": sa.x, "y": sa.y, "rect": [r.x0, r.y0, r.x1, r.y1]}),
                )]
            }
            _ => vec![],
        }
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<crate::Overlay> {
        match self.start {
            Some(a) => vec![crate::Overlay::Marquee(designcraft_geom::Rect::from_points(a, self.cur))],
            None => vec![],
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::LoadedGraphic
    }
}

/// Page tool (Shift+P): click a page to set its own size.
#[derive(Default)]
pub struct PageTool;

impl Tool for PageTool {
    fn id(&self) -> &'static str {
        "page"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Up {
            return vec![];
        }
        let Some((designcraft_doc::SpreadRef::Doc(si), sp)) = cx.layout.spread_at(ev.pos) else { return vec![] };
        let Some(spread) = cx.doc.spreads.get(si) else { return vec![] };
        let Some(pi) = spread.pages.iter().position(|p| p.bounds().contains(sp)) else { return vec![] };
        let pg = &spread.pages[pi];
        let n = cx.doc.first_page_of_spread(si) + pi + 1;
        vec![Action::Dialog("cmd:layout.pageSize".into(), json!({"pages": [n], "width": pg.width, "height": pg.height}))]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Arrow
    }
}

/// Type on a Path tool (Shift+T): click a path to put type on it; typing goes there.
#[derive(Default)]
pub struct PathTypeTool;

impl Tool for PathTypeTool {
    fn id(&self) -> &'static str {
        "typeOnPath"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Up {
            return vec![];
        }
        let Some((_, id)) = cx.hit(ev.pos) else { return vec![] };
        let Some(it) = cx.doc.item(id) else { return vec![] };
        match &it.content {
            designcraft_doc::Content::Unassigned => vec![Action::Exec("type.onPath".into(), json!({"id": id.0})), Action::SwitchTool("type".into())],
            _ => vec![],
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Text
    }
}

/// Gap tool (U): drag the space between objects (or between an object and the page edge); the
/// objects on both sides resize so the gap moves.
#[derive(Default)]
pub struct GapTool {
    /// Spread, its spread → canvas transform and the press point in spread coordinates.
    start: Option<(designcraft_doc::SpreadRef, designcraft_geom::Affine, Point)>,
    active: bool,
}

impl Tool for GapTool {
    fn id(&self) -> &'static str {
        "gap"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                if let Some((sr, p)) = cx.layout.spread_at(ev.pos) {
                    self.start = Some((sr, cx.layout.xf(sr), p));
                }
                self.active = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some((sr, xf, p)) = self.start else { return vec![] };
                let d = (xf.inverse() * ev.pos) - p;
                let mut out = vec![];
                if !self.active {
                    if d.hypot() < cx.tol(2.0) {
                        return vec![];
                    }
                    self.active = true;
                    out.push(Action::Begin("Move Gap".into()));
                }
                out.push(Action::Preview(
                    "gap.move".into(),
                    serde_json::json!({"spread": crate::spread_json(sr), "at": [p.x, p.y], "dx": d.x, "dy": d.y}),
                ));
                out
            }
            PointerKind::Up => {
                self.start = None;
                if std::mem::take(&mut self.active) { vec![Action::Commit] } else { vec![] }
            }
            _ => vec![],
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn busy(&self) -> bool {
        self.active
    }
}

/// Content Collector (B): click objects to put copies on the conveyor. Content Placer: click to
/// place the next collected object there.
pub struct ConveyorTool {
    placer: bool,
}

impl ConveyorTool {
    pub fn new(placer: bool) -> Self {
        Self { placer }
    }
}

impl Tool for ConveyorTool {
    fn id(&self) -> &'static str {
        if self.placer { "contentPlacer" } else { "contentCollector" }
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        if self.placer {
            let Some((sr, p)) = cx.layout.spread_at(ev.pos) else { return vec![] };
            return vec![Action::Exec("conveyor.place".into(), json!({"spread": crate::spread_json(sr), "x": p.x, "y": p.y, "keep": ev.mods.alt}))];
        }
        match cx.hit(ev.pos) {
            Some((_, id)) => vec![Action::Exec("conveyor.collect".into(), json!({"ids": [id.0]}))],
            None => vec![],
        }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
}
