//! QR codes as vector paths (Object › Generate QR Code).

use kurbo::Rect;

use crate::Point;
use crate::path::{PathData, SubPath};

/// Error correction as InDesign uses for print: medium (15%).
const ECC: qrcodegen::QrCodeEcc = qrcodegen::QrCodeEcc::Medium;

/// The dark modules of a QR code for `text`, fitted into `r` (square, centred; a quiet zone of
/// two modules each side), merged into one rectangle per horizontal run. `None` if the text is
/// too long for any QR version.
pub fn qr_path(text: &str, r: Rect) -> Option<PathData> {
    let qr = qrcodegen::QrCode::encode_text(text, ECC).ok()?;
    let n = qr.size();
    let quiet = 2;
    let side = r.width().min(r.height());
    let m = side / (n + 2 * quiet) as f64;
    let origin = Point::new(r.center().x - side / 2.0 + quiet as f64 * m, r.center().y - side / 2.0 + quiet as f64 * m);
    let mut subpaths = Vec::new();
    for y in 0..n {
        let mut x = 0;
        while x < n {
            if !qr.get_module(x, y) {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < n && qr.get_module(x, y) {
                x += 1;
            }
            let (a, b) = (origin.x + x0 as f64 * m, origin.x + x as f64 * m);
            let (t, u) = (origin.y + y as f64 * m, origin.y + (y + 1) as f64 * m);
            subpaths.push(SubPath::polyline(&[Point::new(a, t), Point::new(b, t), Point::new(b, u), Point::new(a, u)], true));
        }
    }
    Some(PathData::new(subpaths))
}

/// The modules per side for `text` (the version's size), if it can be encoded.
pub fn qr_size(text: &str) -> Option<i32> {
    qrcodegen::QrCode::encode_text(text, ECC).ok().map(|q| q.size())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_modules_fill_the_square() {
        let p = qr_path("https://example.com", Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let n = qr_size("https://example.com").unwrap();
        assert!(n >= 21 && (n - 17) % 4 == 0, "a QR version size: {n}");
        let b = p.bounds().unwrap();
        let m = 100.0 / (n + 4) as f64;
        // Finder patterns put dark modules at the corners of the symbol.
        assert!((b.x0 - 2.0 * m).abs() < 1e-9 && (b.y0 - 2.0 * m).abs() < 1e-9);
        assert!((b.x1 - (100.0 - 2.0 * m)).abs() < 1e-9);
        assert!(p.subpaths.len() > 20);
        assert!(qr_path(&"x".repeat(5000), Rect::new(0.0, 0.0, 10.0, 10.0)).is_none());
    }
}
