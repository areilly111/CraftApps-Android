//! GPU effects, particle render passes (kernels in `shaders/fx_particles.wgsl`, every entry
//! point prefixed `fxp_`): CC Particle World, CC Particle Systems II, Particle Playground, CC
//! Ball Action, CC Pixel Polly, CC Scatterize and Curl Noise; plus Foam's textured discs,
//! reflections and flow-map preview (`fx_sim` draws Foam's bubble sprites).
//!
//! As in `fx_sim`, each frame's plan is built on the CPU and shared with the CPU effect, which
//! stays the oracle: the particle systems' sprites (`effects::sprite_plan`, from the stepped
//! simulation — on the GPU's `ParticleSim` when the host offers it — sorted as the CPU sorts
//! them), Particle Playground's dots, text strokes and Layer Map frames
//! (`effects::playground_plan`), and the plans of the effects that read the layer's pixels to
//! place or colour their particles (`effects::pixel_plan`: CC Ball Action's balls, CC
//! Scatterize's pixel dots, CC Pixel Polly's shards; the frame is read back once). Rasterising
//! runs here: items are tiled like `fx_sim`'s and `fxp_raster` composites each pixel's items in
//! the CPU's order over a base image, so a plan too large for one item table is drawn in
//! several passes, each over the previous one ([`Chunked`]; the per-pixel arithmetic is the
//! same). Particle Playground's Layer Map frames are packed into one atlas texture.
//!
//! Curl Noise runs fully here: the fBm potential, its curl (central differences, edges
//! clamped) and the backwards streamline trace, each a kernel mirroring the CPU's passes.

use effectcraft_effects::{Buf, EffectCtx, PixelPlan};
use effectcraft_raster::Image;

use crate::context::{Enc, GpuImage, Params};
use crate::effects::GBuf;
use crate::fx_sim::{Items, piece_pass, sprite_layer, sprites};

/// Compute entry points in `fx_particles.wgsl`.
pub(crate) const KERNELS: &[&str] = &["fxp_raster", "fxp_under", "fxp_curl_pot", "fxp_curl_vel", "fxp_curl_view", "fxp_curl_flow"];

/// Effect ids implemented here.
pub(crate) const IDS: &[&str] = &[
    "ec.sim.ccparticleworld",
    "ec.sim.ccparticlesystems2",
    "ec.sim.particleplayground",
    "ec.sim.ccballaction",
    "ec.sim.ccpixelpolly",
    "ec.sim.ccscatterize",
    "ec.noise.curlnoise",
];

/// Run effect `id` (one of [`IDS`]); `None` = this parameter combination runs on the CPU.
pub(crate) fn apply(e: &mut Enc, id: &str, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let geo = Buf { img: Image::new(0, 0), offset: b.offset, scale: b.scale };
    match id {
        "ec.sim.ccparticleworld" | "ec.sim.ccparticlesystems2" => match effectcraft_effects::sprite_plan(id, ctx, &geo) {
            Some(plan) => sprites(e, b, &plan),
            None => Some(b),
        },
        "ec.sim.particleplayground" => playground(e, ctx, b, &geo),
        "ec.noise.curlnoise" => curl_noise(e, ctx, b),
        _ => {
            // The plan reads the layer's pixels.
            let img = e.download(&b.img)?;
            let cpu = Buf { img, offset: b.offset, scale: b.scale };
            match effectcraft_effects::pixel_plan(id, ctx, &cpu) {
                Some(PixelPlan::Sprites(plan)) => sprites(e, b, &plan),
                Some(PixelPlan::Pieces(plan)) => piece_pass(e, b, &plan),
                None => Some(b),
            }
        }
    }
}

// ---------------------------------------------------------------- chunked raster

/// Items rasterised by `fxp_raster` over a base image (transparent at first), flushed into a
/// new pass whenever the item table would outgrow one kernel's data. `kind`: 0 sprites, 1
/// Layer Map frames, 2 Foam texture discs, 3 Foam reflection discs, 4 pieces without a back
/// texture; `mode`: 0 over, 1 add, 2 Foam's disc compositing.
pub(crate) struct Chunked {
    kind: u32,
    mode: u32,
    stride: usize,
    w: u32,
    h: u32,
    items: Items,
    base: Option<GpuImage>,
    ok: bool,
    /// Kernel parameters besides `u[0]` and `u[1].x`.
    pub p: Params,
}

impl Chunked {
    /// `stride` = floats per item, its four bounds included.
    pub(crate) fn new(kind: u32, mode: u32, stride: usize, w: u32, h: u32) -> Chunked {
        Chunked { kind, mode, stride, w, h, items: Items::new(stride, w, h), base: None, ok: true, p: Params::default() }
    }

    /// Start from `base` instead of a transparent frame.
    pub(crate) fn over(mut self, base: GpuImage) -> Chunked {
        self.base = Some(base);
        self
    }

    /// Add an item (`rec` without its bounds) covering float bounds `bb`; `src` is the texture
    /// the items sample.
    pub(crate) fn push(&mut self, e: &mut Enc, src: &GpuImage, bb: Option<[f32; 4]>, rec: &[f32]) {
        if !self.items.fits(bb) && !self.items.is_empty() {
            self.flush(e, src);
        }
        self.items.push(bb, rec);
    }

    fn flush(&mut self, e: &mut Enc, src: &GpuImage) {
        let items = std::mem::replace(&mut self.items, Items::new(self.stride, self.w, self.h));
        let tx = items.tx as u32;
        let Some((data, items_at, stride)) = items.data() else {
            self.ok = false;
            return;
        };
        let buf = e.data(&data);
        let base = self.base.take().unwrap_or_else(|| e.zeros(self.w, self.h));
        let mut p = self.p;
        p.u[0] = [self.kind, self.mode, items_at, stride];
        p.u[1][0] = tx;
        let out = e.scratch(self.w, self.h);
        e.pixels("fxp_raster", &p, src, Some(&base), &out, Some(&buf));
        self.base = Some(out);
    }

    /// The result (`None` when an item alone outgrew a kernel's data).
    pub(crate) fn finish(mut self, e: &mut Enc, src: &GpuImage) -> Option<GpuImage> {
        if !self.items.is_empty() {
            self.flush(e, src);
        }
        if !self.ok {
            return None;
        }
        Some(match self.base {
            Some(b) => b,
            None => e.image(self.w, self.h),
        })
    }
}

/// Bounds of a disc drawn over `[floor(x − r − 1), ceil(x + r + 1))` (sim2::foam_disc), as
/// inclusive float bounds for [`Items`].
fn disc_bounds(x: f64, y: f64, r: f64) -> Option<[f32; 4]> {
    (r > 0.0).then(|| [(x - r - 1.0) as f32, (y - r - 1.0) as f32, ((x + r + 1.0).ceil() - 1.0) as f32, ((y + r + 1.0).ceil() - 1.0) as f32])
}

// ---------------------------------------------------------------- Foam's extras

/// Foam: the bubble sprites, then the User Defined texture's discs, the Environment Map's
/// reflections and the flow-map preview (sim2::FoamPlan::finish).
pub(crate) fn foam(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let (w, h) = (b.img.width, b.img.height);
    // The plan fits other layers to the buffer: it needs the buffer's size (not its pixels).
    let geo = Buf { img: Image::new(w, h), offset: b.offset, scale: b.scale };
    let plan = effectcraft_effects::foam_full_plan(ctx, &geo);
    let mut fx = sprite_layer(e, &b.img, &plan.sprites)?;
    if let Some((tex, discs)) = &plan.user {
        let t = e.g.upload_image(&tex.buf.img)?;
        let mut ch = Chunked::new(2, 2, 16, w, h).over(fx);
        for d in discs {
            let (s, c) = d.rot.sin_cos();
            let rec = [2.0, d.x as f32, d.y as f32, d.r, s, c, d.fade, tex.size[0] as f32, tex.size[1] as f32, tex.buf.scale as f32];
            let rec = [&rec[..], &[tex.buf.offset[0] as f32, tex.buf.offset[1] as f32]].concat();
            ch.push(e, &t, disc_bounds(d.x, d.y, d.r as f64), &rec);
        }
        fx = ch.finish(e, &t)?;
    }
    if let Some(env) = &plan.env {
        let t = e.g.upload_image(&env.img)?;
        let mut ch = Chunked::new(3, 2, 12, w, h).over(fx);
        for d in &env.discs {
            let rec = [3.0, d.x as f32, d.y as f32, d.r, env.conv as f32, env.strength, w as f32, h as f32];
            ch.push(e, &t, disc_bounds(d.x, d.y, d.r as f64), &rec);
        }
        fx = ch.finish(e, &t)?;
    }
    if let Some(fm) = &plan.flow {
        let f = e.g.upload_image(fm)?;
        let out = e.scratch(w, h);
        e.pixels("fxp_under", &Params::default(), &fx, Some(&f), &out, None);
        fx = out;
    }
    Some(GBuf { img: fx, ..b })
}

// ---------------------------------------------------------------- Particle Playground

fn playground(e: &mut Enc, ctx: &EffectCtx, b: GBuf, geo: &Buf) -> Option<GBuf> {
    let plan = effectcraft_effects::playground_plan(ctx, geo);
    let (w, h) = (b.img.width, b.img.height);
    let mut fx = sprite_layer(e, &b.img, &plan.sprites)?;
    if !plan.blits.is_empty() {
        // The Layer Map frames, packed into one atlas on a grid of equal cells.
        let cw = plan.frames.iter().map(|f| f.buf.img.width).max().unwrap_or(1).max(1);
        let chh = plan.frames.iter().map(|f| f.buf.img.height).max().unwrap_or(1).max(1);
        let max = e.g.max_dim;
        let cols = (max / cw).max(1).min(plan.frames.len() as u32);
        let rows = (plan.frames.len() as u32).div_ceil(cols);
        if !e.g.fits(cw * cols, chh * rows) {
            return None;
        }
        let mut atlas = Image::new(cw * cols, chh * rows);
        let mut at = vec![];
        for (i, f) in plan.frames.iter().enumerate() {
            let (ax, ay) = ((i as u32 % cols) * cw, (i as u32 / cols) * chh);
            let img = &f.buf.img;
            for y in 0..img.height {
                let row = &img.data[(y * img.width) as usize..((y + 1) * img.width) as usize];
                let o = ((ay + y) * atlas.width + ax) as usize;
                atlas.data[o..o + row.len()].copy_from_slice(row);
            }
            at.push((ax, ay));
        }
        let t = e.g.upload_image(&atlas)?;
        let mut ch = Chunked::new(1, 0, 24, w, h).over(fx);
        for q in &plan.blits {
            let lp = &plan.frames[q.frame];
            if q.scale[0] == 0.0 || q.scale[1] == 0.0 {
                continue;
            }
            // sim3::blit_layer's pixel range.
            let (fw, fh) = (lp.size[0] * q.scale[0].abs() * b.scale, lp.size[1] * q.scale[1].abs() * b.scale);
            let r = (fw * fw + fh * fh).sqrt() * 0.5 + 1.0;
            let bb = [(q.x - r) as f32, (q.y - r) as f32, ((q.x + r).ceil() - 1.0) as f32, ((q.y + r).ceil() - 1.0) as f32];
            let (sa, ca) = q.angle.to_radians().sin_cos();
            let (ax, ay) = at[q.frame];
            let rec = [
                1.0,
                q.x as f32,
                q.y as f32,
                sa as f32,
                ca as f32,
                q.scale[0] as f32,
                q.scale[1] as f32,
                b.scale as f32,
                q.alpha,
                lp.size[0] as f32,
                lp.size[1] as f32,
                lp.buf.scale as f32,
                lp.buf.offset[0] as f32,
                lp.buf.offset[1] as f32,
                ax as f32,
                ay as f32,
                lp.buf.img.width as f32,
                lp.buf.img.height as f32,
            ];
            ch.push(e, &t, Some(bb), &rec);
        }
        fx = ch.finish(e, &t)?;
    }
    // The particles replace the layer's own pixels.
    Some(GBuf { img: fx, ..b })
}

// ---------------------------------------------------------------- Curl Noise

/// noise2::curl_noise: the potential, its curl and the streamline trace.
fn curl_noise(e: &mut Enc, ctx: &EffectCtx, b: GBuf) -> Option<GBuf> {
    let pr = ctx.params;
    let amount = pr.f("displacementAmount") * b.scale;
    let view = pr.e("view");
    if amount.abs() < 1e-6 && view == 0 {
        return Some(b);
    }
    let scale = (pr.f("scale").max(1.0) * b.scale) as f32;
    let rot = pr.f("rotation").to_radians() as f32;
    let off = pr.v2("offset");
    let (ox, oy) = ((off[0] * b.scale + b.offset[0]) as f32, (off[1] * b.scale + b.offset[1]) as f32);
    let evo = (pr.f("evolution") / 360.0) as f32;
    let seed = pr.f("randomSeed") as i64 as u32 ^ 0x51_7cc1;
    let steps = pr.f("steps").clamp(1.0, 16.0) as usize;
    let (w, h) = (b.img.width, b.img.height);
    let mut p = Params::default();
    p.u[0] = [seed, steps as u32, (pr.e("edgeBehavior") == 1) as u32, 0];
    p.f[0] = [ox, oy, scale, pr.f("complexity") as f32];
    p.f[1] = [rot.cos(), rot.sin(), evo * 4.0, amount as f32 / steps as f32];
    let pot = e.scratch(w, h);
    e.pixels("fxp_curl_pot", &p, &b.img, None, &pot, None);
    let vel = e.scratch(w, h);
    e.pixels("fxp_curl_vel", &p, &pot, None, &vel, None);
    let out = e.scratch(w, h);
    if view == 1 {
        e.pixels("fxp_curl_view", &p, &vel, None, &out, None);
    } else {
        e.pixels("fxp_curl_flow", &p, &b.img, Some(&vel), &out, None);
    }
    Some(GBuf { img: out, ..b })
}
