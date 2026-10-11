//! View → Show Print Tiling: the pages of a job in document space, as [`super::preview`] lays
//! them out: the layout [`super::plan`] gives the PDF and PostScript writers, so the canvas shows
//! what prints (without separating the plates, which the overlay doesn't need).

use kurbo::{Affine, Rect};
use serde::Serialize;

use super::{PrintPreview, PrintScaling, PrintSettings};

/// One page of the print tiling.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TilingPage {
    /// 0-based artboard (`None`: artboards ignored).
    pub artboard: Option<usize>,
    /// 1-based: the tile (across then down), or the page of the job.
    pub number: usize,
    /// It prints (tiles outside the tile range don't).
    pub printed: bool,
    /// The paper's edge, `[x0, y0, x1, y1]` in document space.
    pub page: [f64; 4],
    /// Its imageable area (inside the unprintable margin).
    pub imageable: [f64; 4],
}

fn arr(r: Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// The pages of the job `pv` previews (printed with `set`), in document space: every tile of each
/// artboard when tiling, else each page once (not once per ink).
pub fn tiling(pv: &PrintPreview, set: &PrintSettings) -> Vec<TilingPage> {
    let m = set.margin;
    if set.scaling.tiles() {
        // Tiles are the paper (full pages) or its imageable area, at the print scale.
        let (sx, sy) = (set.scale.width / 100.0, set.scale.height / 100.0);
        let inset = (m / sx, m / sy);
        return pv
            .tiles
            .iter()
            .flat_map(|g| {
                g.tiles.iter().enumerate().map(move |(i, t)| {
                    let tile = Rect::new(t[0], t[1], t[2], t[3]);
                    let (page, imageable) = if set.scaling == PrintScaling::TileFull {
                        (tile, tile.inflate(-inset.0, -inset.1))
                    } else {
                        (tile.inflate(inset.0, inset.1), tile)
                    };
                    TilingPage {
                        artboard: g.artboard,
                        number: i + 1,
                        printed: g.printed.contains(&(i + 1)),
                        page: arr(page),
                        imageable: arr(imageable),
                    }
                })
            })
            .collect();
    }
    let mut out: Vec<TilingPage> = vec![];
    let mut last = None;
    for s in &pv.sheets {
        // Separations repeat each page once per ink, one after the other.
        if last == Some((s.artboard, s.tile)) {
            continue;
        }
        last = Some((s.artboard, s.tile));
        let to_page = Affine::new(s.transform);
        let det = to_page.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            continue;
        }
        let back = to_page.inverse();
        let paper = Rect::new(0.0, 0.0, s.width, s.height);
        let page = back.transform_rect_bbox(paper);
        let imageable = back.transform_rect_bbox(paper.inflate(-m, -m));
        out.push(TilingPage { artboard: s.artboard, number: out.len() + 1, printed: true, page: arr(page), imageable: arr(imageable) });
    }
    out
}
