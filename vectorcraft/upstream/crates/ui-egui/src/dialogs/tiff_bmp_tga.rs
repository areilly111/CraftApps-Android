//! The options of TIFF (colour model, byte order, LZW, profile), BMP (colour model, layout, depth,
//! palette, RLE, row order) and Targa (depth), after the raster export rows they share with PNG
//! Options ([`super::png_options`]): the `tiffOptions`, `bmpOptions` and `tgaOptions` dialogs.

use serde_json::{Map, Value, json};
use vectorcraft_render::encode::bmp::{self, BmpOptions};
use vectorcraft_render::encode::jpeg::ColorModel;
use vectorcraft_render::encode::quantize::{Dither, PaletteOptions, Reduction};
use vectorcraft_render::encode::tga::{self, TgaOptions};
use vectorcraft_render::encode::tiff::{ByteOrder, TiffOptions};

use super::form;
use super::png_options::choice;
use crate::state::Dialog;
use crate::widgets;

/// BMP colour models and layouts: param values and labels.
const BMP_MODELS: [&str; 2] = ["rgb", "gray"];
const BMP_MODEL_LABELS: [&str; 2] = ["RGB", "Grayscale"];
const LAYOUTS: [&str; 2] = ["windows", "os2"];
const LAYOUT_LABELS: [&str; 2] = ["Windows", "OS/2"];
/// Labels of [`bmp::DEPTHS`] and [`tga::DEPTHS`].
const BMP_DEPTH_LABELS: [&str; 6] = ["1 bit", "4 bit", "8 bit", "16 bit", "24 bit", "32 bit"];
const TGA_DEPTH_LABELS: [&str; 3] = ["16 bits/pixel", "24 bits/pixel", "32 bits/pixel"];

type Label<'a> = &'a dyn Fn(&mut egui::Ui, &str) -> egui::Response;

/// The options format `id` starts with (`cmyk`: a CMYK document exports CMYK TIFFs).
pub(super) fn defaults(id: &str, cmyk: bool, o: &mut Map<String, Value>) {
    match id {
        "tiff" => {
            let t = TiffOptions::default();
            let model = if cmyk { ColorModel::Cmyk } else { t.color_model };
            o.extend([
                ("colorModel".into(), json!(model.id())),
                ("byteOrder".into(), json!(t.byte_order.id())),
                ("lzw".into(), json!(t.lzw)),
                ("embedIcc".into(), json!(t.embed_icc)),
            ]);
        }
        "bmp" => {
            let (b, p) = (BmpOptions::default(), PaletteOptions::default());
            o.extend([
                ("colorModel".into(), json!(BMP_MODELS[usize::from(b.gray)])),
                ("fileFormat".into(), json!(LAYOUTS[usize::from(b.os2)])),
                ("depth".into(), json!(b.depth)),
                ("rle".into(), json!(b.rle)),
                ("flipRows".into(), json!(b.top_down)),
                ("reduction".into(), json!(p.reduction.id())),
                ("dither".into(), json!(p.dither.id())),
            ]);
        }
        "tga" => {
            o.insert("depth".into(), json!(TgaOptions::default().depth));
        }
        _ => {}
    }
}

/// Does format `id` keep transparency with the dialog's options (else it is flattened on white)?
pub(super) fn keeps_alpha(id: &str, d: &Dialog) -> bool {
    match id {
        "jpg" => false,
        "tiff" => ColorModel::from_id(&d.str("colorModel")).unwrap_or_default() == ColorModel::Rgb,
        "bmp" => d.f64("depth", 24.0) == 32.0,
        "tga" => d.f64("depth", 24.0) != 24.0,
        // A flat PSD is flattened; its layers keep transparency.
        "psd" => d.bool("layers"),
        _ => true,
    }
}

/// The grid rows of format `id`'s own options.
pub(super) fn rows(ui: &mut egui::Ui, d: &mut Dialog, id: &str, label: Label) {
    match id {
        "tiff" => tiff_rows(ui, d, label),
        "bmp" => bmp_rows(ui, d, label),
        "tga" => {
            label(ui, tl!("Depth:"));
            depth_row(ui, d, "tga-depth", &tga::DEPTHS, &TGA_DEPTH_LABELS, |_| true);
            ui.end_row();
        }
        _ => {}
    }
}

fn tiff_rows(ui: &mut egui::Ui, d: &mut Dialog, label: Label) {
    label(ui, tl!("Color Model:"));
    choice(ui, d, "colorModel", &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label));
    ui.end_row();

    label(ui, tl!("Byte Order:"));
    choice(ui, d, "byteOrder", &ByteOrder::ALL.map(ByteOrder::id), &ByteOrder::ALL.map(ByteOrder::label));
    ui.end_row();

    ui.label("");
    ui.horizontal(|ui| {
        form::check(ui, d, "lzw", tl!("LZW Compression"));
        form::check(ui, d, "embedIcc", tl!("Embed ICC Profile"));
    });
    ui.end_row();
}

fn bmp_rows(ui: &mut egui::Ui, d: &mut Dialog, label: Label) {
    label(ui, tl!("Color Model:"));
    choice(ui, d, "colorModel", &BMP_MODELS, &BMP_MODEL_LABELS);
    ui.end_row();

    label(ui, tl!("File Format:"));
    choice(ui, d, "fileFormat", &LAYOUTS, &LAYOUT_LABELS);
    ui.end_row();

    let os2 = d.str("fileFormat") == LAYOUTS[1];
    label(ui, tl!("Depth:"));
    depth_row(ui, d, "bmp-depth", &bmp::DEPTHS, &BMP_DEPTH_LABELS, |depth| !os2 || bmp::OS2_DEPTHS.contains(&depth));
    ui.end_row();

    // The palette of a colour image at 4 or 8 bits (1 bit is black and white; greys are fixed).
    let depth = d.f64("depth", 24.0) as u8;
    if matches!(depth, 4 | 8) && d.str("colorModel") != BMP_MODELS[1] {
        label(ui, tl!("Color Reduction:"));
        choice(ui, d, "reduction", &Reduction::ALL.map(Reduction::id), &Reduction::ALL.map(Reduction::label));
        ui.end_row();
        label(ui, tl!("Dither:"));
        choice(ui, d, "dither", &Dither::ALL.map(Dither::id), &Dither::ALL.map(Dither::label));
        ui.end_row();
    }

    ui.label("");
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!os2 && matches!(depth, 4 | 8), |ui| form::check(ui, d, "rle", tl!("Compress (RLE)")));
        ui.add_enabled_ui(!os2 && !d.bool("rle"), |ui| form::check(ui, d, "flipRows", tl!("Flip Row Order")));
    });
    ui.end_row();
    normalize_bmp(d);
}

/// Keep the BMP options writable together: OS/2 has no 16 or 32 bits, compression or flipped
/// rows; only 4- and 8-bit images are compressed, and those store their rows bottom-up.
fn normalize_bmp(d: &mut Dialog) {
    let os2 = d.str("fileFormat") == LAYOUTS[1];
    let mut depth = d.f64("depth", 24.0) as u8;
    if os2 && !bmp::OS2_DEPTHS.contains(&depth) {
        depth = 24;
        d.fields.insert("depth".into(), json!(depth));
    }
    let rle = d.bool("rle") && !os2 && matches!(depth, 4 | 8);
    for (key, on) in [("rle", rle), ("flipRows", d.bool("flipRows") && !os2 && !rle)] {
        if d.bool(key) != on {
            d.fields.insert(key.into(), json!(on));
        }
    }
}

/// A bits-per-pixel dropdown bound to `d.fields["depth"]` (`depths` shown as `labels`); `enabled`
/// greys out depths.
fn depth_row(ui: &mut egui::Ui, d: &mut Dialog, id: &str, depths: &[u8], labels: &[&str], enabled: impl Fn(u8) -> bool) {
    let cur = d.f64("depth", 24.0);
    let shown = depths.iter().position(|b| f64::from(*b) == cur).and_then(|i| labels.get(i)).copied().unwrap_or_default();
    if let Some(depth) =
        widgets::dropdown_with(ui, id, shown, labels, 150.0, |i| depths.get(i).is_some_and(|b| enabled(*b))).and_then(|i| depths.get(i))
    {
        d.fields.insert("depth".into(), json!(depth));
    }
}
