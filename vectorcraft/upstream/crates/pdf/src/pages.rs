//! The pages of a PDF to import: opening with a password, the pages picked, the box each page
//! is cropped to ([`CropTo`]) and [`info`].

use hayro_syntax::object::{Dict, Stream};
use hayro_syntax::page::Page;
use hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use kurbo::{Affine, Rect};

use crate::{Choice, CropTo, ImportOptions, PdfError};

/// What `info` tells about one page.
#[derive(Clone, Debug, PartialEq)]
pub struct PageInfo {
    /// The page's size in points as viewers show it (its crop box, rotated).
    pub width: f64,
    pub height: f64,
    /// The page's rotation in degrees (0, 90, 180 or 270).
    pub rotation: u16,
    /// Every box but Bounding, in PDF user space (y up), clipped to the media box. A missing
    /// bleed, trim or art box is the crop box.
    pub boxes: Vec<(CropTo, Rect)>,
}

/// The pages of a PDF ([`info`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfInfo {
    pub pages: Vec<PageInfo>,
}

/// Is this a PostScript file (plain, or an EPS with a binary header) rather than a PDF?
pub fn is_postscript(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%!PS") || bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6])
}

/// Read a PDF, decrypting it with `password` (none: the empty user password). The permissions
/// password opens a file too.
pub(crate) fn open(bytes: &[u8], password: Option<&str>) -> Result<Pdf, PdfError> {
    if is_postscript(bytes) {
        return Err(PdfError::PostScript);
    }
    let password = password.unwrap_or_default();
    let read = |pw: &str| Pdf::new_with_password(bytes.to_vec(), pw);
    read(password)
        .or_else(|e| match e {
            // The reader takes only the open password of RC4 and 128-bit AES files: the
            // permissions password gives it.
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) if !password.is_empty() => {
                crate::encrypt::user_password(bytes, password).map_or(Err(e), |user| read(&user))
            }
            e => Err(e),
        })
        .map_err(|e| match e {
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) if password.is_empty() => PdfError::NeedsPassword,
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => PdfError::WrongPassword,
            e => PdfError::Parse(format!("{e:?}")),
        })
}

/// The pages and their boxes. An encrypted PDF needs its `password`
/// ([`PdfError::NeedsPassword`] / [`PdfError::WrongPassword`]).
pub fn info(bytes: &[u8], password: Option<&str>) -> Result<PdfInfo, PdfError> {
    let pdf = open(bytes, password)?;
    let pages = pdf
        .pages()
        .iter()
        .map(|page| {
            let (w, h) = page.render_dimensions();
            let rotation = match page.rotation() {
                hayro_syntax::page::Rotation::None => 0,
                hayro_syntax::page::Rotation::Horizontal => 90,
                hayro_syntax::page::Rotation::Flipped => 180,
                hayro_syntax::page::Rotation::FlippedHorizontal => 270,
            };
            let boxes = CropTo::ALL.iter().filter(|c| **c != CropTo::Bounding).map(|c| (*c, page_box(page, *c))).collect();
            PageInfo { width: w as f64, height: h as f64, rotation, boxes }
        })
        .collect();
    Ok(PdfInfo { pages })
}

/// The 0-based pages `opts` pick out of `count`.
pub(crate) fn picked(opts: &ImportOptions, count: usize) -> Result<Vec<usize>, PdfError> {
    let mut v = match &opts.pages {
        Some(p) => {
            if let Some(bad) = p.iter().find(|i| **i >= count) {
                return Err(PdfError::BadPage(bad.saturating_add(1), count));
            }
            p.clone()
        }
        None => (0..count).collect(),
    };
    if let Some(m) = opts.max_pages {
        v.truncate(m);
    }
    if v.is_empty() {
        return Err(PdfError::NoPages);
    }
    Ok(v)
}

fn rect(r: hayro_syntax::object::Rect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1).abs()
}

/// Box `which` of `page` in PDF user space, clipped to the media box (Bounding: the crop box,
/// which the art's bounds replace after import). A missing or empty box is the crop box.
pub(crate) fn page_box(page: &Page<'_>, which: CropTo) -> Rect {
    let media = rect(page.media_box());
    let crop = rect(page.intersected_crop_box());
    let key: &[u8] = match which {
        CropTo::Bounding | CropTo::Crop => return crop,
        CropTo::Media if media.area() > 0.0 => return media,
        CropTo::Media => return crop,
        CropTo::Bleed => b"BleedBox",
        CropTo::Trim => b"TrimBox",
        CropTo::Art => b"ArtBox",
    };
    page.raw().get::<hayro_syntax::object::Rect>(key).map(|r| rect(r).intersect(media)).filter(|r| r.area() > 0.0).unwrap_or(crop)
}

/// Where `page` draws on its artboard: the page transform (PDF user space → y-down page space
/// with the crop box at the origin, rotation applied) and `which` box in that space (a page with
/// no area gets the size viewers give it).
pub(crate) fn frame(page: &Page<'_>, which: CropTo) -> (Affine, Rect) {
    let init = Affine::new(page.initial_transform(true).as_coeffs());
    let b = init.transform_rect_bbox(page_box(page, which));
    let (w, h) = page.render_dimensions();
    (init, if b.is_finite() && b.area() > 0.0 { b } else { Rect::new(0.0, 0.0, w as f64, h as f64) })
}

/// The `PieceInfo` key such files keep their private data under.
const PRIVATE_DATA_OWNER: &[u8] = b"Illustrator"; // brand-ok: the key the files use

/// Most bytes of an editor's private data read.
const MAX_PRIVATE: usize = 256 << 20;

/// The private data of an Illustrator `.ai` (its first page's `PieceInfo`): the `AIPrivateData`
/// streams in order, joined. `None` for a PDF without them (or with a stream that can't be read).
/// What it holds is the editor's own copy of the art (see `vectorcraft_eps::layered_ai`).
pub fn illustrator_data(bytes: &[u8], password: Option<&str>) -> Option<Vec<u8>> {
    let pdf = open(bytes, password).ok()?;
    let page = pdf.pages().first()?;
    let private = page.raw().get::<Dict<'_>>(b"PieceInfo")?.get::<Dict<'_>>(PRIVATE_DATA_OWNER)?.get::<Dict<'_>>(b"Private")?;
    let mut data = vec![];
    for n in 1..=100_000u32 {
        let Some(stream) = private.get::<Stream<'_>>(format!("AIPrivateData{n}").as_bytes()) else { break };
        data.extend_from_slice(&inflated(&stream, MAX_PRIVATE - data.len())?);
    }
    (!data.is_empty()).then_some(data)
}

/// `stream` decoded, up to `most` bytes: the decoder is run here rather than by the PDF reader's
/// `decoded()`, which has no limit, so a stream of zeros can't grow to the size of the memory. Only a
/// stream that is plain or only deflated is read (the private data is no other); else, or when it
/// is larger once decoded, `None`.
fn inflated(stream: &Stream<'_>, most: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    if stream.dict().get::<Dict<'_>>(b"DecodeParms").is_some() {
        return None;
    }
    let raw = stream.raw_data();
    let mut out = vec![];
    let limit = u64::try_from(most).ok()?.checked_add(1)?;
    match stream.filters().len() {
        0 => out.extend_from_slice(&raw),
        1 => {
            flate2::read::ZlibDecoder::new(&*raw).take(limit).read_to_end(&mut out).ok()?;
        }
        _ => return None,
    }
    (out.len() <= most).then_some(out)
}

/// Does `page` carry an editor's private data (`PieceInfo` keys starting with `AIPrivateData`)?
/// A file saved without PDF compatibility keeps its art there and shows a placeholder page.
pub(crate) fn has_private_data(page: &Page<'_>) -> bool {
    fn walk(d: &Dict<'_>, depth: u32) -> bool {
        depth < 4 && d.keys().any(|k| k.starts_with(b"AIPrivateData") || d.get::<Dict<'_>>(k.as_ref()).is_some_and(|sub| walk(&sub, depth + 1)))
    }
    page.raw().get::<Dict<'_>>(b"PieceInfo").is_some_and(|d| walk(&d, 0))
}
