//! Soft object effects (Effects panel): drop shadow, outer glow, inner shadow and basic feather,
//! drawn with vello filter layers.
//!
//! vello_cpu only supports filter layers in single-threaded contexts, so on the multithreaded
//! pipeline each effect is rendered into an offscreen single-threaded context cropped to its
//! reach and composited back as an image ([`Renderer::with_filters`]); single-threaded contexts
//! draw directly.
//!
//! - Drop shadow / outer glow: the object's silhouette (everything it paints: fill, image, text,
//!   stroke) in a `DropShadowOnly` layer under the object. `size` sets the blur (σ = size / 2),
//!   `spread`/`choke` (percent of size) grow the silhouette instead of blurring it. The shadow's
//!   opacity rides on the filter colour's alpha.
//! - Inner shadow: the inverse of the offset shape, blurred, clipped to the shape, on top.
//! - Inner glow, bevel and emboss, satin: built like the inner shadow (an offset inverse of the
//!   shape, blurred and clipped) with screen / multiply blending; satin XORs two offset copies.
//! - Feather: the object is masked by its shape inset by half the width and blurred, so the
//!   edge fades from ~0 at the path to full strength `width` inside.

use designcraft_doc::Item;
use designcraft_geom::{Affine, BezPath, Rect, Shape, Vec2};
use vello_common::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::peniko::{self, BlendMode, Compose, Mix};
use vello_cpu::{RenderContext, kurbo};

use crate::{Frame, Renderer, color_of};

/// Shadow offset (spread units) for an InDesign angle (degrees, light direction) and distance.
pub(crate) fn offset(angle: f64, distance: f64) -> Vec2 {
    let a = angle.to_radians();
    Vec2::new(-a.cos() * distance, a.sin() * distance)
}

/// How far (points) the effects of `it` reach outside its geometry.
pub(crate) fn outset(it: &Item) -> f64 {
    let e = &it.effects;
    let mut o: f64 = 0.0;
    if e.drop_shadow.on {
        o = o.max(e.drop_shadow.distance.abs() + e.drop_shadow.size.abs() * 1.6 + 1.0);
    }
    if e.outer_glow.on {
        o = o.max(e.outer_glow.size.abs() * 1.6 + 1.0);
    }
    o
}

/// (blur σ, growth) in points for an effect `size` with `pct` percent hardened.
fn split(size: f64, pct: f64) -> (f64, f64) {
    let size = size.max(0.0);
    let p = (pct / 100.0).clamp(0.0, 1.0);
    (size * (1.0 - p) / 2.0, size * p)
}

fn blur(sigma: f64) -> Filter {
    Filter::from_primitive(FilterPrimitive::GaussianBlur { std_deviation: sigma.max(0.0) as f32, edge_mode: EdgeMode::None })
}

fn shadow(sigma: f64, color: peniko::Color) -> Filter {
    Filter::from_primitive(FilterPrimitive::DropShadowOnly {
        dx: 0.0,
        dy: 0.0,
        std_deviation: sigma.max(0.0) as f32,
        color,
        edge_mode: EdgeMode::None,
    })
}

impl Renderer {
    /// Draw an item with soft effects: below-effects, the (feathered) body, inner shadow.
    pub(crate) fn draw_item_fx(&mut self, ctx: &mut RenderContext, f: &Frame, it: &Item, bp: &BezPath, xf: Affine, page_name: Option<&str>) {
        let doc = f.doc;
        let e = &it.effects;
        let sw = if it.stroke.is_none() { 0.0 } else { it.stroke.extent() };
        // Spread-space reach of what the object paints.
        let reach = xf.transform_rect_bbox(bp.bounding_box()).inflate(sw + 1.0, sw + 1.0);
        // The silhouette is the shape when the object paints its area (fill / image); text frames
        // without a fill cast the shadow of their text only, which can't be grown.
        let solid = !it.fill.is_none() || matches!(it.content, designcraft_doc::Content::Graphic(_));
        let mut below = vec![];
        if e.outer_glow.on {
            let g = &e.outer_glow;
            below.push((Vec2::ZERO, g.size, g.spread, g.color.clone(), g.opacity));
        }
        if e.drop_shadow.on {
            let d = &e.drop_shadow;
            below.push((offset(doc.light_angle(d.angle, d.global_light), d.distance), d.size, d.spread, d.color.clone(), d.opacity));
        }
        for (off, size, pct, color, opacity) in below {
            let Some(c) = doc.resolve_color(&color, 1.0) else { continue };
            let (sigma, grow) = split(size, pct);
            let filter = shadow(sigma, color_of(&c, opacity.clamp(0.0, 1.0)));
            // The offset moves the geometry, not the filter: vello drops layer content that lies
            // entirely outside the viewport before filtering.
            let r = (reach + off).inflate(grow, grow);
            self.with_filters(ctx, f, r, sigma, |me, c, fr| {
                let moved = Frame { view: fr.view * Affine::translate(off), ..*fr };
                c.set_transform(moved.view);
                c.push_layer(None, None, None, None, Some(filter));
                me.draw_body(c, &moved, it, bp, xf, page_name);
                if grow > 0.0 && solid {
                    c.set_transform(moved.view * xf);
                    c.set_paint(peniko::Color::BLACK);
                    c.set_stroke(kurbo::Stroke::new(grow * 2.0).with_join(kurbo::Join::Round));
                    c.stroke_path(bp);
                }
                c.pop_layer();
            });
        }
        // Gradient feather: the object (feathered or not) drawn in its own layer, then kept as
        // much as the opacity gradient says.
        let gf = &e.gradient_feather;
        let directional = e.directional_feather.on && e.directional_feather.widths.iter().any(|w| *w > 0.0);
        if gf.on || directional {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, None, None, None, None);
        }
        // The object, feathered or not.
        if e.feather > 0.0 && it.path.is_closed() {
            let w = e.feather;
            self.with_filters(ctx, f, reach, w / 4.0, |me, c, fr| {
                c.set_transform(fr.view * xf);
                c.push_clip_layer(bp);
                me.draw_body(c, fr, it, bp, xf, page_name);
                // Keep the body where the blurred inset silhouette is.
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::DestIn)), None, None, None);
                c.set_transform(fr.view);
                c.push_layer(None, None, None, None, Some(blur(w / 4.0)));
                c.set_transform(fr.view * xf);
                c.set_paint(peniko::Color::BLACK);
                c.fill_path(bp);
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::DestOut)), None, None, None);
                c.set_transform(fr.view * xf);
                c.set_stroke(kurbo::Stroke::new(w).with_join(kurbo::Join::Round));
                c.stroke_path(bp);
                c.pop_layer();
                c.pop_layer();
                c.pop_layer();
                c.pop_layer();
            });
        } else {
            self.draw_body(ctx, f, it, bp, xf, page_name);
        }
        // Directional feather: one fading ramp per side, multiplied in.
        let df = &e.directional_feather;
        if df.on && df.widths.iter().any(|w| *w > 0.0) {
            let b = it.inner_bounds();
            let area = if xf.determinant().abs() > 1e-12 { xf.inverse().transform_rect_bbox(reach) } else { b };
            let ramps = [
                (df.widths[0], kurbo::Point::new(b.x0, b.y0), kurbo::Point::new(b.x0, b.y0 + df.widths[0])),
                (df.widths[1], kurbo::Point::new(b.x0, b.y0), kurbo::Point::new(b.x0 + df.widths[1], b.y0)),
                (df.widths[2], kurbo::Point::new(b.x0, b.y1), kurbo::Point::new(b.x0, b.y1 - df.widths[2])),
                (df.widths[3], kurbo::Point::new(b.x1, b.y0), kurbo::Point::new(b.x1 - df.widths[3], b.y0)),
            ];
            for (w, p0, p1) in ramps {
                if w <= 0.0 {
                    continue;
                }
                let stops = [
                    peniko::ColorStop::from((0.0, peniko::Color::from_rgba8(0, 0, 0, 0))),
                    peniko::ColorStop::from((1.0, peniko::Color::from_rgba8(0, 0, 0, 255))),
                ];
                ctx.set_transform(Affine::IDENTITY);
                ctx.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::DestIn)), None, None, None);
                ctx.set_transform(f.view * xf);
                ctx.set_paint(peniko::Gradient::new_linear(p0, p1).with_stops(stops.as_slice()));
                ctx.fill_rect(&area.inflate(4.0, 4.0));
                ctx.reset_paint_transform();
                ctx.pop_layer();
            }
        }
        if gf.on {
            let (p0, p1) = gf.points(it.inner_bounds());
            let stop = |o: f32, a: f32| peniko::ColorStop::from((o, peniko::Color::from_rgba8(0, 0, 0, (a.clamp(0.0, 1.0) * 255.0).round() as u8)));
            let stops = [stop(0.0, gf.start), stop(1.0, gf.end)];
            let grad = if gf.radial {
                peniko::Gradient::new_radial(p0, (p1 - p0).hypot().max(1e-3) as f32).with_stops(stops.as_slice())
            } else {
                peniko::Gradient::new_linear(p0, p1).with_stops(stops.as_slice())
            };
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::DestIn)), None, None, None);
            ctx.set_transform(f.view * xf);
            ctx.set_paint(grad);
            let area = if xf.determinant().abs() > 1e-12 { xf.inverse().transform_rect_bbox(reach) } else { it.inner_bounds() };
            ctx.fill_rect(&area.inflate(4.0, 4.0));
            ctx.reset_paint_transform();
            ctx.pop_layer();
        }
        if gf.on || directional {
            ctx.pop_layer();
        }
        if !it.path.is_closed() {
            return;
        }
        // Inner shadow, clipped to the shape.
        if e.inner_shadow.on
            && let Some(c) = doc.resolve_color(&e.inner_shadow.color, 1.0)
        {
            let s = &e.inner_shadow;
            let (sigma, choke) = split(s.size, s.choke);
            let off = offset(doc.light_angle(s.angle, s.global_light), s.distance);
            self.inner_edge(ctx, f, reach, bp, xf, off, sigma, choke, color_of(&c, s.opacity.clamp(0.0, 1.0)), Mix::Normal);
        }
        // Inner glow: from the edges inwards (or out from the centre), screened over the object.
        if e.inner_glow.on
            && let Some(c) = doc.resolve_color(&e.inner_glow.color, 1.0)
        {
            let g = &e.inner_glow;
            let (sigma, choke) = split(g.size, g.choke);
            let paint = color_of(&c, g.opacity.clamp(0.0, 1.0));
            if g.center {
                self.with_filters(ctx, f, reach, sigma, |_, c, fr| {
                    c.set_transform(fr.view * xf);
                    c.push_clip_layer(bp);
                    c.set_transform(Affine::IDENTITY);
                    c.push_layer(None, Some(BlendMode::new(Mix::Screen, Compose::SrcOver)), None, None, None);
                    c.set_transform(fr.view);
                    c.push_layer(None, None, None, None, Some(blur(sigma)));
                    c.set_transform(fr.view * xf);
                    c.set_paint(paint);
                    c.fill_path(bp);
                    // Fade towards the edge: cut a band of the glow's size away.
                    c.set_transform(Affine::IDENTITY);
                    c.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::DestOut)), None, None, None);
                    c.set_transform(fr.view * xf);
                    c.set_paint(peniko::Color::BLACK);
                    c.set_stroke(kurbo::Stroke::new((g.size - choke).max(0.5) * 2.0).with_join(kurbo::Join::Round));
                    c.stroke_path(bp);
                    c.pop_layer();
                    c.pop_layer();
                    c.pop_layer();
                    c.pop_layer();
                });
            } else {
                self.inner_edge(ctx, f, reach, bp, xf, Vec2::ZERO, sigma, choke, paint, Mix::Screen);
            }
        }
        // Satin: the shape shifted both ways along the angle, blurred, where exactly one covers.
        if e.satin.on
            && let Some(c) = doc.resolve_color(&e.satin.color, 1.0)
        {
            let st = &e.satin;
            let off = offset(st.angle, st.distance / 2.0);
            let sigma = st.size.max(0.0) / 2.0;
            let paint = color_of(&c, st.opacity.clamp(0.0, 1.0));
            let invert = st.invert;
            self.with_filters(ctx, f, reach, sigma, |_, c, fr| {
                c.set_transform(fr.view * xf);
                c.push_clip_layer(bp);
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(Mix::Multiply, Compose::SrcOver)), None, None, None);
                c.set_transform(fr.view);
                c.push_layer(None, None, None, None, Some(blur(sigma)));
                if invert {
                    c.set_transform(fr.view * xf);
                    c.set_paint(paint);
                    c.fill_path(bp);
                }
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(Mix::Normal, if invert { Compose::DestOut } else { Compose::SrcOver })), None, None, None);
                c.set_transform(fr.view * Affine::translate(off) * xf);
                c.set_paint(paint);
                c.fill_path(bp);
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(Mix::Normal, Compose::Xor)), None, None, None);
                c.set_transform(fr.view * Affine::translate(-off) * xf);
                c.fill_path(bp);
                c.pop_layer();
                c.pop_layer();
                c.pop_layer();
                c.pop_layer();
                c.pop_layer();
            });
        }
        // Bevel and Emboss (inner bevel): highlight from the lit side, shadow from the other.
        if e.bevel.on {
            let b = &e.bevel;
            let off = offset(doc.light_angle(b.angle, b.global_light), b.size * (b.depth / 100.0).clamp(0.01, 10.0) * 0.5);
            let sigma = b.size.max(0.0) / 2.0;
            if let Some(c) = doc.resolve_color(&b.highlight, 1.0) {
                // The inverse shape moved away from the light lights the edges facing it.
                self.inner_edge(ctx, f, reach, bp, xf, off, sigma, 0.0, color_of(&c, b.highlight_opacity.clamp(0.0, 1.0)), Mix::Screen);
            }
            if let Some(c) = doc.resolve_color(&b.shadow, 1.0) {
                self.inner_edge(ctx, f, reach, bp, xf, -off, sigma, 0.0, color_of(&c, b.shadow_opacity.clamp(0.0, 1.0)), Mix::Multiply);
            }
        }
    }

    /// An inner shadow-like edge: everything outside the shape moved by `off`, blurred by
    /// `sigma`, clipped to the shape and blended with `mix`.
    #[allow(clippy::too_many_arguments)]
    fn inner_edge(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        reach: Rect,
        bp: &BezPath,
        xf: Affine,
        off: Vec2,
        sigma: f64,
        choke: f64,
        paint: peniko::Color,
        mix: Mix,
    ) {
        self.with_filters(ctx, f, reach, sigma, |_, c, fr| {
            c.set_transform(fr.view * xf);
            c.push_clip_layer(bp);
            if mix != Mix::Normal {
                c.set_transform(Affine::IDENTITY);
                c.push_layer(None, Some(BlendMode::new(mix, Compose::SrcOver)), None, None, None);
            }
            c.set_transform(fr.view);
            c.push_layer(None, None, None, None, Some(blur(sigma)));
            // Everything outside the offset shape, so the colour bleeds in from the edges.
            let m = Affine::translate(off) * xf;
            let pad = sigma * 4.0 + choke + off.hypot() + 4.0 * fr.px;
            let mut inv = m.transform_rect_bbox(bp.bounding_box()).inflate(pad, pad).to_path(0.1);
            let mut shape = bp.clone();
            shape.apply_affine(m);
            inv.extend(shape.iter());
            c.set_paint(paint);
            c.set_fill_rule(peniko::Fill::EvenOdd);
            c.fill_path(&inv);
            c.set_fill_rule(peniko::Fill::NonZero);
            if choke > 0.0 {
                c.set_stroke(kurbo::Stroke::new(choke * 2.0).with_join(kurbo::Join::Round));
                c.stroke_path(&shape);
            }
            c.pop_layer();
            if mix != Mix::Normal {
                c.pop_layer();
            }
            c.pop_layer();
        });
    }

    /// Run `draw`, which pushes filter layers. Single-threaded contexts draw directly. On a
    /// multithreaded context the layer is drawn into an offscreen single-threaded context
    /// covering `reach` (spread space) plus the blur's spread, then composited back.
    pub(crate) fn with_filters(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        reach: Rect,
        sigma: f64,
        draw: impl FnOnce(&mut Self, &mut RenderContext, &Frame),
    ) {
        if !f.mt {
            return draw(self, ctx, f);
        }
        // 3σ of the Gaussian in pixels, plus a pixel of antialiasing.
        let spread = sigma.max(0.0) * 3.0 / f.px + 2.0;
        let screen = Rect::new(0.0, 0.0, ctx.width() as f64, ctx.height() as f64).inflate(spread, spread);
        let r = f.view.transform_rect_bbox(reach).inflate(spread, spread).intersect(screen);
        let (x0, y0) = (r.x0.floor(), r.y0.floor());
        let (w, h) = ((r.x1.ceil() - x0).min(crate::MAX_SIDE as f64), (r.y1.ceil() - y0).min(crate::MAX_SIDE as f64));
        if w < 1.0 || h < 1.0 {
            return;
        }
        let (w, h) = (w as u16, h as u16);
        let mut off = RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() });
        let shifted = Frame { mt: false, view: Affine::translate((-x0, -y0)) * f.view, ..*f };
        draw(self, &mut off, &shifted);
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        ctx.set_transform(Affine::translate((x0, y0)));
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(std::sync::Arc::new(pm)), sampler: peniko::ImageSampler::default() });
        ctx.fill_rect(&Rect::new(0.0, 0.0, w as f64, h as f64));
        ctx.set_transform(Affine::IDENTITY);
    }
}
