//! GPU effects, Numbers and Timecode (kernel `ftx_text` in `shaders/fx_text.wgsl`). The text
//! is laid out and its glyph strokes rasterised on the CPU (`effectcraft_effects::text_layer`,
//! the CPU effect's own coverage: fill and stroke ring), uploaded as one image and composited
//! on the GPU with the layer, Timecode's box and the fill / stroke colours.

use effectcraft_effects::{Buf, EffectCtx};
use effectcraft_raster::Image;

use crate::context::{Enc, Params};
use crate::effects::GBuf;

/// Compute entry points in `fx_text.wgsl`.
pub(crate) const KERNELS: &[&str] = &["ftx_text"];

/// Effect ids implemented here.
pub(crate) const IDS: &[&str] = &["ec.text.numbers", "ec.text.timecode"];

/// Run effect `id` (one of [`IDS`]); `None` = this parameter combination runs on the CPU.
pub(crate) fn apply(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let shape = Buf { img: Image { width: b.img.width, height: b.img.height, data: vec![] }, offset: b.offset, scale: b.scale };
    let t = effectcraft_effects::text_layer(id, ctx, &shape)?;
    let cov = e.g.upload_image(&t.coverage)?;
    let mut p = Params::default();
    p.u[0] = [t.display, t.keep as u32, t.boxed.is_some() as u32, 0];
    p.f[0] = t.fill;
    p.f[1] = t.stroke;
    if let Some((r, c, a)) = t.boxed {
        p.f[2] = r.map(|v| v as f32);
        p.f[3] = [c[0], c[1], c[2], a];
    }
    let out = e.scratch(b.img.width, b.img.height);
    e.pixels("ftx_text", &p, &b.img, Some(&cov), &out, None);
    Some(GBuf { img: out, ..b })
}
