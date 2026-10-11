//! Separations: the inks of a job ([`print_inks`]) and the document as one ink's plate, every
//! colour (and image pixel) replaced by the grey of that ink's coverage.

use std::sync::Arc;

use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms, Model, ProfileKind};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_doc::inks::{Link, inks, plates, visit_node_colors};
use vectorcraft_doc::overprint::multiply_overprints;
use vectorcraft_doc::{ColorMode, Document, Node};

use super::{DEFAULT_FREQUENCY, PrintInk, PrintSettings};

/// The inks of a separation of `doc`: the process inks, then one per spot swatch (none when
/// spots print as process), with the options of [`super::PrintOutput::inks`]; and a warning for
/// each option naming no ink.
pub fn print_inks(doc: &Document, set: &PrintSettings) -> (Vec<PrintInk>, Vec<String>) {
    let out = &set.output;
    let list: Vec<PrintInk> = plates(doc)
        .into_iter()
        .filter(|p| !(p.spot && out.spots_to_process))
        .map(|p| {
            let o = out.inks.iter().find(|i| i.name == p.name);
            PrintInk {
                print: o.is_none_or(|o| o.print),
                frequency: o.and_then(|o| o.frequency).unwrap_or(DEFAULT_FREQUENCY),
                angle: o.and_then(|o| o.angle).unwrap_or_else(|| PrintInk::default_angle(&p.name)),
                spot: p.spot,
                name: p.name,
            }
        })
        .collect();
    let warnings = out
        .inks
        .iter()
        .filter(|i| !list.iter().any(|p| p.name == i.name))
        .map(|i| {
            format!("there is no ink named `{}` to print (inks: {})", i.name, list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
        })
        .collect();
    (list, warnings)
}

/// The colour settings separations convert with: the active ones, with the printer profile as
/// their CMYK profile when it is a CMYK one.
fn separation_cms(set: &PrintSettings) -> Arc<Cms> {
    let active = cms::active();
    match cms::profile(set.color.profile.trim()) {
        Some(p) if p.kind == ProfileKind::Cmyk && cms::canonical_name(&active.settings().cmyk) != p.name => {
            let settings = cms::ColorSettings { cmyk: p.name, ..active.settings().clone() };
            Cms::new(&settings).map_or(active, Arc::new)
        }
        _ => active,
    }
}

/// Separates colours into inks as the job's colour options say.
pub(crate) struct Separator<'a> {
    pub doc: &'a Document,
    cms: Arc<Cms>,
    set: &'a PrintSettings,
}

impl<'a> Separator<'a> {
    pub fn new(doc: &'a Document, set: &'a PrintSettings) -> Self {
        Self { doc, cms: separation_cms(set), set }
    }

    /// How much of ink `plate` colour `c` (with swatch link `link`) prints, 0..1.
    fn ink(&self, plate: &str, c: &Color, link: Link) -> f32 {
        let color = &self.set.color;
        // Spot colours as process: only Registration keeps its link.
        let link = link.filter(|(name, _)| !self.set.output.spots_to_process || *name == REGISTRATION);
        // Without Preserve Numbers, CMYK colours are separated again from their appearance.
        let lab;
        let c = if !color.preserve_numbers && matches!(c, Color::Cmyk { .. }) {
            lab = self.cms.convert(c, Model::Lab, color.intent);
            &lab
        } else {
            c
        };
        inks(self.doc, &self.cms, c, link, color.intent).on_plate(plate)
    }

    /// Recolour `n`'s subtree as plate `plate`: the grey of each colour's coverage, unlinked.
    pub fn node(&self, plate: &str, n: &mut Node) {
        visit_node_colors(n, &mut |c, swatch, tint| {
            *c = Color::Gray { k: self.ink(plate, c, swatch.as_deref().map(|s| (s, *tint))) };
            *swatch = None;
            *tint = 1.0;
        });
    }

    /// The document as plate `plate`: art, symbols, pattern tiles and images in the grey of the
    /// ink's coverage. Overprinting fills and strokes multiply, so where they have none of this
    /// ink the plate below shows through instead of being knocked out.
    pub fn plate(&self, plate: &str) -> Document {
        let mut d = self.doc.clone();
        let discard_white = d.setup.discard_white_overprint;
        let nodes = d.layers.iter_mut().chain(d.symbols.iter_mut().map(|s| &mut s.art)).chain(d.patterns.iter_mut().flat_map(|p| p.art.iter_mut()));
        for n in nodes {
            multiply_overprints(n, discard_white);
            self.node(plate, Arc::make_mut(n));
        }
        for blob in d.images.values_mut() {
            let gray = blob.map_rgb(|[r, g, b]| {
                let rgb = |v: u8| v as f32 / 255.0;
                let v = (255.0 * (1.0 - self.ink(plate, &Color::rgb(rgb(r), rgb(g), rgb(b)), None))).round() as u8;
                [v; 3]
            });
            if let Some(gray) = gray {
                *blob = gray;
            }
        }
        // Every colour is a grey now: nothing to separate on the way out.
        d.color_mode = ColorMode::Rgb;
        d
    }
}
