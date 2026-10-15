//! Krita .kra import: the merged PNG is an editable raster layer, not a replacement
//! for Krita's native layer tree. Never write this document back to the .kra source.
//! The ZIP reader checks entry bounds, sizes and CRC before the PNG decoder runs.

use photocraft_codecs::{self as codecs, DecodeOptions, Format};
use photocraft_format::zip::ZipReader;
use photocraft_raster::Interrupt;

use crate::{ImportResult, IoError};

const MIMETYPE: &[u8] = b"application/x-krita";
const MAX_ENTRY: usize = 512 << 20;
const MAX_PIXELS_BYTES: u64 = 1 << 30;

fn invalid(message: impl Into<String>) -> IoError {
    IoError::Kra(message.into())
}

/// Krita writes its mimetype as the first, uncompressed ZIP entry.
pub(crate) fn is_kra(bytes: &[u8]) -> bool {
    crate::ora::zip_mimetype(bytes).is_some_and(|mime| mime.trim_ascii() == MIMETYPE)
}

/// Import Krita's official merged preview as a flattened, editable PhotoCraft document.
/// Do not silently pretend this imported a layered or lossless Krita document.
pub(crate) fn import(name: &str, bytes: &[u8], ctl: &Interrupt<'_>) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let archive = ZipReader::new(bytes).map_err(|e| invalid(e.to_string()))?;
    let mime = archive.read_by_name("mimetype", 256).map_err(|e| invalid(e.to_string()))?;
    if mime.trim_ascii() != MIMETYPE {
        return Err(invalid("the archive's mimetype is not application/x-krita"));
    }
    ctl.progress(0.10);
    let merged = archive.read_by_name("mergedimage.png", MAX_ENTRY).map_err(|e| invalid(format!("mergedimage.png: {e}")))?;
    if codecs::detect(&merged) != Some(Format::Png) {
        return Err(invalid("mergedimage.png is not a PNG image"));
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut opts = DecodeOptions::default();
    opts.limits.max_width = crate::ora::MAX_SIDE;
    opts.limits.max_height = crate::ora::MAX_SIDE;
    opts.limits.max_alloc = MAX_PIXELS_BYTES;
    let image = codecs::decode_with(&merged, &opts).map_err(|e| invalid(format!("mergedimage.png: {e}")))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let mut result = crate::flat::image_to_document(name, &image)?;
    result.source_read_only = true;
    result.warnings.push("Krita layers, masks and editable document structure were not imported; only mergedimage.png is available".into());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Image, SampleType};
    use photocraft_format::zip::ZipWriter;

    fn png() -> Vec<u8> {
        let image = Image::from_raw(2, 1, ChannelLayout::Rgba, SampleType::U8, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
    }

    fn archive(mime: &[u8], with_merged: bool) -> Vec<u8> {
        let mut zip = ZipWriter::new();
        zip.add("mimetype", mime).unwrap();
        zip.add("maindoc.xml", b"<DOC/>").unwrap();
        if with_merged {
            zip.add("mergedimage.png", &png()).unwrap();
        }
        zip.finish().unwrap()
    }

    #[test]
    fn imports_krita_composite_without_claiming_to_preserve_layers() {
        let bytes = archive(MIMETYPE, true);
        assert!(is_kra(&bytes));
        // The mimetype is authoritative even without a .kra extension.
        for name in ["scene.kra", "download"] {
            let r = crate::import(name, &bytes).unwrap();
            assert_eq!((r.document.size.width, r.document.size.height), (2, 1));
            assert_eq!(r.document.name, name);
            assert_eq!(r.document.layers.len(), 1);
            assert_eq!(r.document.layers[0].name, "Layer 0", "alpha-bearing composite");
            let pixels = r.document.layers[0].surface().unwrap();
            assert!(pixels.rgba(0, 0)[0] > 0.99);
            assert!(pixels.rgba(1, 0)[2] > 0.99);
            assert!((pixels.rgba(1, 0)[3] - 128.0 / 255.0).abs() < 0.01);
            assert!(r.source_read_only, ".kra must not be overwritten by a flat save");
            assert!(!r.preview_only, "the flattened image is editable");
            assert!(r.warnings.iter().any(|w| w.contains("layers") && w.contains("not imported")));
        }
    }

    #[test]
    fn malformed_or_non_krita_archives_do_not_open_as_krita() {
        let wrong = archive(b"image/openraster", true);
        assert!(!is_kra(&wrong));
        assert!(import("wrong.kra", &wrong, &Interrupt::NONE).is_err());
        assert!(import("missing.kra", &archive(MIMETYPE, false), &Interrupt::NONE).is_err());
        assert!(import("broken.kra", b"not a zip", &Interrupt::NONE).is_err());
    }

    #[test]
    fn a_png_with_a_misleading_kra_extension_remains_a_png() {
        let r = crate::import("renamed.kra", &png()).unwrap();
        assert_eq!(r.document.size.width, 2);
        assert!(!r.source_read_only, "a real PNG must not be treated as a Krita source");
    }
}
