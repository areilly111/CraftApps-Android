use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_raster::from_rgba;
use serde_json::json;
use std::time::Instant;

use super::*;

/// The single pass over the content bounds that the tiled `adjust_surface` replaced (#1772).
fn one_pass(s: &Surface, adj: &Adjustment, selection: Option<&Surface>, mode: ColorMode) -> Surface {
    let mut s = s.clone();
    let r = s.content_bounds();
    if !r.is_empty() {
        let out = adjusted(&s, r, adj, selection, mode);
        s.write_region(r, &out);
    }
    s
}

/// Two patches over a 3×3 tile grid, not tile-aligned, with partial alpha; the bottom-left tiles
/// inside the content bounds are never allocated.
fn layer(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    for r in [Rect::new(200, 240, 300, 300), Rect::new(530, 250, 560, 530)] {
        let mut data = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let a = if fmt.alpha { 0.3 + 0.7 * ((x + y) % 7) as f32 / 6.0 } else { 1.0 };
                data.extend(from_rgba(&fmt, [x as f32 / 600.0, y as f32 / 540.0, 0.5 + 0.4 * (x as f32 * 0.05).sin(), a]));
            }
        }
        s.write_region(r, &data);
    }
    assert!(!s.has_tiles_in(Rect::new(200, 512, 256, 530)), "a hole inside the content bounds");
    s
}

/// Release-only comparison for the 5000×7073 px group-merge report (#1561). The reference path
/// models the old full-canvas composite plus a second converted float buffer; the optimized path
/// renders only the group's visible content bounds in bands.
#[test]
#[ignore = "manual 35 MP merge benchmark; run in release with --ignored --nocapture"]
fn benchmark_large_group_merge() {
    let fmt = PixelFormat::RGBA8;
    let mut doc = Document::new("merge benchmark", Size::new(5000, 7073), ColorMode::Rgb, SampleType::U8);
    let content = Rect::new(1700, 2200, 2700, 3200);
    let mut children = Vec::new();
    for i in 0..4 {
        let mut child = Layer::raster(format!("Layer {i}"), fmt);
        if let Some(surface) = child.surface_mut() {
            surface.fill_rect(content, &[(i as f32 + 1.0) / 5.0, 0.2, 0.4, 0.5]);
        }
        children.push(child);
    }
    doc.layers.push(Layer::group("Group", children));
    let expected_area = photocraft_compose::layer_bounds(&doc.layers[0], doc.bounds()).intersect(&doc.bounds());

    let start = Instant::now();
    let reference = photocraft_compose::flatten(&doc);
    let reference_data: Vec<f32> = reference.px.iter().flat_map(|p| photocraft_raster::from_rgba(&fmt, *p)).collect();
    let mut reference_surface = Surface::new(fmt);
    reference_surface.write_region(doc.bounds(), &reference_data);
    reference_surface.prune();
    let reference_time = start.elapsed();

    let start = Instant::now();
    let optimized = composite_layers(&doc, fmt, None);
    let optimized_time = start.elapsed();
    assert_eq!(reference_surface.read_region(expected_area), optimized.read_region(expected_area));
    println!("#1561 5000x7073, 4x 1000x1000 layers: full-canvas {:?}, bounded/banded {:?}", reference_time, optimized_time);
}

#[test]
fn tiled_adjustment_matches_one_pass_over_the_content() {
    let adjustments = [
        ("invert", json!({})),
        ("levels", json!({"inBlack": 20, "gamma": 1.3, "inWhite": 230})),
        ("curves", json!({"points": [[0, 10], [128, 160], [255, 240]]})),
        ("hueSaturation", json!({"hue": 40, "saturation": 20})),
        ("brightnessContrast", json!({"brightness": 30, "contrast": 20})),
        ("exposure", json!({"exposure": 0.8})),
        ("posterize", json!({"levels": 5})),
        ("threshold", json!({"level": 120})),
        ("colorBalance", json!({"midtones": [20, -10, 5]})),
        ("blackWhite", json!({"tint": true})),
        ("photoFilter", json!({"density": 50})),
        ("channelMixer", json!({"red": [80, 30, -10, 5]})),
        ("vibrance", json!({"vibrance": 40})),
        ("selectiveColor", json!({"reds": [10, -20, 30, 5]})),
        // Dithered kinds vary by position, so they show a tile's pixels kept their coordinates.
        ("gradientMap", json!({"stops": [[0, "#102030"], [1, "#f0e0a0"]], "dither": true})),
        ("colorLookup", json!({"lut": "warm", "dither": true})),
    ];
    let mut formats = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            formats.push(PixelFormat { mode, sample, alpha: true });
        }
    }
    formats.push(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: false });
    formats.push(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U16, alpha: false });
    // A soft selection that ends inside the content.
    let mut soft = Surface::new(PixelFormat::GRAY8);
    let sr = Rect::new(0, 0, 500, 400);
    soft.write_region(sr, &(0..sr.height()).flat_map(|_| (0..sr.width()).map(|x| x as f32 / 500.0)).collect::<Vec<_>>());
    for fmt in formats {
        let s = layer(fmt);
        for (kind, p) in &adjustments {
            let adj = crate::adjust_params::from_params(kind, p, None, fmt.mode).unwrap_or_else(|e| panic!("{kind}: {e}"));
            for sel in [None, Some(&soft)] {
                let want = one_pass(&s, &adj, sel, fmt.mode);
                let mut got = s.clone();
                adjust_surface(&mut got, &adj, sel, fmt.mode);
                assert!(want != s, "{kind} {fmt:?}: the adjustment changes the layer");
                assert!(got == want, "{kind} {fmt:?} selection {}: tiled result differs", sel.is_some());
            }
        }
    }
}

#[test]
fn an_empty_layer_is_left_alone() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    adjust_surface(&mut s, &Adjustment::Invert, None, ColorMode::Rgb);
    assert_eq!(s.tile_count(), 0);
}

/// The whole-region float pass that the tiled Clear and background Fill replaced (#3463).
fn reference_fill_or_clear(s: &Surface, area: Rect, selection: Option<&Surface>, fill: Option<([f32; 4], bool)>) -> Surface {
    let mut s = s.clone();
    let fmt = s.format();
    let n = fmt.channels();
    if fill.is_none() && !fmt.alpha {
        return s;
    }
    let w = area.width() as usize;
    let mut region = s.read_region(area);
    for (i, px) in region.chunks_exact_mut(n).enumerate() {
        let k = selection.map_or(1.0, |sel| sel.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0));
        match fill {
            Some((color, lock)) => {
                if k > 0.0 {
                    fill_pixel(&fmt, px, color, k, lock);
                }
            }
            None => {
                px[n - 1] *= 1.0 - k;
                if px[n - 1] <= 0.0 {
                    px.fill(0.0);
                }
            }
        }
    }
    s.write_region(area, &region);
    s
}

/// [`layer`] plus a fully painted tile and a transparent pixel that still carries a colour.
fn clear_layer(fmt: PixelFormat) -> Surface {
    let mut s = layer(fmt);
    s.fill_rect(Rect::new(0, 0, 256, 256), &from_rgba(&fmt, [0.2, 0.7, 0.4, if fmt.alpha { 0.8 } else { 1.0 }]));
    s.write_pixel(260, 250, &from_rgba(&fmt, [0.9, 0.1, 0.1, 0.0]));
    s
}

fn same_pixels(a: &Surface, b: &Surface) -> bool {
    let r = Rect::new(-10, -10, 800, 800);
    a.read_region(r) == b.read_region(r)
}

#[test]
fn tiled_clear_and_fill_match_one_pass_over_the_area() {
    let mut formats = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            formats.push(PixelFormat { mode, sample, alpha: true });
        }
    }
    formats.push(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: false });
    formats.push(PixelFormat { mode: ColorMode::Cmyk, sample: SampleType::U16, alpha: false });
    // A soft (feathered) selection: fully selected over the first tile, fading out across the next.
    let mut soft = Surface::new(PixelFormat::GRAY8);
    let sr = Rect::new(0, 0, 600, 400);
    soft.write_region(sr, &(0..sr.height()).flat_map(|_| (0..sr.width()).map(|x| (1.0 - (x as f32 - 256.0).max(0.0) / 300.0).max(0.0))).collect::<Vec<_>>());
    let canvas = Rect::new(0, 0, 700, 600);
    for fmt in formats {
        let s = clear_layer(fmt);
        for sel in [None, Some(&soft)] {
            let area = sel.map_or(canvas, |m| m.content_bounds());
            let what = format!("{fmt:?} selection {}", sel.is_some());
            let mut got = s.clone();
            try_clear_surface(&mut got, area, sel).unwrap();
            assert!(same_pixels(&got, &reference_fill_or_clear(&s, area, sel, None)), "clear {what}");
            if fmt.alpha {
                assert!(!same_pixels(&got, &s), "clear {what} changes the layer");
            }
            for color in [[1.0, 1.0, 1.0, 1.0], [0.3, 0.5, 0.9, 1.0], [0.3, 0.5, 0.9, 0.4]] {
                for lock in [true, false] {
                    let mut got = s.clone();
                    try_fill_surface(&mut got, area, color, sel, lock).unwrap();
                    assert!(same_pixels(&got, &reference_fill_or_clear(&s, area, sel, Some((color, lock)))), "fill {color:?} lock {lock} {what}");
                }
            }
        }
    }
}

#[test]
fn clear_leaves_unselected_tiles_shared_and_drops_cleared_ones() {
    let s = clear_layer(PixelFormat::RGBA8);
    let mut sel = Surface::new(PixelFormat::GRAY8);
    // Fully selects tile (0, 0) and part of (1, 0); the selection's bounds reach tile (2, 1),
    // whose pixels it leaves alone.
    sel.fill_rect(Rect::new(0, 0, 280, 256), &[1.0]);
    sel.write_pixel(600, 300, &[0.0]);
    sel.write_pixel(601, 301, &[0.5]);
    let mut got = s.clone();
    try_clear_surface(&mut got, Rect::new(0, 0, 602, 302), Some(&sel)).unwrap();
    let tile = |s: &Surface, tx, ty| s.tile(photocraft_geom::TileCoord::new(tx, ty)).cloned();
    assert!(tile(&got, 0, 0).is_none(), "a fully cleared tile is no tile");
    let (a, b) = (tile(&s, 2, 1).unwrap(), tile(&got, 2, 1).unwrap());
    assert!(std::sync::Arc::ptr_eq(&a, &b), "a tile the selection leaves alone is not copied");
    assert!(!std::sync::Arc::ptr_eq(&tile(&s, 1, 0).unwrap(), &tile(&got, 1, 0).unwrap()), "a partly cleared tile is copied");
}

/// Release-only timing for Edit › Clear on a 48 MP layer (#3463): about 2 s with the old
/// whole-region float pass, tens of milliseconds tiled.
#[test]
#[ignore = "benchmark; run with --release -- --ignored --nocapture"]
fn benchmark_clear_large_selection() {
    for (background, feather) in [(false, 0), (false, 200), (true, 0), (true, 200)] {
        let mut s = crate::Session::new();
        s.execute("file.new", json!({"width": 8000, "height": 6000, "background": "white"})).unwrap();
        if !background {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        }
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 7980, "height": 5980, "ellipse": feather > 0, "feather": feather})).unwrap();
        let t = Instant::now();
        s.execute("edit.clear", json!({})).unwrap();
        println!("#3463 clear 8000x6000, background {background}, feather {feather}: {:?}", t.elapsed());
    }
}

/// Image › Adjustments › Invert in a Lab document mirrors L, a and b, at every depth.
#[test]
fn invert_in_a_lab_document_mirrors_l_a_and_b() {
    use photocraft_color::convert::{lab_to_srgb, srgb_to_lab};
    let c = lab_to_srgb([30.0, 12.0, -18.0]);
    for depth in SampleType::ALL {
        let fmt = PixelFormat { mode: ColorMode::Lab, sample: depth, alpha: true };
        let mut s = Surface::new(fmt);
        let r = Rect::new(0, 0, 2, 2);
        s.write_region(r, &(0..4).flat_map(|_| from_rgba(&fmt, [c[0], c[1], c[2], 1.0])).collect::<Vec<_>>());
        adjust_surface(&mut s, &Adjustment::Invert, None, ColorMode::Lab);
        let px = s.read_region(Rect::new(1, 1, 2, 2));
        let rgba = photocraft_raster::to_rgba(&fmt, &px);
        let got = srgb_to_lab([rgba[0], rgba[1], rgba[2]]);
        assert!(got.iter().zip([70.0, -13.0, 17.0]).all(|(g, w)| (g - w).abs() < 1.0), "{depth:?}: {got:?}");
    }
}
