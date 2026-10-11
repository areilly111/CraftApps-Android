//! Soft proofing (View → Proof Colors), Overprint Preview and Separations Preview.
//!
//! * **Proof Colors** post-processes the rendered frame: every pixel goes display sRGB → proof
//!   space (CMYK press or RGB/colour-blindness simulation) → display, through a cached 17³ LUT.
//! * **Separations Preview** recolours the *document* before rendering: each paint colour is split
//!   into process inks (colour-managed), or a spot ink at the colour's tint when it's linked to a
//!   spot swatch (solid colours and gradient stops alike). With one plate visible the plate
//!   renders as greyscale ink coverage (black = 100%); with several, the visible inks are
//!   composited through the proof profile, spots multiplied on top. Colours linked to the
//!   Registration swatch print their tint on every plate, spot plates included.
//!   Placed raster images are not separated (limitation).
//! * **Overprint Preview** (and Separations Preview, which implies it): fills and strokes that
//!   overprint ([`vectorcraft_doc::FillLayer::overprint`], characters' too) are drawn with
//!   Multiply, which approximates their inks printing over the inks below: a zero ink lets the
//!   inks below show through, where a knockout would replace them.
//!
//! The app's current view state lives in [`view`] / [`set_view`]; the canvas copies it into
//! [`crate::RenderOptions`] with [`active_proof`] and [`overprint_preview_on`].

use std::borrow::Cow;
use std::sync::RwLock;

use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms, PROCESS_PLATES};
pub use vectorcraft_color::cms::{Intent, ProofSetup, ProofTarget};
use vectorcraft_doc::Document;
pub use vectorcraft_doc::inks::{Inks, Link, Plate, inks, map_document_colors, map_node_colors, plates, spot_color};
use vectorcraft_doc::overprint::multiply_overprints;

use crate::RenderOptions;

/// Whether overprinting shows: Overprint Preview, or Separations Preview (which implies it).
pub(crate) fn overprints(opts: &RenderOptions) -> bool {
    opts.overprint_preview || opts.proof.as_ref().is_some_and(|p| p.separations.is_some())
}

fn plate_color(doc: &Document, c: &Cms, proof: &ProofSetup, visible: &[String], color: &Color, link: Link) -> Color {
    let ink = inks(doc, c, color, link, proof.intent);
    let rgb = if visible.len() == 1 {
        [1.0 - ink.on_plate(&visible[0]); 3]
    } else {
        let mut cmyk = ink.cmyk;
        for (i, p) in PROCESS_PLATES.iter().enumerate() {
            if !visible.iter().any(|v| v == p) {
                cmyk[i] = 0.0;
            }
        }
        let mut rgb = c.proof_cmyk_to_srgb(cmyk, proof);
        for name in visible.iter().filter(|v| !PROCESS_PLATES.contains(&v.as_str())) {
            let t = ink.spot_tint(name);
            if t > 0.0
                && let Some(sc) = spot_color(doc, name)
            {
                let s = c.display_rgb(&sc);
                for i in 0..3 {
                    rgb[i] *= 1.0 - t * (1.0 - s[i]);
                }
            }
        }
        rgb
    };
    let [r, g, b] = c.srgb_to_rgb(rgb);
    Color::Rgb { r, g, b }
}

/// The document as it should be drawn for these options (overprints, separations).
pub(crate) fn prepare<'a>(doc: &'a Document, opts: &RenderOptions) -> Cow<'a, Document> {
    let seps = opts.proof.as_ref().and_then(|p| p.separations.as_ref().map(|s| (p, s)));
    let overprint = overprints(opts) && (doc.layers.iter().any(|l| l.has_overprint()) || doc.symbols.iter().any(|s| s.art.has_overprint()));
    if !overprint && seps.is_none() {
        return Cow::Borrowed(doc);
    }
    let mut d = doc.clone();
    if overprint {
        let discard_white = doc.setup.discard_white_overprint;
        for l in &mut d.layers {
            multiply_overprints(l, discard_white);
        }
        for s in &mut d.symbols {
            multiply_overprints(&mut s.art, discard_white);
        }
    }
    if let Some((proof, visible)) = seps {
        let c = cms::active();
        let src = doc.clone();
        map_document_colors(&mut d, &mut |col, sw| plate_color(&src, &c, proof, visible, col, sw));
    }
    Cow::Owned(d)
}

/// Soft-proof the rendered (premultiplied RGBA8) pixels in place.
pub(crate) fn post(pixels: &mut [u8], opts: &RenderOptions) {
    let Some(proof) = opts.proof.as_ref() else { return };
    if proof.separations.is_some() || matches!(proof.target, ProofTarget::MonitorRgb | ProofTarget::Srgb) {
        return;
    }
    let lut = cms::active().proof_lut(proof);
    let mut last_in = [0u8; 4];
    let mut last_out = [0u8; 4];
    for px in pixels.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a == 0 {
            continue;
        }
        if *px == last_in {
            px.copy_from_slice(&last_out);
            continue;
        }
        last_in.copy_from_slice(px);
        let un = |c: u8| if a == 255 { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        let out = lut.apply8([un(px[0]), un(px[1]), un(px[2])]);
        for i in 0..3 {
            px[i] = if a == 255 { out[i] } else { ((out[i] as u32 * a as u32 + 127) / 255) as u8 };
        }
        last_out.copy_from_slice(px);
    }
}

/// View-level proof state (View → Proof Setup / Proof Colors / Overprint Preview, Separations
/// Preview panel). Process-wide: the app has one proof state for all windows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProofView {
    /// View → Proof Setup.
    pub setup: ProofSetup,
    /// View → Proof Colors.
    pub proof_colors: bool,
    /// View → Overprint Preview.
    pub overprint: bool,
    /// Separations Preview: `Some(visible plates)` when on.
    pub separations: Option<Vec<String>>,
}

static VIEW: RwLock<Option<ProofView>> = RwLock::new(None);

pub fn view() -> ProofView {
    VIEW.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

pub fn set_view(v: ProofView) {
    *VIEW.write().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// The proof to pass in [`RenderOptions::proof`] for the current view state.
pub fn active_proof() -> Option<ProofSetup> {
    let v = view();
    match v.separations {
        Some(s) => Some(ProofSetup { separations: Some(s), ..v.setup }),
        None if v.proof_colors => Some(ProofSetup { separations: None, ..v.setup }),
        None => None,
    }
}

/// Whether [`RenderOptions::overprint_preview`] should be on (Separations Preview implies it).
pub fn overprint_preview_on() -> bool {
    let v = view();
    v.overprint || v.separations.is_some()
}
