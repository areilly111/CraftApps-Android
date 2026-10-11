//! Pixel access for embedded images: a stable content key and recolouring the pixels.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use crate::ImageBlob;

impl ImageBlob {
    /// A PNG blob of `png` bytes.
    pub fn png(png: Vec<u8>) -> Self {
        Self::new("image/png", png)
    }

    /// A key derived from the bytes (FNV-1a), the same for identical images.
    pub fn content_key(&self) -> String {
        crate::links::hash_bytes(&self.bytes)
    }

    /// The image with `f` applied to each pixel's (straight) RGB, alpha kept and fully transparent
    /// pixels left alone, re-encoded as PNG. `f` runs once per distinct colour. `None` when the
    /// image can't be decoded or no pixel changed.
    pub fn map_rgb(&self, mut f: impl FnMut([u8; 3]) -> [u8; 3]) -> Option<ImageBlob> {
        let mut img = image::load_from_memory(&self.bytes).ok()?.to_rgba8();
        let mut memo: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
        let mut changed = false;
        for px in img.pixels_mut().filter(|p| p.0[3] > 0) {
            let rgb = [px.0[0], px.0[1], px.0[2]];
            let out = *memo.entry(rgb).or_insert_with(|| f(rgb));
            if out != rgb {
                px.0[..3].copy_from_slice(&out);
                changed = true;
            }
        }
        if !changed {
            return None;
        }
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(Self::png(png))
    }
}

impl ImageBlob {
    /// The colour around pixel (`x`, `y`) (pixel space): the alpha-weighted average of the `size` ×
    /// `size` pixels centred on it, clipped to the image, as straight RGBA. `None` outside the
    /// image, where every pixel is transparent, or when the image can't be decoded. The last image
    /// decoded is kept for the next sample (the Eyedropper samples one image click after click).
    pub fn sample(&self, x: f64, y: f64, size: u32) -> Option<[u8; 4]> {
        sample_pixels(&*self.decoded()?, x, y, size)
    }

    /// [`Self::sample`] at (`x`, `y`) in the pixel space of an image object of `object` (width,
    /// height) pixels, which the decoded pixels may not match (a linked image showing its preview).
    pub fn sample_object(&self, x: f64, y: f64, object: (u32, u32), size: u32) -> Option<[u8; 4]> {
        let img = self.decoded()?;
        let k = |n: u32, of: u32| n as f64 / of.max(1) as f64;
        sample_pixels(&img, x * k(img.width(), object.0), y * k(img.height(), object.1), size)
    }

    /// The decoded pixels (the last image decoded is kept for the next sample).
    fn decoded(&self) -> Option<Arc<image::RgbaImage>> {
        thread_local! {
            static LAST: std::cell::RefCell<Option<(String, Arc<image::RgbaImage>)>> = const { std::cell::RefCell::new(None) };
        }
        let key = self.content_key();
        LAST.with_borrow_mut(|last| match last {
            Some((k, img)) if *k == key => Some(img.clone()),
            _ => {
                let img = Arc::new(image::load_from_memory(&self.bytes).ok()?.to_rgba8());
                *last = Some((key, img.clone()));
                Some(img)
            }
        })
    }
}

/// [`ImageBlob::sample`] of decoded pixels `img`.
fn sample_pixels(img: &image::RgbaImage, x: f64, y: f64, size: u32) -> Option<[u8; 4]> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (cx, cy) = (x.floor() as i64, y.floor() as i64);
    if !(0..w).contains(&cx) || !(0..h).contains(&cy) {
        return None;
    }
    let r = size.max(1) as i64 / 2;
    let (mut sum, mut alpha, mut n) = ([0u64; 3], 0u64, 0u64);
    for py in (cy - r).max(0)..=(cy + r).min(h - 1) {
        for px in (cx - r).max(0)..=(cx + r).min(w - 1) {
            let [pr, pg, pb, pa] = img.get_pixel(px as u32, py as u32).0;
            let a = pa as u64;
            sum = [sum[0] + pr as u64 * a, sum[1] + pg as u64 * a, sum[2] + pb as u64 * a];
            alpha += a;
            n += 1;
        }
    }
    if alpha == 0 {
        return None;
    }
    let c = |v: u64| ((v + alpha / 2) / alpha) as u8;
    Some([c(sum[0]), c(sum[1]), c(sum[2]), ((alpha + n / 2) / n) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(px: &[[u8; 4]], w: u32, h: u32) -> ImageBlob {
        let img = image::RgbaImage::from_raw(w, h, px.concat()).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        ImageBlob::png(png)
    }

    fn pixels(b: &ImageBlob) -> Vec<[u8; 4]> {
        image::load_from_memory(&b.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
    }

    #[test]
    fn samples_a_pixel_or_the_average_around_it() {
        let b = blob(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 0]], 4, 1);
        assert_eq!(b.sample(1.5, 0.5, 1), Some([0, 255, 0, 255]));
        assert_eq!(b.sample(1.5, 0.5, 3), Some([85, 85, 85, 255]), "red, green and blue averaged");
        assert_eq!(b.sample(3.2, 0.5, 3), Some([0, 0, 255, 128]), "transparent pixels don't count towards the colour");
        assert_eq!(b.sample(3.2, 0.5, 1), None, "all transparent");
        assert_eq!(b.sample(-0.5, 0.5, 5), None, "outside");
        assert_eq!(b.sample(0.0, 0.0, 5), Some([85, 85, 85, 255]), "clipped to the image");
    }

    #[test]
    fn samples_in_the_pixel_space_of_an_object_with_more_pixels() {
        // An object of 8 × 2 pixels showing these 4 × 1 (a linked image's preview).
        let b = blob(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [9, 9, 9, 255]], 4, 1);
        assert_eq!(b.sample_object(5.0, 1.5, (8, 2), 1), Some([0, 0, 255, 255]));
        assert_eq!(b.sample_object(1.0, 0.0, (4, 1), 1), b.sample(1.0, 0.0, 1), "the same size: the same pixels");
    }

    #[test]
    fn maps_each_colour_once_and_keeps_alpha() {
        let b = blob(&[[255, 0, 0, 255], [255, 0, 0, 128], [0, 0, 255, 255], [10, 20, 30, 0]], 2, 2);
        let mut calls = 0;
        let out = b
            .map_rgb(|[r, g, bl]| {
                calls += 1;
                [255 - r, 255 - g, 255 - bl]
            })
            .unwrap();
        assert_eq!(calls, 2, "red and blue, once each; the transparent pixel is skipped");
        assert_eq!(pixels(&out), [[0, 255, 255, 255], [0, 255, 255, 128], [255, 255, 0, 255], [10, 20, 30, 0]]);
        assert_ne!(out.content_key(), b.content_key());
        assert_eq!(b.content_key(), blob(&[[255, 0, 0, 255], [255, 0, 0, 128], [0, 0, 255, 255], [10, 20, 30, 0]], 2, 2).content_key());
        assert!(b.map_rgb(|c| c).is_none(), "nothing changed");
        assert!(ImageBlob::png(vec![1, 2, 3]).map_rgb(|c| c).is_none(), "undecodable");
    }
}
