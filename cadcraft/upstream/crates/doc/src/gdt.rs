//! Geometric tolerancing (GD&T): the TOLERANCE entity (feature control frames) and its layout.
//!
//! The frame is stored as the DXF Reference's "visual representation string" (TOLERANCE group 1):
//! frame lines separated by `^J`, compartments separated by `%%v`, and the geometric symbols as
//! characters of the `gdt` font inside `{\Fgdt;…}` runs. CADCraft never uses that font: every
//! symbol is drawn by [`symbol_prims`] from our own geometry (standard ISO/ASME symbol shapes).

use cadcraft_geom::{Bounds2, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Caps for hostile frame strings (files, command parameters).
pub const MAX_FRAME_CHARS: usize = 4096;
pub const MAX_FRAME_ROWS: usize = 8;
pub const MAX_FRAME_CELLS: usize = 12;

/// A TOLERANCE entity: a feature control frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tolerance {
    /// Insertion point: the middle of the left edge of the first frame line.
    pub insert: Vec3,
    /// The frame's X-axis direction (DXF group 11).
    #[serde(default = "x_axis")]
    pub dir: Vec3,
    /// Dimension style: text height (DIMTXT × DIMSCALE), gap, colours, text style.
    #[serde(default = "standard")]
    pub style: String,
    /// The frame contents (DXF group 1).
    pub text: String,
}

fn x_axis() -> Vec3 {
    Vec3::new(1.0, 0.0, 0.0)
}
fn standard() -> String {
    "Standard".into()
}

impl Tolerance {
    /// The frame's rotation (radians from +X); 0 when the direction is unusable.
    pub fn rotation(&self) -> f64 {
        let d = self.dir.xy();
        if d.is_finite() && d.len() > 1e-12 { d.angle() } else { 0.0 }
    }
    /// Text height and padding from the dimension style: DIMTXT and DIMGAP times the style's
    /// DIMSCALE (the drawing's when 0), or the text style's fixed height.
    pub fn text_size(&self, d: &crate::Drawing) -> (f64, f64) {
        let st = d.dim_style(&self.style).cloned().unwrap_or_default();
        let k = st.effective_scale(d.header.f64("DIMSCALE", 1.0));
        let fixed = d.text_style(&st.text_style).map(|t| t.height).filter(|h| h.is_finite() && *h > 0.0);
        let h = fixed.unwrap_or(st.text_height * k);
        (if h.is_finite() && h > 0.0 { h } else { 0.18 }, st.text_gap * k)
    }
    /// The frame laid out with `measure(text, height)` as the width of plain text.
    pub fn layout(&self, d: &crate::Drawing, measure: &dyn Fn(&str, f64) -> f64) -> GdtLayout {
        let (h, gap) = self.text_size(d);
        layout_frame(&self.text, self.insert.xy(), self.rotation(), h, gap, measure)
    }
}

/// A geometric characteristic, material condition or zone symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GdtSymbol {
    Position,
    Concentricity,
    Symmetry,
    Parallelism,
    Perpendicularity,
    Angularity,
    Cylindricity,
    Flatness,
    Circularity,
    Straightness,
    SurfaceProfile,
    LineProfile,
    CircularRunout,
    TotalRunout,
    Diameter,
    /// Maximum material condition (circled M).
    Mmc,
    /// Least material condition (circled L).
    Lmc,
    /// Regardless of feature size (circled S).
    Rfs,
    /// Projected tolerance zone (circled P).
    Projected,
}

/// Every symbol with its `gdt` font character and its name.
const SYMBOLS: [(GdtSymbol, char, &str); 19] = [
    (GdtSymbol::Position, 'j', "position"),
    (GdtSymbol::Concentricity, 'r', "concentricity"),
    (GdtSymbol::Symmetry, 'i', "symmetry"),
    (GdtSymbol::Parallelism, 'f', "parallelism"),
    (GdtSymbol::Perpendicularity, 'b', "perpendicularity"),
    (GdtSymbol::Angularity, 'a', "angularity"),
    (GdtSymbol::Cylindricity, 'g', "cylindricity"),
    (GdtSymbol::Flatness, 'c', "flatness"),
    (GdtSymbol::Circularity, 'e', "circularity"),
    (GdtSymbol::Straightness, 'u', "straightness"),
    (GdtSymbol::SurfaceProfile, 'd', "surfaceprofile"),
    (GdtSymbol::LineProfile, 'k', "lineprofile"),
    (GdtSymbol::CircularRunout, 'h', "circularrunout"),
    (GdtSymbol::TotalRunout, 't', "totalrunout"),
    (GdtSymbol::Diameter, 'n', "diameter"),
    (GdtSymbol::Mmc, 'm', "mmc"),
    (GdtSymbol::Lmc, 'l', "lmc"),
    (GdtSymbol::Rfs, 's', "rfs"),
    (GdtSymbol::Projected, 'p', "projected"),
];

impl GdtSymbol {
    /// The symbol's character in the frame string's `{\Fgdt;…}` runs.
    pub fn code(self) -> char {
        SYMBOLS.iter().find(|(s, _, _)| *s == self).map(|(_, c, _)| *c).unwrap_or('j')
    }
    pub fn from_code(c: char) -> Option<GdtSymbol> {
        SYMBOLS.iter().find(|(_, k, _)| *k == c).map(|(s, _, _)| *s)
    }
    /// Lower-case name ("position", "mmc", …).
    pub fn name(self) -> &'static str {
        SYMBOLS.iter().find(|(s, _, _)| *s == self).map(|(_, _, n)| *n).unwrap_or("position")
    }
    /// A symbol by name (case, spaces, dashes and underscores ignored) or by a few common
    /// alternatives ("true position", "runout", "profile", "M", "L", "S").
    pub fn parse(s: &str) -> Option<GdtSymbol> {
        let k: String = s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect();
        let alias = match k.as_str() {
            "trueposition" | "pos" => Some(GdtSymbol::Position),
            "coaxiality" => Some(GdtSymbol::Concentricity),
            "roundness" => Some(GdtSymbol::Circularity),
            "profile" | "surface" | "profileofasurface" => Some(GdtSymbol::SurfaceProfile),
            "line" | "profileofaline" => Some(GdtSymbol::LineProfile),
            "runout" => Some(GdtSymbol::CircularRunout),
            "total" => Some(GdtSymbol::TotalRunout),
            "m" => Some(GdtSymbol::Mmc),
            "l" => Some(GdtSymbol::Lmc),
            "s" => Some(GdtSymbol::Rfs),
            "dia" | "ø" | "⌀" | "%%c" => Some(GdtSymbol::Diameter),
            _ => None,
        };
        alias.or_else(|| SYMBOLS.iter().find(|(_, _, n)| *n == k).map(|(s, _, _)| *s))
    }
    /// A geometric characteristic (the first compartment of a frame line).
    pub fn is_characteristic(self) -> bool {
        !matches!(self, GdtSymbol::Diameter | GdtSymbol::Mmc | GdtSymbol::Lmc | GdtSymbol::Rfs | GdtSymbol::Projected)
    }
    /// A material condition modifier.
    pub fn is_material(self) -> bool {
        matches!(self, GdtSymbol::Mmc | GdtSymbol::Lmc | GdtSymbol::Rfs)
    }
}

/// A piece of a compartment: plain text or a symbol.
#[derive(Clone, Debug, PartialEq)]
pub enum GdtRun {
    Text(String),
    Symbol(GdtSymbol),
}

/// One compartment's runs.
pub type Compartment = Vec<GdtRun>;

/// Parse a frame string into lines of compartments. Unknown formatting codes are skipped, an
/// unterminated code ends the string, and the sizes are capped ([`MAX_FRAME_CHARS`],
/// [`MAX_FRAME_ROWS`], [`MAX_FRAME_CELLS`]).
pub fn parse_frame(text: &str) -> Vec<Vec<Compartment>> {
    let cs: Vec<char> = text.chars().take(MAX_FRAME_CHARS).collect();
    let at = |i: usize| cs.get(i).copied();
    let mut rows: Vec<Vec<Compartment>> = vec![vec![Vec::new()]];
    // Font stack: whether the `gdt` font is current, per brace level.
    let mut gdt = false;
    let mut stack: Vec<bool> = Vec::new();
    let push = |rows: &mut Vec<Vec<Compartment>>, run: GdtRun| {
        let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) else { return };
        match (cell.last_mut(), run) {
            (Some(GdtRun::Text(t)), GdtRun::Text(s)) => t.push_str(&s),
            (_, run) => cell.push(run),
        }
    };
    let mut i = 0;
    while let Some(c) = at(i) {
        let next = at(i + 1);
        let lower = |k: usize| at(k).map(|c| c.to_ascii_lowercase());
        // Line break: `^J`, `\P` or a raw newline.
        if (c == '^' && matches!(next, Some('J' | 'j'))) || (c == '\\' && next == Some('P')) || c == '\n' {
            if rows.len() >= MAX_FRAME_ROWS {
                break;
            }
            rows.push(vec![Vec::new()]);
            i += if c == '\n' { 1 } else { 2 };
            continue;
        }
        if c == '\r' {
            i += 1;
            continue;
        }
        if c == '%' && next == Some('%') {
            match lower(i + 2) {
                Some('v') => {
                    if let Some(r) = rows.last_mut()
                        && r.len() < MAX_FRAME_CELLS
                    {
                        r.push(Vec::new());
                    }
                }
                Some('c') => push(&mut rows, GdtRun::Symbol(GdtSymbol::Diameter)),
                Some('d') => push(&mut rows, GdtRun::Text("°".into())),
                Some('p') => push(&mut rows, GdtRun::Text("±".into())),
                Some('%') => push(&mut rows, GdtRun::Text("%".into())),
                Some(o) => push(&mut rows, GdtRun::Text(format!("%%{o}"))),
                None => break,
            }
            i += 3;
            continue;
        }
        match c {
            '{' => {
                if stack.len() < 64 {
                    stack.push(gdt);
                }
                i += 1;
            }
            '}' => {
                gdt = stack.pop().unwrap_or(false);
                i += 1;
            }
            '\\' => match next {
                Some('\\' | '{' | '}') => {
                    if let Some(n) = next {
                        push(&mut rows, GdtRun::Text(n.to_string()));
                    }
                    i += 2;
                }
                // Font switch `\Fname;` / `\fname|b0|i0;`: the gdt font draws symbols.
                Some('F' | 'f') => {
                    let Some(end) = (i + 2..cs.len()).find(|k| at(*k) == Some(';')) else { break };
                    let name: String = cs.get(i + 2..end).unwrap_or(&[]).iter().take_while(|c| **c != '|').collect();
                    gdt = name.trim().to_ascii_lowercase().starts_with("gdt");
                    i = end + 1;
                }
                // Codes with a value up to `;` (height, colour, tracking, width, oblique, alignment…).
                Some('A' | 'C' | 'c' | 'H' | 'Q' | 'T' | 'W' | 'S') => {
                    let Some(end) = (i + 2..cs.len()).find(|k| at(*k) == Some(';')) else { break };
                    i = end + 1;
                }
                // On/off toggles (underline, overline, strike-through) and the non-breaking space.
                Some('L' | 'l' | 'O' | 'o' | 'K' | 'k') => i += 2,
                Some('~') => {
                    push(&mut rows, GdtRun::Text(" ".into()));
                    i += 2;
                }
                Some(_) => {
                    push(&mut rows, GdtRun::Text("\\".into()));
                    i += 1;
                }
                None => break,
            },
            c if gdt => {
                push(&mut rows, GdtSymbol::from_code(c).map(GdtRun::Symbol).unwrap_or_else(|| GdtRun::Text(c.to_string())));
                i += 1;
            }
            c => {
                push(&mut rows, GdtRun::Text(c.to_string()));
                i += 1;
            }
        }
    }
    rows
}

/// A compartment with nothing to show.
pub fn compartment_is_empty(c: &Compartment) -> bool {
    c.iter().all(|r| matches!(r, GdtRun::Text(t) if t.trim().is_empty()))
}

/// A tolerance value: optional diameter symbol, the value, optional material condition.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GdtTol {
    #[serde(default)]
    pub diameter: bool,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

/// A datum reference: the letter(s) and an optional material condition.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GdtDatum {
    #[serde(default)]
    pub letter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

/// One frame line as the Geometric Tolerance dialog fills it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GdtLine {
    pub symbol: Option<GdtSymbol>,
    pub tol1: Option<GdtTol>,
    pub tol2: Option<GdtTol>,
    pub datums: Vec<GdtDatum>,
}

/// A whole feature control frame: up to two tolerance lines, the projected tolerance zone and
/// the datum identifier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GdtFrame {
    pub lines: Vec<GdtLine>,
    /// Projected tolerance zone height ("" = none).
    pub height: String,
    pub projected: bool,
    /// Datum identifier ("" = none).
    pub datum_id: String,
}

/// Escape free text for a frame string (no line or compartment breaks, no formatting codes).
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().take(256) {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            // `%%%` is a literal percent sign (a lone `%` could start a `%%` code).
            '%' => out.push_str("%%%"),
            '\n' | '\r' | '^' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn sym(s: GdtSymbol) -> String {
    format!("{{\\Fgdt;{}}}", s.code())
}

/// The material condition symbol named by `m` ("M", "L", "S", "mmc"…).
fn material(m: &Option<String>) -> Option<GdtSymbol> {
    m.as_deref().and_then(GdtSymbol::parse).filter(|s| s.is_material())
}

/// The frame string for `f` (DXF group 1 of a TOLERANCE).
pub fn compose_frame(f: &GdtFrame) -> String {
    let mut lines: Vec<String> = Vec::new();
    let tol = |t: &Option<GdtTol>| -> String {
        let Some(t) = t else { return String::new() };
        let mut s = String::new();
        if t.diameter {
            s.push_str(&sym(GdtSymbol::Diameter));
        }
        s.push_str(&escape(t.value.trim()));
        if let Some(m) = material(&t.material) {
            s.push_str(&sym(m));
        }
        if t.value.trim().is_empty() && !t.diameter && material(&t.material).is_none() { String::new() } else { s }
    };
    for l in f.lines.iter().take(MAX_FRAME_ROWS) {
        let mut cells = vec![l.symbol.map(sym).unwrap_or_default(), tol(&l.tol1), tol(&l.tol2)];
        for d in l.datums.iter().take(3) {
            let mut s = escape(d.letter.trim());
            if let Some(m) = material(&d.material) {
                s.push_str(&sym(m));
            }
            cells.push(s);
        }
        while cells.last().is_some_and(String::is_empty) {
            cells.pop();
        }
        if !cells.is_empty() {
            lines.push(cells.join("%%v"));
        }
    }
    if !f.height.trim().is_empty() || f.projected {
        let mut s = escape(f.height.trim());
        if f.projected {
            s.push_str(&sym(GdtSymbol::Projected));
        }
        lines.push(s);
    }
    if !f.datum_id.trim().is_empty() {
        lines.push(escape(f.datum_id.trim()));
    }
    lines.join("^J")
}

/// A primitive of a laid-out frame, in WCS.
#[derive(Clone, Debug, PartialEq)]
pub enum GdtPrim {
    Line(Vec<Vec2>),
    Circle {
        center: Vec2,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` (radians).
    Arc {
        center: Vec2,
        radius: f64,
        start: f64,
        end: f64,
    },
    /// A filled triangle (arrowheads).
    Fill([Vec2; 3]),
}

/// A text run of a laid-out frame: left baseline point (WCS), height and rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct GdtText {
    pub pos: Vec2,
    pub height: f64,
    pub rotation: f64,
    pub value: String,
}

/// A laid-out feature control frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GdtLayout {
    /// The frame's boxes (drawn with the dimension line colour).
    pub frame: Vec<GdtPrim>,
    /// The symbols (drawn with the text colour).
    pub symbols: Vec<GdtPrim>,
    pub texts: Vec<GdtText>,
    pub bounds: Bounds2,
}

/// Lay out the frame `text` at `insert`, rotated by `rotation`, with text height `h` and
/// horizontal padding `gap`. `measure(s, h)` is the width of plain text `s` at height `h`.
pub fn layout_frame(text: &str, insert: Vec2, rotation: f64, h: f64, gap: f64, measure: &dyn Fn(&str, f64) -> f64) -> GdtLayout {
    let h = if h.is_finite() && h > 0.0 { h.min(1e12) } else { 0.18 };
    let gap = if gap.is_finite() { gap.abs().clamp(h * 0.25, h * 2.0) } else { h * 0.5 };
    let rotation = if rotation.is_finite() { rotation } else { 0.0 };
    let (insert, row_h) = (if insert.is_finite() { insert } else { Vec2::ZERO }, 2.0 * h);
    let to_w = |p: Vec2| insert + p.rotate(rotation);
    let mut out = GdtLayout::default();
    let mut prev_width = 0.0;
    let rows: Vec<Vec<Compartment>> = parse_frame(text)
        .into_iter()
        .map(|r| r.into_iter().filter(|c| !compartment_is_empty(c)).collect::<Vec<_>>())
        .filter(|r| !r.is_empty())
        .collect();
    let run_w = |r: &GdtRun| match r {
        GdtRun::Symbol(_) => h,
        GdtRun::Text(t) => {
            let w = measure(t, h);
            if w.is_finite() { w.clamp(0.0, h * 4096.0) } else { 0.0 }
        }
    };
    for (k, row) in rows.iter().enumerate() {
        let top = h - k as f64 * row_h;
        let (bot, mid) = (top - row_h, top - h);
        let mut x = 0.0;
        for cell in row {
            // Runs a little apart, so a modifier doesn't touch the value before it.
            let sep = 0.25 * h;
            let w: f64 = cell.iter().map(run_w).sum::<f64>() + sep * cell.len().saturating_sub(1) as f64;
            let cw = (w + 2.0 * gap).max(row_h);
            // The separator on the cell's left (the frame's left edge for the first cell).
            out.frame.push(GdtPrim::Line(vec![to_w(Vec2::new(x, top)), to_w(Vec2::new(x, bot))]));
            let mut cx = x + (cw - w) / 2.0;
            for r in cell {
                match r {
                    GdtRun::Symbol(s) => {
                        let o = Vec2::new(cx, mid - h / 2.0);
                        out.symbols.extend(symbol_prims(*s).into_iter().map(|p| map_prim(p, |q| to_w(o + q * h), h, rotation)));
                    }
                    GdtRun::Text(t) => {
                        out.texts.push(GdtText { pos: to_w(Vec2::new(cx, mid - h / 2.0)), height: h, rotation, value: t.clone() });
                    }
                }
                cx += run_w(r) + sep;
            }
            x += cw;
        }
        out.frame.push(GdtPrim::Line(vec![to_w(Vec2::new(x, top)), to_w(Vec2::new(x, bot))]));
        // Top edge: the first line's, or the part sticking out past the line above.
        if k == 0 {
            out.frame.push(GdtPrim::Line(vec![to_w(Vec2::new(0.0, top)), to_w(Vec2::new(x, top))]));
        } else if x > prev_width {
            out.frame.push(GdtPrim::Line(vec![to_w(Vec2::new(prev_width, top)), to_w(Vec2::new(x, top))]));
        }
        out.frame.push(GdtPrim::Line(vec![to_w(Vec2::new(0.0, bot)), to_w(Vec2::new(x, bot))]));
        out.bounds = out.bounds.union(&Bounds2::from_points([
            to_w(Vec2::new(0.0, top)),
            to_w(Vec2::new(x, bot)),
            to_w(Vec2::new(0.0, bot)),
            to_w(Vec2::new(x, top)),
        ]));
        prev_width = x;
    }
    if rows.is_empty() {
        out.bounds = Bounds2::from_points([insert]);
    }
    out
}

/// Map a unit-box primitive through `f` (scale `s`, rotation `rot`).
fn map_prim(p: GdtPrim, f: impl Fn(Vec2) -> Vec2, s: f64, rot: f64) -> GdtPrim {
    match p {
        GdtPrim::Line(v) => GdtPrim::Line(v.into_iter().map(f).collect()),
        GdtPrim::Circle { center, radius } => GdtPrim::Circle { center: f(center), radius: radius * s },
        GdtPrim::Arc { center, radius, start, end } => GdtPrim::Arc { center: f(center), radius: radius * s, start: start + rot, end: end + rot },
        GdtPrim::Fill(t) => GdtPrim::Fill(t.map(f)),
    }
}

/// An arrowhead at `tip` pointing along `d` (unit box units).
fn arrow(tip: Vec2, d: Vec2) -> GdtPrim {
    let d = d.normalized();
    let back = tip - d * 0.3;
    GdtPrim::Fill([tip, back + d.perp() * 0.1, back - d.perp() * 0.1])
}

/// A symbol drawn in a unit box (x and y in 0..1, the baseline at y = 0).
pub fn symbol_prims(s: GdtSymbol) -> Vec<GdtPrim> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let v = Vec2::new;
    let line = |pts: &[(f64, f64)]| GdtPrim::Line(pts.iter().map(|(x, y)| v(*x, *y)).collect());
    let circle = |x: f64, y: f64, r: f64| GdtPrim::Circle { center: v(x, y), radius: r };
    // Circled letters: the circle plus the letter's strokes.
    let circled = |mut letter: Vec<GdtPrim>| {
        letter.insert(0, circle(0.5, 0.5, 0.5));
        letter
    };
    match s {
        GdtSymbol::Position => vec![circle(0.5, 0.5, 0.3), line(&[(0.0, 0.5), (1.0, 0.5)]), line(&[(0.5, 0.0), (0.5, 1.0)])],
        GdtSymbol::Concentricity => vec![circle(0.5, 0.5, 0.45), circle(0.5, 0.5, 0.25)],
        GdtSymbol::Symmetry => vec![line(&[(0.0, 0.5), (1.0, 0.5)]), line(&[(0.2, 0.2), (0.8, 0.2)]), line(&[(0.2, 0.8), (0.8, 0.8)])],
        GdtSymbol::Parallelism => vec![line(&[(0.1, 0.05), (0.5, 0.95)]), line(&[(0.5, 0.05), (0.9, 0.95)])],
        GdtSymbol::Perpendicularity => vec![line(&[(0.05, 0.05), (0.95, 0.05)]), line(&[(0.5, 0.05), (0.5, 0.95)])],
        GdtSymbol::Angularity => vec![line(&[(0.95, 0.05), (0.05, 0.05), (0.85, 0.75)])],
        GdtSymbol::Cylindricity => {
            let (c, r) = (v(0.5, 0.5), 0.3);
            let d = Vec2::from_angle(65f64.to_radians());
            let n = d.perp() * r;
            vec![circle(0.5, 0.5, r), GdtPrim::Line(vec![c + n - d * 0.5, c + n + d * 0.5]), GdtPrim::Line(vec![c - n - d * 0.5, c - n + d * 0.5])]
        }
        GdtSymbol::Flatness => vec![line(&[(0.0, 0.25), (0.7, 0.25), (1.0, 0.75), (0.3, 0.75), (0.0, 0.25)])],
        GdtSymbol::Circularity => vec![circle(0.5, 0.5, 0.45)],
        GdtSymbol::Straightness => vec![line(&[(0.0, 0.5), (1.0, 0.5)])],
        GdtSymbol::SurfaceProfile => {
            vec![GdtPrim::Arc { center: v(0.5, 0.2), radius: 0.45, start: 0.0, end: PI }, line(&[(0.05, 0.2), (0.95, 0.2)])]
        }
        GdtSymbol::LineProfile => vec![GdtPrim::Arc { center: v(0.5, 0.2), radius: 0.45, start: 0.0, end: PI }],
        GdtSymbol::CircularRunout => vec![line(&[(0.2, 0.05), (0.7, 0.95)]), arrow(v(0.7, 0.95), v(0.5, 0.9))],
        GdtSymbol::TotalRunout => vec![
            line(&[(0.05, 0.05), (0.5, 0.05)]),
            line(&[(0.05, 0.05), (0.45, 0.95)]),
            line(&[(0.5, 0.05), (0.9, 0.95)]),
            arrow(v(0.45, 0.95), v(0.4, 0.9)),
            arrow(v(0.9, 0.95), v(0.4, 0.9)),
        ],
        GdtSymbol::Diameter => vec![circle(0.5, 0.5, 0.35), line(&[(0.1, 0.0), (0.9, 1.0)])],
        GdtSymbol::Mmc => circled(vec![line(&[(0.3, 0.25), (0.3, 0.75), (0.5, 0.45), (0.7, 0.75), (0.7, 0.25)])]),
        GdtSymbol::Lmc => circled(vec![line(&[(0.38, 0.75), (0.38, 0.25), (0.65, 0.25)])]),
        GdtSymbol::Rfs => circled(vec![
            GdtPrim::Arc { center: v(0.5, 0.625), radius: 0.125, start: 20f64.to_radians(), end: 3.0 * FRAC_PI_2 },
            GdtPrim::Arc { center: v(0.5, 0.375), radius: 0.125, start: 200f64.to_radians(), end: FRAC_PI_2 },
        ]),
        GdtSymbol::Projected => circled(vec![
            line(&[(0.5, 0.5), (0.38, 0.5), (0.38, 0.75), (0.5, 0.75)]),
            GdtPrim::Arc { center: v(0.5, 0.625), radius: 0.125, start: -FRAC_PI_2, end: FRAC_PI_2 },
            line(&[(0.38, 0.5), (0.38, 0.25)]),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_symbols_compartments_and_lines() {
        let rows = parse_frame("{\\Fgdt;j}%%v{\\Fgdt;n}0.1{\\Fgdt;m}%%vA%%vB^J2.5{\\Fgdt;p}");
        assert_eq!(rows.len(), 2);
        let r0 = &rows[0];
        assert_eq!(r0.len(), 4);
        assert_eq!(r0[0], vec![GdtRun::Symbol(GdtSymbol::Position)]);
        assert_eq!(r0[1], vec![GdtRun::Symbol(GdtSymbol::Diameter), GdtRun::Text("0.1".into()), GdtRun::Symbol(GdtSymbol::Mmc)]);
        assert_eq!(r0[3], vec![GdtRun::Text("B".into())]);
        assert_eq!(rows[1][0], vec![GdtRun::Text("2.5".into()), GdtRun::Symbol(GdtSymbol::Projected)]);
    }

    #[test]
    fn compose_then_parse() {
        let f = GdtFrame {
            lines: vec![GdtLine {
                symbol: Some(GdtSymbol::Flatness),
                tol1: Some(GdtTol { diameter: false, value: "0.05".into(), material: None }),
                tol2: None,
                datums: vec![GdtDatum { letter: "A".into(), material: Some("L".into()) }],
            }],
            height: String::new(),
            projected: false,
            datum_id: "B".into(),
        };
        let s = compose_frame(&f);
        assert_eq!(s, "{\\Fgdt;c}%%v0.05%%v%%vA{\\Fgdt;l}^JB");
        let rows = parse_frame(&s);
        assert_eq!(rows[0][3], vec![GdtRun::Text("A".into()), GdtRun::Symbol(GdtSymbol::Lmc)]);
        assert_eq!(rows[1][0], vec![GdtRun::Text("B".into())]);
    }

    #[test]
    fn hostile_strings_do_not_panic() {
        let huge = "%%v{\\Fgdt;j}".repeat(100_000);
        for s in ["{\\Fgdt;", "{\\Fgdt", "\\", "%%", "%", "{{{{", "}}}}", "^", "\\P\\P\\P", "{\\H", &huge, "\u{0}%%v\u{ffff}"] {
            let rows = parse_frame(s);
            assert!(rows.len() <= MAX_FRAME_ROWS && rows.iter().all(|r| r.len() <= MAX_FRAME_CELLS));
            let l = layout_frame(s, Vec2::new(f64::NAN, 1.0), f64::INFINITY, f64::NAN, -1.0, &|t, h| t.len() as f64 * h);
            assert!(l.frame.iter().all(|p| matches!(p, GdtPrim::Line(v) if v.iter().all(|q| q.is_finite()))));
        }
    }

    #[test]
    fn layout_boxes_each_compartment() {
        let l = layout_frame("{\\Fgdt;j}%%v0.1%%vA", Vec2::ZERO, 0.0, 1.0, 0.5, &|t, h| t.chars().count() as f64 * h);
        // Three cells: four separators, top and bottom.
        assert_eq!(l.frame.len(), 6);
        assert_eq!(l.texts.len(), 2);
        assert!((l.bounds.min.y + 1.0).abs() < 1e-9 && (l.bounds.max.y - 1.0).abs() < 1e-9);
        assert!(l.bounds.min.x.abs() < 1e-9 && l.bounds.max.x > 6.0);
    }
}
