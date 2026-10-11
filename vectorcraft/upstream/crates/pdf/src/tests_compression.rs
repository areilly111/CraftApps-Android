//! Image compression and downsampling: images above the threshold resolution are resampled to the
//! target, compressed with JPEG or ZIP as their kind (colour, greyscale, monochrome) asks, and the
//! codecs the writer lacks are written as ZIP with a warning.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;

use crate::*;

fn encoded(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

/// A 600 × 600 colour photo-like image.
fn colour() -> DynamicImage {
    image::RgbImage::from_fn(600, 600, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x * y) % 251) as u8])).into()
}

/// A document with image `bytes` (600 × 600 pixels) placed one inch wide: 600 ppi.
fn doc(bytes: Vec<u8>, mime: &str) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("img".into(), ImageBlob::new(mime, bytes));
    let xf = Affine::scale(72.0 / 600.0);
    let n = Node::new(
        d.alloc_id(),
        NodeKind::Image(ImageObject { key: "img".into(), width: 600, height: 600, xf, link: None, placement: Default::default() }),
    );
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The PDF of `d` with Compression options `compression` (content streams left readable) and its
/// warnings.
fn pdf(d: &Document, compression: Value) -> (String, Vec<String>) {
    let mut settings: PdfSettings = serde_json::from_value(json!({ "compression": compression })).unwrap();
    settings.compression.compress_text = false;
    let r = export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap();
    (String::from_utf8_lossy(&r.bytes).into_owned(), r.warnings)
}

#[test]
fn images_above_the_threshold_are_resampled_to_the_target_resolution() {
    let d = doc(encoded(colour(), ImageFormat::Png), "image/png");
    for method in ["average", "subsample", "bicubic"] {
        let (text, warnings) = pdf(&d, json!({"color": {"downsample": method, "ppi": 150, "abovePpi": 225}}));
        assert!(text.contains("/Width 150") && text.contains("/Height 150"), "{method}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }
    // Below the threshold, or without downsampling, the image keeps its pixels.
    for c in [json!({"color": {"downsample": "bicubic", "ppi": 150, "abovePpi": 700}}), json!({})] {
        assert!(pdf(&d, c.clone()).0.contains("/Width 600"), "{c}");
    }
    // Greyscale images take the greyscale settings.
    let grey = doc(encoded(colour().grayscale(), ImageFormat::Png), "image/png");
    let (text, _) = pdf(&grey, json!({"color": {"downsample": "bicubic", "ppi": 150}, "gray": {"downsample": "bicubic", "ppi": 100}}));
    assert!(text.contains("/Width 100") && text.contains("/DeviceGray"), "grey at 100 ppi");
}

#[test]
fn jpeg_zip_and_automatic_compression() {
    let png = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let jpeg = doc(encoded(colour(), ImageFormat::Jpeg), "image/jpeg");
    let dct = |d: &Document, c: Value| pdf(d, c).0.contains("/DCTDecode");
    assert!(dct(&png, json!({"color": {"compression": "jpeg", "quality": "low"}})), "JPEG compression");
    assert!(!dct(&png, json!({"color": {"compression": "zip"}})) && pdf(&png, json!({})).0.contains("/FlateDecode"), "ZIP");
    assert!(!dct(&png, json!({})), "automatic: lossless for a PNG");
    assert!(dct(&jpeg, json!({})) && dct(&jpeg, json!({"color": {"downsample": "average", "ppi": 150}})), "automatic: a JPEG stays JPEG");
    assert!(!dct(&jpeg, json!({"color": {"compression": "zip"}})), "ZIP re-encodes a JPEG losslessly");
    // JPEG has no transparency: an image with some stays lossless.
    let mut clear = colour().to_rgba8();
    clear.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
    let clear = doc(encoded(clear.into(), ImageFormat::Png), "image/png");
    let (text, _) = pdf(&clear, json!({"color": {"compression": "jpeg"}}));
    assert!(!text.contains("/DCTDecode") && text.contains("/SMask"));
}

#[test]
fn codecs_the_writer_lacks_are_zip_with_a_warning() {
    let png = doc(encoded(colour(), ImageFormat::Png), "image/png");
    let (text, warnings) = pdf(&png, json!({"color": {"compression": "jpeg2000"}}));
    assert!(!text.contains("/JPXDecode") && text.contains("/FlateDecode"));
    assert!(warnings.len() == 1 && warnings[0].contains("JPEG 2000"), "{warnings:?}");
    // Black-and-white images are monochrome: their settings apply, CCITT becomes ZIP.
    let bw = image::GrayImage::from_fn(600, 600, |x, y| image::Luma([if (x / 7 + y / 5) % 2 == 0 { 0 } else { 255 }]));
    let bw = doc(encoded(bw.into(), ImageFormat::Png), "image/png");
    let (text, warnings) = pdf(&bw, json!({"mono": {"downsample": "bicubic", "ppi": 300, "abovePpi": 450, "compression": "ccittG4"}}));
    assert!(text.contains("/Width 300") && !text.contains("/CCITTFaxDecode"));
    assert!(warnings.len() == 1 && warnings[0].contains("CCITT"), "{warnings:?}");
    // The codecs of other kinds of images say nothing.
    assert!(pdf(&png, json!({"mono": {"compression": "ccittG4"}})).1.is_empty());
}
