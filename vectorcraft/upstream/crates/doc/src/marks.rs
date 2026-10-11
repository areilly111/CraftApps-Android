//! Printer's marks drawn as art: the trim marks of Object → Create Trim Marks and Effect → Crop
//! Marks, stroked in the Registration colour so that they print on every plate, and the printer's
//! marks around a printed page ([`PrinterMarks`]: PDF export's Marks and Bleeds, and print).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Point, Rect, shapes};

use crate::{Appearance, CharStyle, Node, NodeId, NodeKind, TextObject};

/// Trim mark style (the preference `japaneseCropMarks` picks Japanese).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkStyle {
    /// One line per edge at each corner, set off from the trim box.
    #[default]
    Roman,
    /// Double lines at each corner (the trim and the bleed edges) and centre marks on each side.
    Japanese,
}

impl MarkStyle {
    pub fn id(self) -> &'static str {
        match self {
            MarkStyle::Roman => "roman",
            MarkStyle::Japanese => "japanese",
        }
    }

    /// Parse an id, ignoring case.
    pub fn parse(s: &str) -> Option<Self> {
        [MarkStyle::Roman, MarkStyle::Japanese].into_iter().find(|m| m.id().eq_ignore_ascii_case(s))
    }
}

/// Trim marks around a rectangle (the trim box).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimMarks {
    pub style: MarkStyle,
    /// Roman: the gap between the trim box and the marks.
    pub offset: f64,
    /// How long each mark is.
    pub length: f64,
    /// Japanese: the bleed, between the trim lines and the outer lines (3 mm).
    pub bleed: f64,
    /// Stroke weight.
    pub weight: f64,
}

impl Default for TrimMarks {
    fn default() -> Self {
        Self { style: MarkStyle::Roman, offset: 9.0, length: 18.0, bleed: 3.0 * 72.0 / 25.4, weight: 0.3 }
    }
}

impl TrimMarks {
    /// The default marks of `style`.
    pub fn of(style: MarkStyle) -> Self {
        Self { style, ..Default::default() }
    }

    /// How far the marks reach outside the trim box.
    pub fn reach(&self) -> f64 {
        let gap = match self.style {
            MarkStyle::Roman => self.offset,
            MarkStyle::Japanese => self.bleed,
        };
        gap + self.length + self.weight
    }

    /// The marks' lines around `r`.
    pub fn lines(&self, r: Rect) -> Vec<(Point, Point)> {
        let (l, mut out) = (self.length, vec![]);
        // Each corner with its outward directions.
        let corners = [(r.x0, r.y0, -1.0, -1.0), (r.x1, r.y0, 1.0, -1.0), (r.x1, r.y1, 1.0, 1.0), (r.x0, r.y1, -1.0, 1.0)];
        match self.style {
            MarkStyle::Roman => {
                let o = self.offset;
                for (x, y, sx, sy) in corners {
                    out.push((Point::new(x + sx * o, y), Point::new(x + sx * (o + l), y)));
                    out.push((Point::new(x, y + sy * o), Point::new(x, y + sy * (o + l))));
                }
            }
            MarkStyle::Japanese => {
                let b = self.bleed;
                for (x, y, sx, sy) in corners {
                    // The trim lines, outside the bleed, and the bleed lines meeting at its corner.
                    out.push((Point::new(x + sx * b, y), Point::new(x + sx * (b + l), y)));
                    out.push((Point::new(x, y + sy * b), Point::new(x, y + sy * (b + l))));
                    out.push((Point::new(x, y + sy * b), Point::new(x + sx * (b + l), y + sy * b)));
                    out.push((Point::new(x + sx * b, y), Point::new(x + sx * b, y + sy * (b + l))));
                }
                // A cross outside the bleed at the middle of each side.
                let c = r.center();
                for (p, dx, dy) in [
                    (Point::new(c.x, r.y0), 0.0, -1.0),
                    (Point::new(r.x1, c.y), 1.0, 0.0),
                    (Point::new(c.x, r.y1), 0.0, 1.0),
                    (Point::new(r.x0, c.y), -1.0, 0.0),
                ] {
                    let at = |d: f64| Point::new(p.x + dx * d, p.y + dy * d);
                    out.push((at(b), at(b + l)));
                    let m = at(b + l / 2.0);
                    out.push((Point::new(m.x - dy * l / 2.0, m.y - dx * l / 2.0), Point::new(m.x + dy * l / 2.0, m.y + dx * l / 2.0)));
                }
            }
        }
        out
    }

    /// The marks around `r` as a group named "Trim Marks" of lines stroked in Registration, with
    /// ids from `alloc` (the group's last).
    pub fn group(&self, r: Rect, alloc: &mut dyn FnMut() -> NodeId) -> Node {
        let ap = Appearance::basic(Paint::None, Paint::registration(), self.weight);
        let children = self.lines(r).into_iter().map(|(a, b)| Arc::new(Node::path(alloc(), shapes::line(a, b), ap.clone()))).collect();
        let mut g = Node::group(alloc(), children);
        g.name = Some("Trim Marks".into());
        g
    }
}

/// `r` grown by `[top, bottom, left, right]` (a bleed, or how far marks reach).
pub fn outset(r: Rect, [top, bottom, left, right]: [f64; 4]) -> Rect {
    Rect::new(r.x0 - left, r.y0 - top, r.x1 + right, r.y1 + bottom)
}

/// The printer's marks around a printed page's trim box (its artboard): trim marks, a
/// registration target at the middle of each side, colour bars along the top and the page
/// information line below the bottom marks. They sit in a band [`PrinterMarks::LENGTH`] deep that
/// starts at the offset from the trim box (outside the bleed), and are drawn in Registration, the
/// colour bars aside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrinterMarks {
    pub trim: bool,
    pub registration: bool,
    pub color_bars: bool,
    pub page_info: bool,
    pub style: MarkStyle,
    /// Stroke weight of the trim marks and registration targets.
    pub weight: f64,
    /// Roman: the gap between the trim box and the marks (at least the bleed).
    pub offset: f64,
}

/// Where the printer's marks of one page go ([`PrinterMarks::layout`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarksLayout {
    /// Lines stroked in Registration: the trim marks and the targets' cross hairs.
    pub lines: Vec<(Point, Point)>,
    /// Registration target rings (centre, radius), stroked in Registration.
    pub rings: Vec<(Point, f64)>,
    /// The colour bars: each patch and its paint.
    pub patches: Vec<(Rect, Paint)>,
    /// The page information line's baseline start.
    pub info: Option<Point>,
}

impl PrinterMarks {
    /// How deep the band of marks is (the trim marks' length).
    pub const LENGTH: f64 = 18.0;
    /// The side of a colour bar patch.
    pub const PATCH: f64 = 10.0;
    /// The page information's type size.
    pub const INFO_SIZE: f64 = 7.0;

    /// Any mark is on.
    pub fn any(&self) -> bool {
        self.trim || self.registration || self.color_bars || self.page_info
    }

    /// The trim marks for a page with `bleed` (`[top, bottom, left, right]`): Roman ones outside
    /// the bleed, Japanese ones with their outer lines on the bleed (3 mm without one).
    pub fn trim_marks(&self, bleed: [f64; 4]) -> TrimMarks {
        let most = bleed.iter().copied().fold(0.0, f64::max);
        let d = TrimMarks::of(self.style);
        TrimMarks { offset: self.offset.max(most), length: Self::LENGTH, bleed: if most > 0.0 { most } else { d.bleed }, weight: self.weight, ..d }
    }

    /// The distance from the trim box to the band of marks.
    fn gap(&self, bleed: [f64; 4]) -> f64 {
        let t = self.trim_marks(bleed);
        match self.style {
            MarkStyle::Roman => t.offset,
            MarkStyle::Japanese => t.bleed,
        }
    }

    /// How far the marks reach outside the trim box, `[top, bottom, left, right]` (0 without
    /// marks): the band on every side, and the page information below it.
    pub fn reach(&self, bleed: [f64; 4]) -> [f64; 4] {
        if !self.any() {
            return [0.0; 4];
        }
        let band = self.gap(bleed) + Self::LENGTH + self.weight;
        let bottom = if self.page_info { band + Self::INFO_SIZE * 1.6 } else { band };
        [band, bottom, band, band]
    }

    /// Where the marks around trim box `trim` go, for a page with `bleed` and the spot inks
    /// `spots` (their colour bar patches follow the process ones).
    pub fn layout(&self, trim: Rect, bleed: [f64; 4], spots: &[Paint]) -> MarksLayout {
        let mut out = MarksLayout::default();
        if !self.any() {
            return out;
        }
        let (gap, l) = (self.gap(bleed), Self::LENGTH);
        if self.trim {
            out.lines = self.trim_marks(bleed).lines(trim);
        }
        let c = trim.center();
        // The middle of the band.
        let mid = gap + l / 2.0;
        if self.registration {
            let sides = [
                (Point::new(c.x, trim.y0), 0.0, -1.0),
                (Point::new(trim.x1, c.y), 1.0, 0.0),
                (Point::new(c.x, trim.y1), 0.0, 1.0),
                (Point::new(trim.x0, c.y), -1.0, 0.0),
            ];
            for (p, dx, dy) in sides {
                let at = Point::new(p.x + dx * mid, p.y + dy * mid);
                out.rings.push((at, l / 3.0));
                out.rings.push((at, l / 6.0));
                // Japanese trim marks draw this cross already.
                if !(self.trim && self.style == MarkStyle::Japanese) {
                    out.lines.push((Point::new(at.x - l / 2.0, at.y), Point::new(at.x + l / 2.0, at.y)));
                    out.lines.push((Point::new(at.x, at.y - l / 2.0), Point::new(at.x, at.y + l / 2.0)));
                }
            }
        }
        if self.color_bars {
            let s = Self::PATCH;
            let y = trim.y0 - mid - s / 2.0;
            let cmyk = |c, m, y, k| Paint::solid(Color::cmyk(c, m, y, k));
            // Solid inks, their overprints and the spot inks left of the top target; black tints
            // right of it. Patches that don't fit are left out.
            let solids = [
                cmyk(1.0, 0.0, 0.0, 0.0),
                cmyk(0.0, 1.0, 0.0, 0.0),
                cmyk(0.0, 0.0, 1.0, 0.0),
                cmyk(0.0, 0.0, 0.0, 1.0),
                cmyk(1.0, 1.0, 0.0, 0.0),
                cmyk(1.0, 0.0, 1.0, 0.0),
                cmyk(0.0, 1.0, 1.0, 0.0),
                cmyk(1.0, 1.0, 1.0, 0.0),
            ];
            let tints: Vec<Paint> = (1..=10).map(|i| cmyk(0.0, 0.0, 0.0, i as f32 / 10.0)).collect();
            let clear = l / 2.0 + 2.0;
            let rows = [(solids.iter().chain(spots).cloned().collect(), trim.x0 + s / 2.0, c.x - clear), (tints, c.x + clear, trim.x1 - s / 2.0)];
            for (paints, from, to) in rows {
                let fits = ((to - from) / s).floor().max(0.0) as usize;
                for (i, paint) in paints.into_iter().take(fits).enumerate() {
                    let x = from + i as f64 * s;
                    out.patches.push((Rect::new(x, y, x + s, y + s), paint));
                }
            }
        }
        if self.page_info {
            out.info = Some(Point::new(trim.x0, trim.y1 + gap + l + self.weight + Self::INFO_SIZE * 1.1));
        }
        out
    }

    /// The marks around `trim` as a group named "Printer's Marks", with ids from `alloc` (the
    /// group's last): the colour bar patches, then Registration lines and rings, then `info` as
    /// point type in Registration.
    pub fn art(&self, trim: Rect, bleed: [f64; 4], spots: &[Paint], info: &str, alloc: &mut dyn FnMut() -> NodeId) -> Node {
        let m = self.layout(trim, bleed, spots);
        let reg = Appearance::basic(Paint::None, Paint::registration(), self.weight);
        let shape = |id, path, ap: &Appearance| Arc::new(Node::path(id, path, ap.clone()));
        let mut children: Vec<Arc<Node>> =
            m.patches.into_iter().map(|(r, p)| shape(alloc(), shapes::rectangle(r), &Appearance::basic(p, Paint::None, 0.0))).collect();
        children.extend(m.lines.into_iter().map(|(a, b)| shape(alloc(), shapes::line(a, b), &reg)));
        children.extend(m.rings.into_iter().map(|(c, r)| shape(alloc(), shapes::ellipse(Rect::from_center_size(c, (2.0 * r, 2.0 * r))), &reg)));
        if let Some(at) = m.info.filter(|_| !info.is_empty()) {
            let style = CharStyle { size: Self::INFO_SIZE, fill: Paint::registration(), ..Default::default() };
            children.push(Arc::new(Node::new(alloc(), NodeKind::Text(Box::new(TextObject::point(at, info, style))))));
        }
        let mut g = Node::group(alloc(), children);
        g.name = Some("Printer's Marks".into());
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roman_marks_sit_off_the_corners_and_japanese_ones_are_doubled() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let roman = TrimMarks::default().lines(r);
        assert_eq!(roman.len(), 8);
        assert!(roman.iter().all(|(a, b)| !r.contains(*a) && !r.contains(*b)));
        assert_eq!(roman[0], (Point::new(-9.0, 0.0), Point::new(-27.0, 0.0)));
        let jp = TrimMarks::of(MarkStyle::Japanese);
        let lines = jp.lines(r);
        assert_eq!(lines.len(), 24, "four lines per corner and a cross per side");
        // Two horizontal lines at the top-left corner: the trim line and the bleed line.
        let tl: Vec<f64> = lines[..4].iter().filter(|(a, b)| a.y == b.y).map(|(a, _)| a.y).collect();
        assert_eq!(tl.len(), 2);
        assert!((tl[0] - 0.0).abs() < 1e-9 && (tl[1] + jp.bleed).abs() < 1e-9, "{tl:?}");
        let mut n = 0;
        let g = jp.group(r, &mut || {
            n += 1;
            NodeId(n)
        });
        assert_eq!((g.children().unwrap().len(), g.id), (24, NodeId(25)));
        assert!(g.children().unwrap()[0].appearance.stroke().unwrap().paint.is_registration());
        assert_eq!(MarkStyle::parse("Japanese"), Some(MarkStyle::Japanese));
    }

    fn all(style: MarkStyle) -> PrinterMarks {
        PrinterMarks { trim: true, registration: true, color_bars: true, page_info: true, style, weight: 0.25, offset: 6.0 }
    }

    #[test]
    fn printer_marks_sit_in_a_band_outside_the_trim_box() {
        let trim = Rect::new(0.0, 0.0, 200.0, 100.0);
        let m = all(MarkStyle::Roman);
        let lay = m.layout(trim, [0.0; 4], &[]);
        // Golden positions: the band starts at the 6 pt offset and is 18 pt deep.
        assert_eq!(lay.lines[0], (Point::new(-6.0, 0.0), Point::new(-24.0, 0.0)), "top-left trim mark");
        assert_eq!(lay.rings[..2], [(Point::new(100.0, -15.0), 6.0), (Point::new(100.0, -15.0), 3.0)], "top target");
        assert_eq!(lay.lines[8..10], [(Point::new(91.0, -15.0), Point::new(109.0, -15.0)), (Point::new(100.0, -24.0), Point::new(100.0, -6.0))]);
        assert_eq!(lay.patches[0], (Rect::new(5.0, -20.0, 15.0, -10.0), Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0))), "cyan first");
        assert_eq!(lay.patches[8].0, Rect::new(111.0, -20.0, 121.0, -10.0), "black tints right of the target");
        assert_eq!(lay.patches.len(), 8 + 8, "as many patches as fit each half");
        assert_eq!(lay.info, Some(Point::new(0.0, 100.0 + 24.25 + 7.7)));
        let reach = m.reach([0.0; 4]);
        assert_eq!(reach, [24.25, 24.25 + 11.2, 24.25, 24.25]);
        // Everything lies outside the trim box and inside the sheet the reach makes.
        let sheet = outset(trim, reach);
        let points = lay.lines.iter().flat_map(|(a, b)| [*a, *b]).chain(lay.rings.iter().map(|(c, _)| *c));
        for p in points.chain(lay.patches.iter().flat_map(|(r, _)| [r.origin(), Point::new(r.x1, r.y1)])) {
            assert!(sheet.contains(p) && !trim.inflate(-1e-9, -1e-9).contains(p), "{p:?}");
        }
        // A bleed pushes the marks out; no marks reach nothing.
        assert_eq!(m.layout(trim, [9.0, 0.0, 0.0, 0.0], &[]).lines[0], (Point::new(-9.0, 0.0), Point::new(-27.0, 0.0)));
        assert_eq!(PrinterMarks { trim: false, registration: false, color_bars: false, page_info: false, ..m }.reach([9.0; 4]), [0.0; 4]);
    }

    #[test]
    fn japanese_printer_marks_share_the_centre_crosses_and_draw_as_art() {
        let trim = Rect::new(0.0, 0.0, 200.0, 100.0);
        let m = all(MarkStyle::Japanese);
        let lay = m.layout(trim, [0.0; 4], &[]);
        assert_eq!(lay.lines.len(), 24, "the targets add rings to the trim marks' centre crosses");
        let b = TrimMarks::default().bleed;
        assert_eq!(lay.rings[0].0, Point::new(100.0, -(b + 9.0)));
        let spot = Paint::Solid { color: Color::cmyk(0.0, 0.5, 1.0, 0.0), swatch: Some("Ink".into()), tint: 1.0 };
        let wide = Rect::new(0.0, 0.0, 400.0, 100.0);
        assert_eq!(m.layout(wide, [0.0; 4], std::slice::from_ref(&spot)).patches[8].1, spot, "spot inks follow the process patches");
        let mut n = 0;
        let g = m.art(trim, [0.0; 4], &[], "Page 1", &mut || {
            n += 1;
            NodeId(n)
        });
        let kids = g.children().unwrap();
        let NodeKind::Text(t) = &kids.last().unwrap().kind else { panic!("the page information is type") };
        assert_eq!((t.plain_text().as_str(), t.first_style().fill.is_registration()), ("Page 1", true));
        let strokes: Vec<_> = kids.iter().filter_map(|k| k.appearance.stroke()).filter(|s| !s.paint.is_none()).collect();
        assert!(strokes.len() == 24 + 8 && strokes.iter().all(|s| s.paint.is_registration()), "lines and rings in Registration");
    }
}
