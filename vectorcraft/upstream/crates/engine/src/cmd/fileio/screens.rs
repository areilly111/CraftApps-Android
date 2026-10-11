//! File → Export → Export for Screens (`document.exportForScreens`): the chosen artboards (or the
//! whole document, or the chosen assets of the Asset Export panel) in every format row at each
//! row's size, into a folder and its sub-folders, or returned for download (each file, or one ZIP);
//! the presets the dialog offers; and the settings the document remembers
//! (`document.exportSettings`), which the Asset Export panel shares ([`SHARED_KEYS`]).

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;
use vectorcraft_geom::Rect;

use super::super::*;
use super::encode::single_artboard;
use super::export::{isolated, without_artboards};
use super::{ARTBOARD_PARAMS, ArtboardPick, Format, artboard_file_names, create_dir, encode, unique_file_names, writable, write_file};

const C: &str = "document.exportForScreens";

/// How big a raster row's files are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenSize {
    /// Pixels per point (`2x`).
    Scale(f64),
    /// Pixels per inch, stored in the file (`144ppi`: twice 72, so `2x`).
    Ppi(f64),
    /// The width in pixels; the height follows the artboard's proportions (`100w`).
    Width(f64),
    /// The height in pixels (`100h`).
    Height(f64),
}

impl ScreenSize {
    /// `2x` (or `2`), `100w`, `100h` or `72ppi`, in any case; positive numbers only.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let num = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
        if let Some(n) = s.strip_suffix("ppi") {
            return num(n).map(Self::Ppi);
        }
        if let Some(n) = s.strip_suffix('w') {
            return num(n).map(Self::Width);
        }
        if let Some(n) = s.strip_suffix('h') {
            return num(n).map(Self::Height);
        }
        num(s.strip_suffix('x').unwrap_or(&s)).map(Self::Scale)
    }

    /// The size a format row asks for: `width`, else `height`, else `ppi`, else `scale` (a number,
    /// or text such as `"2x"`, `"100w"`, `"72ppi"`); 1x when it names none.
    fn of_row(row: &Value) -> Result<Self> {
        let positive = |key: &str, make: fn(f64) -> Self| -> Option<Result<Self>> {
            let v = row.get(key).filter(|v| !v.is_null())?;
            Some(
                v.as_f64().filter(|n| n.is_finite() && *n > 0.0).map(make).ok_or_else(|| bad(C, format!("{key} must be a positive number, not {v}"))),
            )
        };
        if let Some(r) = positive("width", Self::Width).or_else(|| positive("height", Self::Height)).or_else(|| positive("ppi", Self::Ppi)) {
            return r;
        }
        match row.get("scale") {
            None | Some(Value::Null) => Ok(Self::Scale(1.0)),
            Some(Value::String(s)) => Self::parse(s)
                .ok_or_else(|| bad(C, format!("scale `{s}`: a factor such as 2x, a width (100w), a height (100h) or a resolution (72ppi)"))),
            Some(_) => positive("scale", Self::Scale).unwrap_or(Ok(Self::Scale(1.0))),
        }
    }

    /// The scale factor, for the sizes that have one of their own.
    fn factor(self) -> Option<f64> {
        match self {
            Self::Scale(s) => Some(s),
            Self::Ppi(p) => Some(p / 72.0),
            Self::Width(_) | Self::Height(_) => None,
        }
    }

    /// Pixels per point for `region`.
    fn scale_for(self, region: Rect) -> Result<f64> {
        let per = |px: f64, pt: f64| (pt > 0.0).then(|| px / pt).ok_or_else(|| bad(C, "an empty artboard has no width or height to scale to"));
        match self {
            Self::Width(w) => per(w, region.width()),
            Self::Height(h) => per(h, region.height()),
            _ => Ok(self.factor().unwrap_or(1.0)),
        }
    }

    /// The sub-folder files of this size go into: `1x`, `2x`, `100w`, `100h` (numbers to 3
    /// decimals: 80ppi is `1.111x`).
    pub fn label(self) -> String {
        let n = |v: f64| (v * 1000.0).round() / 1000.0;
        match self {
            Self::Width(w) => format!("{}w", n(w)),
            Self::Height(h) => format!("{}h", n(h)),
            _ => format!("{}x", n(self.factor().unwrap_or(1.0))),
        }
    }

    /// The file-name suffix by default: none at 1x, else `@2x`, `@100w`, `@100h` (a resolution
    /// names its scale: 144ppi is `@2x`).
    pub fn suffix(self) -> String {
        match self.factor() {
            Some(s) if (s - 1.0).abs() < 1e-9 => String::new(),
            _ => format!("@{}", self.label()),
        }
    }
}

/// A file-name suffix that names a pixel size (`@2x`, `@0.5x`, `@100w`, `@100h`): vector rows
/// leave it out.
fn is_size_suffix(s: &str) -> bool {
    s.strip_prefix('@').is_some_and(|t| t.ends_with(|c: char| c.is_ascii_alphabetic()) && ScreenSize::parse(t).is_some())
}

/// The presets of the dialog's format rows: (id, label).
pub const PRESETS: [(&str, &str); 2] = [("mobile", "Mobile 1x/2x/3x"), ("density", "Density buckets (0.75x–4x)")];

/// Screen density buckets: (sub-folder, scale).
const DENSITY: [(&str, f64); 6] = [("ldpi", 0.75), ("mdpi", 1.0), ("hdpi", 1.5), ("xhdpi", 2.0), ("xxhdpi", 3.0), ("xxxhdpi", 4.0)];

/// The format rows of preset `id` (see [`PRESETS`]): PNG at 1x, 2x and 3x (`@2x`, `@3x`), or PNG
/// at every density bucket, each in its bucket's sub-folder under the same name.
pub fn preset_rows(id: &str) -> Option<Vec<Value>> {
    let png = |s: f64| ScreenSize::Scale(s);
    match id {
        "mobile" => Some([1.0, 2.0, 3.0].iter().map(|s| json!({"format": "png", "scale": png(*s).label(), "suffix": png(*s).suffix()})).collect()),
        "density" => {
            Some(DENSITY.iter().map(|(folder, s)| json!({"format": "png", "scale": png(*s).label(), "suffix": "", "folder": folder})).collect())
        }
        _ => None,
    }
}

/// A name safe as one file or folder name: letters, digits, `-`, `_` and `.` kept, anything else
/// `-` (`None` when nothing but dots is left: `.` and `..` name other folders).
fn safe_name(s: &str) -> Option<String> {
    let n: String = s.trim().chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect();
    (!n.trim_matches('.').is_empty()).then_some(n)
}

/// Keys of a format row that aren't encoder options.
const ROW_KEYS: [&str; 7] = ["format", "scale", "ppi", "width", "height", "suffix", "folder"];

/// One format row: what to write, at which size, with which suffix, into which sub-folder.
struct Row {
    format: &'static Format,
    size: ScreenSize,
    suffix: String,
    /// The sub-folder with Create Sub-folders: the row's `folder`, else its size (raster) or format.
    folder: String,
    /// The encoder options: the format's settings, then the row's own.
    options: Value,
}

impl Row {
    fn parse(s: &Session, row: &Value, settings: &Value) -> Result<Self> {
        if !row.is_object() {
            return Err(bad(C, "each format is an object {format, scale?, suffix?}"));
        }
        let format = writable(C, Some(str_param(row, "format").unwrap_or("png")), None)?;
        if matches!(format.id, "vectorcraft" | "template" | "ai" | "txt" | "dxf" | "eps" | "emf" | "wmf") {
            return Err(bad(C, "Export for Screens writes png, png8, jpg, webp, gif, svg, svgz or pdf"));
        }
        // Vector formats have no pixel size: it doesn't apply and adds no @2x suffix (not even one
        // left over from a raster row switched to SVG or PDF).
        let size = if format.raster { ScreenSize::of_row(row)? } else { ScreenSize::Scale(1.0) };
        let suffix = match str_param(row, "suffix") {
            Some(s) if !format.raster && is_size_suffix(s) => String::new(),
            Some(s) => s.to_string(),
            None => size.suffix(),
        };
        let folder = match str_param(row, "folder").and_then(safe_name) {
            Some(f) => f,
            None if format.raster => size.label(),
            None => format.id.to_uppercase(),
        };
        // The format's settings, without anything that picks artboards or sizes (rows do that).
        let mut options = Map::new();
        if let Some(o) = settings.get(format.id).and_then(Value::as_object) {
            options.extend(
                o.iter().filter(|(k, _)| !ROW_KEYS.contains(&k.as_str()) && k.as_str() != "useArtboards").map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        if let Some(o) = without_artboards(row).as_object() {
            options.extend(o.iter().filter(|(k, _)| !ROW_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())));
        }
        for k in ARTBOARD_PARAMS {
            options.remove(k);
        }
        // A file per artboard is for viewing: it doesn't carry the whole document to edit.
        options.entry("preserveEditing").or_insert(json!(false));
        let options = Value::Object(options);
        // A PDF preset (built-in or saved) becomes its settings.
        let options = if format.id == "pdf" { super::pdf::expand_preset(s, C, &options)?.into_owned() } else { options };
        Ok(Self { format, size, suffix, folder, options })
    }

    /// The encoder options for `region`, `artboard` of the document written.
    fn options_for(&self, region: Rect, artboard: Option<usize>) -> Result<Value> {
        let mut o = self.options.clone();
        if let Some(b) = artboard {
            o["artboard"] = json!(b);
        }
        if self.format.raster {
            match self.size {
                // The resolution is stored in the file too.
                ScreenSize::Ppi(ppi) => o["ppi"] = json!(ppi),
                size => {
                    o["scale"] = json!(size.scale_for(region)?);
                    if let Some(m) = o.as_object_mut() {
                        m.remove("ppi");
                    }
                }
            }
        }
        Ok(o)
    }
}

/// The settings the Asset Export panel shares with the dialog: what is exported, not which art
/// or where to.
pub(crate) const SHARED_KEYS: [&str; 6] = ["formats", "preset", "settings", "prefix", "subfolders", "antiAlias"];

/// What `document.exportForScreens` reads besides the formats.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ScreenParams {
    #[serde(flatten)]
    boards: ArtboardPick,
    folder: Option<String>,
    zip: bool,
    full_document: bool,
    include_bleed: bool,
    subfolders: bool,
    preset: Option<String>,
    prefix: Option<String>,
    /// Asset ids (Asset Export) to write instead of artboards.
    assets: Option<Vec<u64>>,
}

impl ScreenParams {
    fn of(p: &Value) -> Result<Self> {
        if p.is_object() { Self::deserialize(p).map_err(|e| bad(C, e.to_string())) } else { Ok(Self::default()) }
    }
}

/// The format rows `p` asks for (`formats`, else `preset`'s, else one PNG at 1x), each with the
/// format's `settings` and the top-level `antiAlias`.
fn rows_of(s: &Session, p: &Value, preset: Option<&str>) -> Result<Vec<Row>> {
    let settings = p.get("settings").cloned().unwrap_or(Value::Null);
    let rows: Vec<Value> = match (preset, p.get("formats")) {
        (Some(_), Some(f)) if !f.is_null() => return Err(bad(C, "give formats or a preset, not both")),
        (Some(id), _) => preset_rows(id).ok_or_else(|| bad(C, format!("preset `{id}`: mobile or density")))?,
        (None, Some(Value::Array(rows))) => rows.clone(),
        (None, None | Some(Value::Null)) => vec![json!({})],
        (None, Some(v)) => return Err(bad(C, format!("formats must be an array of rows, not {v}"))),
    };
    let mut rows: Vec<Row> = rows.iter().map(|r| Row::parse(s, r, &settings)).collect::<Result<_>>()?;
    if rows.is_empty() {
        return Err(bad(C, "no formats to export"));
    }
    // A top-level anti-aliasing mode applies to every row that doesn't name its own.
    if let Some(aa) = p.get("antiAlias") {
        for r in &mut rows {
            if let Some(m) = r.options.as_object_mut() {
                m.entry("antiAlias").or_insert_with(|| aa.clone());
            }
        }
    }
    Ok(rows)
}

/// Check the export settings in `p` (formats, preset, settings…) as an export would read them.
pub(crate) fn check_settings(s: &Session, p: &Value) -> Result<()> {
    rows_of(s, p, ScreenParams::of(p)?.preset.as_deref()).map(|_| ())
}

/// The file names (and bytes) an export wrote, or the files it returns.
#[derive(Default)]
struct Output {
    /// Lower-case relative names already used (two identical rows write one file).
    taken: HashSet<String>,
    /// Sub-folders already made.
    made: HashSet<String>,
    /// `(relative name, bytes)` kept to return or zip (empty when written to a folder).
    kept: Vec<(String, Vec<u8>)>,
    /// The paths written.
    written: Vec<String>,
}

impl Output {
    /// Is `name` new (not written by an identical row already)?
    fn claim(&mut self, name: &str) -> bool {
        self.taken.insert(name.to_lowercase())
    }

    /// Write `bytes` as `name` (relative) under `folder`, or keep them.
    fn put(&mut self, folder: Option<&str>, keep: bool, name: String, bytes: Vec<u8>) -> Result<()> {
        match folder {
            Some(dir) if !keep => {
                if let Some((sub, _)) = name.rsplit_once('/')
                    && self.made.insert(sub.to_lowercase())
                {
                    create_dir(&format!("{dir}/{sub}"))?;
                }
                let path = format!("{dir}/{name}");
                write_file(&path, &bytes)?;
                self.written.push(path);
            }
            _ => self.kept.push((name, bytes)),
        }
        Ok(())
    }
}

/// File → Export for Screens: [`export`], with the params remembered in the document
/// (`document.exportSettings`).
pub(super) fn export_for_screens(s: &mut Session, p: &Value) -> Result<Value> {
    let r = export(s, p)?;
    remember(s, p);
    Ok(r)
}

/// Every chosen artboard in every format row, one file each (a PDF holds its artboard alone), or
/// with `fullDocument` one file per row (a PDF of every artboard, else the bounds of all art), or
/// with `assets` each asset's art alone, cropped to it. Artboards (assets) that share a name get
/// `-2`, `-3`… instead of overwriting.
pub(crate) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let o = ScreenParams::of(p)?;
    let rows = rows_of(s, p, o.preset.as_deref())?;
    if o.full_document && o.assets.is_some() {
        return Err(bad(C, "give assets or fullDocument, not both"));
    }
    let subfolders = o.subfolders || o.preset.is_some();
    // A prefix names files, not folders.
    let prefix: String = o.prefix.as_deref().unwrap_or("").chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect();
    let folder = o.folder.as_deref().filter(|f| !f.is_empty());
    if let Some(dir) = folder {
        create_dir(dir)?;
    }
    let doc = s.doc()?.doc.clone();
    // Include Bleed: each artboard grown by the document's bleed.
    let doc = if o.include_bleed && doc.setup.has_bleed() {
        let mut d = (*doc).clone();
        for a in &mut d.artboards {
            a.rect = doc.setup.bleed_rect(a.rect);
        }
        Arc::new(d)
    } else {
        doc
    };
    let name_of = |r: &Row, stem: &str| {
        let file = format!("{prefix}{stem}{}.{}", r.suffix, r.format.extensions[0]);
        if subfolders { format!("{}/{file}", r.folder) } else { file }
    };
    let mut out = Output::default();
    if o.full_document {
        let stem = safe_name(&super::file_stem(&doc.title)).unwrap_or_else(|| "Untitled".into());
        // The bounds of all visible art, as the one artboard of the files that aren't PDFs.
        let mut whole: Option<Document> = None;
        for r in &rows {
            let name = name_of(r, &stem);
            if !out.claim(&name) {
                continue;
            }
            let bytes = if r.format.id == "pdf" {
                // One page per artboard.
                let mut options = r.options.clone();
                options["range"] = json!("all");
                encode(&doc, "pdf", &options)?
            } else {
                if whole.is_none() {
                    let bounds =
                        vectorcraft_render::encode::art_bounds(&doc).ok_or_else(|| bad(C, "nothing to export: the document has no visible art"))?;
                    whole = Some(single_artboard(&doc, bounds, "Document"));
                }
                let Some(d) = whole.as_ref() else { continue };
                let region = d.artboards.first().map_or(Rect::ZERO, |a| a.rect);
                encode(d, r.format.id, &r.options_for(region, Some(0))?)?
            };
            out.put(folder, o.zip, name, bytes)?;
        }
    } else if let Some(ids) = &o.assets {
        let assets =
            ids.iter().map(|id| doc.asset(*id).ok_or_else(|| bad(C, format!("no asset {id} (see assets.list)")))).collect::<Result<Vec<_>>>()?;
        if assets.is_empty() {
            return Err(bad(C, "no assets to export"));
        }
        for (a, stem) in assets.iter().zip(unique_file_names(assets.iter().map(|a| (a.name.as_str(), "Asset".to_string())))) {
            let (d, bounds) = isolated(&doc, &doc.paint_order(a.nodes.iter().copied()), &a.name)
                .ok_or_else(|| bad(C, format!("asset `{}` has no art to export", a.name)))?;
            for r in &rows {
                let name = name_of(r, &stem);
                if !out.claim(&name) {
                    continue;
                }
                let bytes = encode(&d, r.format.id, &r.options_for(bounds, Some(0))?)?;
                out.put(folder, o.zip, name, bytes)?;
            }
        }
    } else {
        let n = doc.artboards.len();
        let boards = o.boards.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect());
        if boards.is_empty() {
            return Err(bad(C, "the document has no artboard"));
        }
        for (b, stem) in boards.iter().copied().zip(artboard_file_names(&doc, &boards)) {
            let region = doc.artboards.get(b).map_or(Rect::ZERO, |a| a.rect);
            for r in &rows {
                let name = name_of(r, &stem);
                if !out.claim(&name) {
                    continue; // the same file from two identical rows
                }
                let bytes = encode(&doc, r.format.id, &r.options_for(region, Some(b))?)?;
                out.put(folder, o.zip, name, bytes)?;
            }
        }
    }
    let names: Vec<&str> = out.kept.iter().map(|(n, _)| n.as_str()).collect();
    if o.zip {
        let zip = super::zip::store(&out.kept).map_err(|e| bad(C, e))?;
        let zip_name = format!("{prefix}{}.zip", safe_name(&super::file_stem(&doc.title)).unwrap_or_else(|| "Untitled".into()));
        return match folder {
            Some(dir) => {
                let path = format!("{dir}/{zip_name}");
                write_file(&path, &zip)?;
                Ok(json!({ "path": path, "bytes": zip.len(), "files": names }))
            }
            None => Ok(json!({ "name": zip_name, "dataBase64": vectorcraft_format::base64_encode(&zip), "bytes": zip.len(), "files": names })),
        };
    }
    Ok(match folder {
        Some(_) => json!({ "files": out.written }),
        None => {
            json!({ "files": out.kept.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect::<Vec<_>>() })
        }
    })
}

/// Keep the params of a successful export in the document (without `zip`, a delivery detail, and
/// `assets`, a choice of art), so the dialog reopens on them.
fn remember(s: &mut Session, p: &Value) {
    let Some(o) = p.as_object() else { return };
    let mut settings = o.clone();
    settings.remove("zip");
    settings.remove("assets");
    // Only after a successful export, which had a document.
    let _ = store_settings(s, settings);
}

/// Make `settings` the document's export settings. Not an undo step (like the settings of any
/// export); the document has changed only if they differ.
pub(crate) fn store_settings(s: &mut Session, settings: Map<String, Value>) -> Result<()> {
    let st = s.doc_mut()?;
    if st.doc.export_settings != settings {
        Arc::make_mut(&mut st.doc).export_settings = settings;
        st.revision += 1;
    }
    Ok(())
}

/// `document.exportSettings`: the Export for Screens settings the document last exported with.
pub(super) fn export_settings(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "settings": s.doc()?.doc.export_settings }))
}
