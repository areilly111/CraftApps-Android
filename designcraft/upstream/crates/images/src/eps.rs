//! EPS: there's no PostScript interpreter here, so a placed EPS shows (and prints) through a
//! proxy, as InDesign does on screen: the file's TIFF preview when it has one, else a placeholder
//! the size of its bounding box.

/// An EPS file (plain, or DOS EPS with a binary header)?
pub fn is_eps(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6])
        || (bytes.starts_with(b"%!PS-Adobe") && String::from_utf8_lossy(&bytes[..bytes.len().min(64)]).contains("EPSF"))
}

fn u32le(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// The PostScript section and the TIFF preview (DOS EPS).
fn sections(bytes: &[u8]) -> (&[u8], Option<&[u8]>) {
    if bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]) {
        let ps = u32le(bytes, 4).zip(u32le(bytes, 8)).and_then(|(o, l)| bytes.get(o..o.checked_add(l)?)).unwrap_or(&[]);
        let tiff = u32le(bytes, 20).zip(u32le(bytes, 24)).filter(|(o, l)| *o > 0 && *l > 0).and_then(|(o, l)| bytes.get(o..o.checked_add(l)?));
        return (ps, tiff);
    }
    (bytes, None)
}

/// The bounding box in points: (x0, y0, x1, y1), the high-resolution one when given.
pub fn bounding_box(bytes: &[u8]) -> Option<[f64; 4]> {
    let (ps, _) = sections(bytes);
    let head = String::from_utf8_lossy(&ps[..ps.len().min(64 * 1024)]);
    let read = |key: &str| {
        head.lines().find_map(|l| {
            let v: Vec<f64> = l.strip_prefix(key)?.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            (v.len() == 4 && v[2] > v[0] && v[3] > v[1]).then(|| [v[0], v[1], v[2], v[3]])
        })
    };
    read("%%HiResBoundingBox:").or_else(|| read("%%BoundingBox:"))
}

/// The proxy for a placed EPS: (image bytes, its size in points). The TIFF preview when present,
/// else a light grey placeholder with a cross, at 2 pixels per point.
pub fn eps_proxy(bytes: &[u8]) -> Option<(Vec<u8>, (f64, f64))> {
    let bb = bounding_box(bytes)?;
    let (w, h) = (bb[2] - bb[0], bb[3] - bb[1]);
    if let (_, Some(tiff)) = sections(bytes)
        && crate::pixel_size(tiff).is_some()
    {
        return Some((tiff.to_vec(), (w, h)));
    }
    let (pw, ph) = ((w * 2.0).round().clamp(2.0, 4000.0) as u32, (h * 2.0).round().clamp(2.0, 4000.0) as u32);
    let mut img = image::RgbaImage::from_pixel(pw, ph, image::Rgba([228, 228, 228, 255]));
    // Diagonals and a border mark it as a stand-in.
    let n = pw.max(ph);
    for i in 0..n {
        let (x, y) = ((i as f64 / n as f64 * pw as f64) as u32, (i as f64 / n as f64 * ph as f64) as u32);
        for (px, py) in [(x, y), (x, ph - 1 - y.min(ph - 1))] {
            img.put_pixel(px.min(pw - 1), py.min(ph - 1), image::Rgba([150, 150, 150, 255]));
        }
    }
    for x in 0..pw {
        img.put_pixel(x, 0, image::Rgba([150, 150, 150, 255]));
        img.put_pixel(x, ph - 1, image::Rgba([150, 150, 150, 255]));
    }
    for y in 0..ph {
        img.put_pixel(0, y, image::Rgba([150, 150, 150, 255]));
        img.put_pixel(pw - 1, y, image::Rgba([150, 150, 150, 255]));
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some((out, (w, h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eps_bounding_box_and_proxies() {
        let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 50\n%%HiResBoundingBox: 0 0 100.5 50.25\nnewpath\n%%EOF\n";
        assert!(is_eps(eps));
        assert_eq!(bounding_box(eps), Some([0.0, 0.0, 100.5, 50.25]));
        let (png, size) = eps_proxy(eps).unwrap();
        assert_eq!(size, (100.5, 50.25));
        assert_eq!(crate::pixel_size(&png), Some((201, 101)));
        // DOS EPS with a TIFF preview: the preview is the proxy.
        let mut tiff = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(10, 5, image::Rgba([255, 0, 0, 255])))
            .write_to(&mut std::io::Cursor::new(&mut tiff), image::ImageFormat::Tiff)
            .unwrap();
        let mut dos = vec![0xC5, 0xD0, 0xD3, 0xC6];
        let ps_off = 30usize;
        let tiff_off = ps_off + eps.len();
        for v in [ps_off, eps.len(), 0, 0, tiff_off, tiff.len()] {
            dos.extend((v as u32).to_le_bytes());
        }
        dos.extend([0xFF, 0xFF]);
        dos.extend_from_slice(eps);
        dos.extend_from_slice(&tiff);
        assert!(is_eps(&dos));
        let (p, size) = eps_proxy(&dos).unwrap();
        assert_eq!(size, (100.5, 50.25));
        assert_eq!(crate::pixel_size(&p), Some((10, 5)));
        assert!(!is_eps(b"%!PS-Adobe-3.0\nnot encapsulated"));
    }
}
