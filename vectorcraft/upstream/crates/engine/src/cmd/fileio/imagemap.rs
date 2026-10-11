//! Image maps of raster exports (JPEG Options → Image Map): the objects with a URL (Attributes
//! panel) as clickable areas of the exported image, written beside it as a client-side map (an
//! HTML page with `<map>`/`<area>`) or a server-side map (an NCSA `.map` file).

use std::fmt::Write as _;

use kurbo::{PathEl, Shape};
use vectorcraft_doc::{Document, ImageMap, Node, NodeKind};
use vectorcraft_geom::Rect;

/// Where the image map goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapKind {
    #[default]
    None,
    /// An HTML page showing the image with a `<map>` (`.html`).
    Client,
    /// An NCSA map file for the web server (`.map`).
    Server,
}

impl MapKind {
    pub fn from_id(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Some(MapKind::None),
            "client" | "clientside" | "client-side" => Some(MapKind::Client),
            "server" | "serverside" | "server-side" => Some(MapKind::Server),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            MapKind::Server => "map",
            _ => "html",
        }
    }
}

/// The shape of a clickable area, in image pixels.
#[derive(Clone, Debug, PartialEq)]
enum Outline {
    Rect([i64; 4]),
    Poly(Vec<[i64; 2]>),
}

#[derive(Clone, Debug, PartialEq)]
struct Area {
    shape: Outline,
    href: String,
    alt: String,
}

/// The image map of one exported image, written next to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub kind: MapKind,
    /// The image it belongs to (an index into the export's files).
    pub file: usize,
    width: u32,
    height: u32,
    /// Topmost first: the first area under the pointer wins.
    areas: Vec<Area>,
}

/// The areas of the visible objects of `doc` with a URL and an Image Map shape that `region`
/// (exported at `scale` pixels per point) shows. `None` when there are none.
pub fn build(doc: &Document, region: Rect, scale: f64, kind: MapKind, file: usize) -> Option<Map> {
    if kind == MapKind::None {
        return None;
    }
    let mut areas = vec![];
    for l in &doc.layers {
        collect(l, region, scale, &mut areas);
    }
    areas.reverse();
    let (width, height) = vectorcraft_render::region_pixels(region, scale);
    (!areas.is_empty()).then_some(Map { kind, file, width, height, areas })
}

fn collect(n: &Node, region: Rect, scale: f64, out: &mut Vec<Area>) {
    if !n.visible || matches!(n.kind, NodeKind::Layer { template: true, .. }) {
        return;
    }
    let shape = n.attrs.as_deref().filter(|a| !a.url.is_empty()).map(|a| a.image_map).unwrap_or_default();
    let px = |x: f64, y: f64| [((x - region.x0) * scale).round() as i64, ((y - region.y0) * scale).round() as i64];
    if let (Some(href), Some(b)) = (n.url(), n.visual_bounds().filter(|b| b.overlaps(region)))
        && shape != ImageMap::None
    {
        let outline = match (&n.kind, shape) {
            (NodeKind::Path { path, .. }, ImageMap::Polygon) => {
                polygon(&path.to_bezpath(), 0.5 / scale).map(|pts| pts.into_iter().map(|p| px(p.x, p.y)).collect())
            }
            _ => None,
        };
        let shape = match outline {
            Some(pts) => Outline::Poly(pts),
            None => {
                let ([x0, y0], [x1, y1]) = (px(b.x0, b.y0), px(b.x1, b.y1));
                Outline::Rect([x0, y0, x1, y1])
            }
        };
        out.push(Area { shape, href: href.to_string(), alt: n.name.clone().unwrap_or_default() });
    }
    for c in n.children().into_iter().flatten() {
        collect(c, region, scale, out);
    }
}

/// The first subpath of `path` as straight segments within `tolerance` (at least three points).
fn polygon(path: &kurbo::BezPath, tolerance: f64) -> Option<Vec<kurbo::Point>> {
    let mut pts = vec![];
    let mut done = false;
    kurbo::flatten(path.path_elements(tolerance), tolerance, |el| match el {
        _ if done => {}
        PathEl::MoveTo(p) if pts.is_empty() => pts.push(p),
        PathEl::LineTo(p) => pts.push(p),
        PathEl::ClosePath | PathEl::MoveTo(_) => done = true,
        _ => {}
    });
    pts.dedup();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    (pts.len() >= 3).then_some(pts)
}

/// Escape text for an HTML attribute (or element text).
pub(crate) fn attr(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).fold(String::with_capacity(s.len()), |mut o, c| {
        match c {
            '&' => o.push_str("&amp;"),
            '"' => o.push_str("&quot;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            c => o.push(c),
        }
        o
    })
}

impl Map {
    /// The map file's text for an image named `image` (its file name).
    pub fn text(&self, image: &str) -> String {
        let join = |pts: &[[i64; 2]], sep: &str| pts.iter().map(|[x, y]| format!("{x},{y}")).collect::<Vec<_>>().join(sep);
        let mut out = String::new();
        match self.kind {
            MapKind::Server => {
                for a in &self.areas {
                    // NCSA format: no spaces inside a URL.
                    let href = a.href.replace(char::is_whitespace, "%20");
                    let _ = match &a.shape {
                        Outline::Rect([x0, y0, x1, y1]) => writeln!(out, "rect {href} {x0},{y0} {x1},{y1}"),
                        Outline::Poly(pts) => writeln!(out, "poly {href} {}", join(pts, " ")),
                    };
                }
            }
            _ => {
                let name = attr(std::path::Path::new(image).file_stem().map_or("image".into(), |s| s.to_string_lossy()).as_ref());
                let _ = writeln!(out, "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{name}</title>\n</head>\n<body>");
                let _ =
                    writeln!(out, "<img src=\"{}\" width=\"{}\" height=\"{}\" usemap=\"#{name}\" alt=\"\">", attr(image), self.width, self.height);
                let _ = writeln!(out, "<map name=\"{name}\">");
                for a in &self.areas {
                    let (shape, coords) = match &a.shape {
                        Outline::Rect(r) => ("rect", r.map(|v| v.to_string()).join(",")),
                        Outline::Poly(pts) => ("poly", join(pts, ",")),
                    };
                    let _ = writeln!(out, "<area shape=\"{shape}\" coords=\"{coords}\" href=\"{}\" alt=\"{}\">", attr(&a.href), attr(&a.alt));
                }
                out.push_str("</map>\n</body>\n</html>\n");
            }
        }
        out
    }
}
