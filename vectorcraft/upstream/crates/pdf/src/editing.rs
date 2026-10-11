//! Preserve Editing: the native document travels inside the PDF as an embedded file, with a hash
//! of the pages it was written with, so a PDF whose pages another app changed is noticed (and
//! opens as its artwork instead of the stale native document).

use std::sync::Arc;

use hayro_syntax::Pdf;
use hayro_syntax::object::{Array, Dict, Object, Stream, String as PdfString};
use krilla::embed::{AssociationKind, EmbeddedFile, MimeType};
use krilla::metadata::DateTime;

use crate::PdfError;
use crate::lab_spot::find;

/// The embedded file holding the native document (its name among the PDF's attachments).
pub const EDITING_FILE: &str = "vectorcraft-editing.vectorcraft";
/// The editing file's name from before the project was renamed: still read.
pub const LEGACY_EDITING_FILE: &str = "drawcraft-editing.drawcraft";

/// The editing file's description: this text, then the hash of the pages.
const DESCRIPTION: &str = "VectorCraft editing data; page hash ";
/// Stands for the hash while the PDF is written: as long as a hash, so writing the hash over it
/// keeps every offset.
const PLACEHOLDER: &str = "0000000000000000";
/// Deepest name tree searched for the editing file.
const MAX_DEPTH: usize = 16;

/// The native document a PDF carries ([`crate::PdfSettings::preserve_editing`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    /// The native document's bytes (empty when they can't be decoded).
    pub data: Vec<u8>,
    /// The pages are still the ones written with it: the hash of their size and content matches.
    /// False once another app changed them; true for editing data written without a hash.
    pub intact: bool,
}

/// The embedded file for `native`, its description holding the hash placeholder [`seal`] fills.
pub(crate) fn embedded_file(native: &[u8], compress: bool, date: Option<DateTime>) -> EmbeddedFile {
    EmbeddedFile {
        path: EDITING_FILE.into(),
        mime_type: MimeType::new("application/json"),
        description: Some(format!("{DESCRIPTION}{PLACEHOLDER}")),
        association_kind: AssociationKind::Source,
        data: native.to_vec().into(),
        modification_date: date,
        compress: Some(compress),
        location: None,
    }
}

/// Write the hash of the pages of `pdf` (written with an [`embedded_file`]) over the placeholder.
pub(crate) fn seal(pdf: Vec<u8>) -> Result<Vec<u8>, PdfError> {
    let data = Arc::new(pdf);
    let hash = page_hash(&Pdf::new(data.clone()).map_err(|e| PdfError::Write(format!("can't read back the written PDF: {e:?}")))?);
    let mut pdf = Arc::try_unwrap(data).unwrap_or_else(|d| (*d).clone());
    let at = find(&pdf, format!("({DESCRIPTION}{PLACEHOLDER})").as_bytes(), 0).map(|i| i + 1 + DESCRIPTION.len());
    let slot =
        at.and_then(|i| pdf.get_mut(i..i + PLACEHOLDER.len())).ok_or_else(|| PdfError::Write("the editing data's hash has no place".into()))?;
    slot.copy_from_slice(hash.as_bytes());
    Ok(pdf)
}

/// The native document `bytes` (a PDF) carries, if any, and whether its pages changed since.
pub fn editing(bytes: &[u8]) -> Option<Editing> {
    editing_with(bytes, None)
}

/// [`editing`] of a PDF that may be encrypted, opened with `password` (its open or permissions
/// password).
pub fn editing_with(bytes: &[u8], password: Option<&str>) -> Option<Editing> {
    editing_in(&crate::pages::open(bytes, password).ok()?)
}

/// [`editing`] of a parsed PDF.
pub(crate) fn editing_in(pdf: &Pdf) -> Option<Editing> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
    let tree = catalog.get::<Dict<'_>>(b"Names")?.get::<Dict<'_>>(b"EmbeddedFiles")?;
    let spec = editing_spec(&tree, 0)?;
    let files = spec.get::<Dict<'_>>(b"EF")?;
    let stream = files.get::<Stream<'_>>(b"F").or_else(|| files.get::<Stream<'_>>(b"UF"))?;
    let data = stream.decoded().map(|d| d.into_owned()).unwrap_or_default();
    // The hash ends the description; editing data described otherwise is trusted.
    let desc = spec.get::<PdfString<'_>>(b"Desc").map(|d| String::from_utf8_lossy(d.as_bytes()).into_owned()).unwrap_or_default();
    let intact = match desc.rsplit_once("hash ") {
        Some((_, hash)) => hash.trim() == page_hash(pdf),
        None => true,
    };
    Some(Editing { data, intact })
}

/// The file specification of the editing file in the name tree node `node`.
fn editing_spec<'a>(node: &Dict<'a>, depth: usize) -> Option<Dict<'a>> {
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        let mut items = names.iter::<Object<'a>>();
        while let (Some(_), Some(spec)) = (items.next(), items.next()) {
            if let Object::Dict(spec) = spec
                && is_editing_file(&spec)
            {
                return Some(spec);
            }
        }
    }
    if depth >= MAX_DEPTH {
        return None;
    }
    node.get::<Array<'a>>(b"Kids")?.iter::<Dict<'a>>().find_map(|kid| editing_spec(&kid, depth + 1))
}

/// A file specification naming the editing file (current or legacy name).
fn is_editing_file(spec: &Dict<'_>) -> bool {
    [&b"UF"[..], b"F"].into_iter().filter_map(|k| spec.get::<PdfString<'_>>(k)).any(|name| {
        let name = name.as_bytes();
        name == EDITING_FILE.as_bytes() || name == LEGACY_EDITING_FILE.as_bytes()
    })
}

/// FNV-1a of the pages' media boxes and decoded content, as 16 hex digits.
fn page_hash(pdf: &Pdf) -> String {
    let fnv = |h: u64, bytes: &[u8]| bytes.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3));
    let mut h = 0xcbf29ce484222325;
    for page in pdf.pages().iter() {
        let b = page.media_box();
        h = fnv(h, format!("[{} {} {} {}]", b.x0, b.y0, b.x1, b.y1).as_bytes());
        h = fnv(h, page.page_stream().unwrap_or_default());
    }
    format!("{h:016x}")
}
