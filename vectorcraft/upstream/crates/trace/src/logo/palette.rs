//! Finding a logo's flat colours, and telling flat-colour art from gradients and photos.

use super::unmix::Px;

/// A colour with every channel at or above this is white, which is always a class of its own.
pub(crate) const WHITE_MIN: u8 = 245;

/// `#rrggbb` or `rrggbb`.
pub(crate) fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some([v(0)?, v(2)?, v(4)?])
}

fn rgb255(p: &Px) -> [f64; 3] {
    let a = f64::from(p[3]).max(1e-6);
    [f64::from(p[0]) / a * 255.0, f64::from(p[1]) / a * 255.0, f64::from(p[2]) / a * 255.0]
}

/// Opaque pixels whose 3×3 neighbourhood is uniform (variance below `max_var`): edge pixels, which
/// are blends, do not get a vote.
fn solid_pixels(pix: &[Px], w: usize, h: usize, max_var: f64) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    if w < 3 || h < 3 {
        return pix.iter().filter(|p| p[3] > 0.98).map(rgb255).collect();
    }
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let Some(c) = pix.get(y * w + x) else { continue };
            if c[3] <= 0.98 {
                continue;
            }
            let (mut s, mut s2, mut cnt) = ([0.0f64; 3], [0.0f64; 3], 0.0f64);
            let mut opaque = true;
            for dy in 0..3 {
                for dx in 0..3 {
                    let Some(q) = pix.get((y + dy - 1) * w + (x + dx - 1)) else { continue };
                    if q[3] <= 0.98 {
                        opaque = false;
                    }
                    let v = rgb255(q);
                    for k in 0..3 {
                        s[k] += v[k];
                        s2[k] += v[k] * v[k];
                    }
                    cnt += 1.0;
                }
            }
            if !opaque || cnt < 9.0 {
                continue;
            }
            let var: f64 = (0..3).map(|k| s2[k] / cnt - (s[k] / cnt).powi(2)).sum();
            if var < max_var {
                out.push(rgb255(c));
            }
        }
    }
    out
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

fn kmeans(px: &[[f64; 3]], k: usize, seed: u64) -> (Vec<[f64; 3]>, Vec<usize>, f64) {
    let mut rng = Lcg(seed);
    let mut c: Vec<[f64; 3]> = vec![px[(rng.next() * px.len() as f64) as usize % px.len()]];
    while c.len() < k {
        let d: Vec<f64> = px.iter().map(|p| c.iter().map(|q| dist2(*p, *q)).fold(f64::MAX, f64::min)).collect();
        let total: f64 = d.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut r = rng.next() * total;
        let mut pick = px.len() - 1;
        for (i, v) in d.iter().enumerate() {
            r -= v;
            if r <= 0.0 {
                pick = i;
                break;
            }
        }
        c.push(px[pick]);
    }
    let mut lab = vec![0usize; px.len()];
    for _ in 0..30 {
        for (i, p) in px.iter().enumerate() {
            lab[i] = (0..c.len()).min_by(|&a, &b| dist2(*p, c[a]).total_cmp(&dist2(*p, c[b]))).unwrap_or(0);
        }
        let mut sum = vec![[0.0f64; 3]; c.len()];
        let mut cnt = vec![0usize; c.len()];
        for (p, &l) in px.iter().zip(&lab) {
            for j in 0..3 {
                sum[l][j] += p[j];
            }
            cnt[l] += 1;
        }
        for (i, ci) in c.iter_mut().enumerate() {
            if cnt[i] > 0 {
                *ci = [sum[i][0] / cnt[i] as f64, sum[i][1] / cnt[i] as f64, sum[i][2] / cnt[i] as f64];
            }
        }
    }
    let cost = px.iter().zip(&lab).map(|(p, &l)| dist2(*p, c[l])).sum();
    (c, lab, cost)
}

/// Up to `max_colors` flat colours, most common first. White is left out (it is always a class of
/// its own), as are colours that make up under 0.1% of the solid pixels (noise).
pub(crate) fn auto_palette(pix: &[Px], w: usize, h: usize, max_colors: usize) -> Vec<[u8; 3]> {
    let mut px = solid_pixels(pix, w, h, 20.0);
    if px.len() < max_colors.max(2) * 20 {
        px = pix.iter().filter(|p| p[3] > 0.9).map(rgb255).collect();
    }
    if px.is_empty() {
        return Vec::new();
    }
    if px.len() > 20_000 {
        let stride = px.len() / 20_000 + 1;
        px = px.into_iter().step_by(stride).collect();
    }
    let k = max_colors.clamp(1, 12).min(px.len());
    let best = (0..4u64).map(|s| kmeans(&px, k, 0x9e37_79b9 + s * 7919)).min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((cent, lab, _)) = best else { return Vec::new() };
    let mut cnt = vec![0usize; cent.len()];
    for &l in &lab {
        if let Some(c) = cnt.get_mut(l) {
            *c += 1;
        }
    }
    let total = lab.len().max(1);
    let mut groups: Vec<([f64; 3], usize)> =
        cent.iter().zip(&cnt).filter(|(c, n)| **n * 1000 > total && !c.iter().all(|&v| v >= f64::from(WHITE_MIN))).map(|(c, &n)| (*c, n)).collect();
    // Centres closer than 20 are one colour split by noise.
    loop {
        let mut merged = false;
        'outer: for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                if dist2(groups[i].0, groups[j].0) < 400.0 {
                    let (a, b) = (groups[i], groups[j]);
                    let n = (a.1 + b.1) as f64;
                    let c = [0, 1, 2].map(|k| (a.0[k] * a.1 as f64 + b.0[k] * b.1 as f64) / n);
                    groups[i] = (c, a.1 + b.1);
                    groups.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.iter().map(|(c, _)| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect()
}

/// Colours the current palette cannot explain. `residual` is each pixel's error after the best blend
/// of the known classes (see `unmix`); a solid run of pixels (a pixel and its four neighbours agree)
/// with a large error is a colour that is missing, such as the small accent that was too few
/// pixels to be found at first. Groups of at least three such pixels at least 25 away from every
/// known colour are returned, most common first.
pub(crate) fn unexplained_colours(pix: &[Px], w: usize, h: usize, residual: &[f32], known: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut pal: Vec<[f64; 3]> = known.iter().map(|c| c.map(f64::from)).collect();
    pal.push([255.0, 255.0, 255.0]);
    let at = |x: usize, y: usize| pix.get(y * w + x).filter(|p| p[3] > 0.98).map(rgb255);
    let mut groups: Vec<([f64; 3], usize)> = Vec::new();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            if residual.get(y * w + x).copied().unwrap_or(0.0) < 0.0064 {
                continue;
            }
            let Some(c) = at(x, y) else { continue };
            let uniform = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|&(xx, yy)| at(xx, yy).is_some_and(|q| dist2(q, c) < 100.0));
            if !uniform || pal.iter().any(|q| dist2(*q, c) < 625.0) {
                continue;
            }
            match groups.iter_mut().find(|(m, _)| dist2(*m, c) < 400.0) {
                Some((m, n)) => {
                    for k in 0..3 {
                        m[k] = (m[k] * *n as f64 + c[k]) / (*n as f64 + 1.0);
                    }
                    *n += 1;
                }
                None => groups.push((c, 1)),
            }
        }
    }
    groups.retain(|g| g.1 >= 3);
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.iter().map(|(c, _)| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect()
}

/// Share of solid pixels within a small colour distance of a palette colour (white included).
/// About 1.0 for flat-colour art; far lower for gradients, shading and photos.
pub(crate) fn flat_fraction(pix: &[Px], w: usize, h: usize, palette: &[[u8; 3]]) -> f64 {
    let px = solid_pixels(pix, w, h, 60.0);
    if px.is_empty() {
        return 0.0;
    }
    let mut pal: Vec<[f64; 3]> = palette.iter().map(|c| c.map(f64::from)).collect();
    pal.push([255.0, 255.0, 255.0]);
    let near = px.iter().filter(|p| pal.iter().any(|q| dist2(**p, *q) < 22.0 * 22.0)).count();
    near as f64 / px.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<Px> {
        (0..w * h)
            .map(|i| {
                let p = f(i % w, i / w);
                let a = f32::from(p[3]) / 255.0;
                [f32::from(p[0]) / 255.0 * a, f32::from(p[1]) / 255.0 * a, f32::from(p[2]) / 255.0 * a, a]
            })
            .collect()
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#6C60F6"), Some([0x6c, 0x60, 0xf6]));
        assert_eq!(parse_hex("ea4335"), Some([0xea, 0x43, 0x35]));
        for bad in ["", "#12345", "#1234567", "#gg0000", "é2345z", "  "] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn three_flat_colours_are_found_exactly() {
        let pix = blocks(60, 20, |x, _| match x / 20 {
            0 => [200, 30, 30, 255],
            1 => [30, 160, 60, 255],
            _ => [60, 70, 180, 255],
        });
        let mut pal = auto_palette(&pix, 60, 20, 6);
        pal.sort();
        let mut want = vec![[200, 30, 30], [30, 160, 60], [60, 70, 180]];
        want.sort();
        assert_eq!(pal, want);
        assert!(flat_fraction(&pix, 60, 20, &pal) > 0.99);
    }

    #[test]
    fn white_is_never_in_the_palette_and_transparency_is_ignored() {
        let pix = blocks(40, 20, |x, _| {
            if x < 10 {
                [255, 255, 255, 255]
            } else if x < 20 {
                [0, 0, 0, 0]
            } else {
                [10, 120, 200, 255]
            }
        });
        let pal = auto_palette(&pix, 40, 20, 4);
        assert_eq!(pal, vec![[10, 120, 200]]);
    }

    #[test]
    fn a_gradient_is_not_flat() {
        let pix = blocks(64, 16, |x, _| [(x * 4) as u8, 255 - (x * 4) as u8, 128, 255]);
        let pal = auto_palette(&pix, 64, 16, 6);
        assert!(flat_fraction(&pix, 64, 16, &pal) < 0.93, "gradient scored flat");
    }

    #[test]
    fn nothing_to_find_in_empty_or_blank_images() {
        assert!(auto_palette(&[], 0, 0, 6).is_empty());
        let clear = blocks(10, 10, |_, _| [0, 0, 0, 0]);
        assert!(auto_palette(&clear, 10, 10, 6).is_empty());
        assert_eq!(flat_fraction(&clear, 10, 10, &[]), 0.0);
        let white = blocks(10, 10, |_, _| [255, 255, 255, 255]);
        assert!(auto_palette(&white, 10, 10, 6).is_empty());
    }
}
