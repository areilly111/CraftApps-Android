//! After Effects-compatible expressions on the boa JavaScript engine.
//!
//! [`Expressions`] implements [`ExprHost`] so the renderer can evaluate property expressions:
//!
//! ```text
//! let host = effectcraft_expr::Expressions;
//! let mut r = Renderer::new(&project, &NoFootage, RenderOpts::default());
//! r.expr = Some(&host);
//! ```
//!
//! * **Language**: JavaScript plus AE's legacy array maths (`value + [10, 0]`, `[1, 2] * 2`),
//!   implemented by rewriting `+ - * /` into helper calls ([`rewrite`]). The last evaluated
//!   statement is the result (`var x = …; x`, `if/else` blocks).
//! * **Object model** (`prelude.js`): `time`, `value`, `thisComp`, `thisLayer`, `thisProperty`,
//!   `comp(name)`, layer attributes (`index`, `inPoint`, `width`, `transform`, `effect(…)(…)`,
//!   `mask`, `content`, `text.sourceText`, `marker`, `toComp`/`fromComp`/`toWorld`/`fromWorld`,
//!   `sourceRectAtTime`), property methods (`valueAtTime`, `velocityAtTime`, `speedAtTime`, `key`,
//!   `nearestKey`, `numKeys`, `wiggle`, `temporalWiggle`, `smooth`, `loopIn/Out[Duration]`) and the
//!   global helpers (`linear`/`ease`/`easeIn`/`easeOut`, `random`/`gaussRandom`/`seedRandom`/
//!   `noise`, vector maths, `clamp`, `lookAt`, time and colour conversions, `posterizeTime`).
//! * **Determinism**: `wiggle` and `random` are seeded by layer index, property uid (and time /
//!   `seedRandom`), so frames render identically on every thread and platform.
//! * **Runtime**: one boa `Context` per thread with compiled scripts cached by text. Scripts read
//!   the project through memoized host requests (see [`host`]), so nothing borrowed ever enters
//!   the JS heap. Expressions reading each other nest up to [`MAX_DEPTH`] levels.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod host;
mod noise;
pub mod rewrite;
mod runtime;
mod sample;

use effectcraft_keyframe::Value;
use effectcraft_project::{ItemId, Layer, LayerId, Project, Property};
use effectcraft_render::{EvalCtx, ExprHost};
use effectcraft_time::Tick;

use host::{Own, Req, Resolver, prop_dims, value_resp};
use runtime::{Begin, FRAMES, Frame, Run};

/// Maximum nesting of expressions that read other expressions (cycles stop here).
pub const MAX_DEPTH: usize = 16;
/// Maximum script re-runs while answering host requests.
const MAX_PASSES: usize = 2000;

/// The expression engine (stateless; runtimes are per thread).
#[derive(Clone, Copy, Debug, Default)]
pub struct Expressions;

impl ExprHost for Expressions {
    fn eval(&self, ctx: &EvalCtx, layer: &Layer, prop: &Property, value: &Value) -> Result<Value, String> {
        match &prop.expr {
            Some(e) => evaluate(self, ctx, layer, prop, value, &e.text),
            None => Ok(value.clone()),
        }
    }

    fn eval_text_selector(&self, ctx: &EvalCtx, layer: &Layer, prop: &Property, index: usize, total: usize, selector: [f64; 3]) -> Result<[f64; 3], String> {
        let Some(e) = prop.expr.as_ref().filter(|_| prop.has_expression()) else {
            let v = prop.value_at(layer.layer_time(ctx.time)).components();
            return Ok([v.first().copied().unwrap_or(100.0), v.get(1).copied().unwrap_or(100.0), v.get(2).copied().unwrap_or(100.0)]);
        };
        let value = prop.value_at(layer.layer_time(ctx.time));
        let out = evaluate_out(self, ctx, layer, prop, &value, &e.text, [index as f64 + 1.0, total as f64], selector)?;
        let num = |o: &runtime::Out| match o {
            runtime::Out::Num(x) => Some(*x),
            runtime::Out::Bool(b) => Some(if *b { 100.0 } else { 0.0 }),
            _ => None,
        };
        let r = match &out {
            runtime::Out::Arr(items) => {
                let g = |i: usize| items.get(i).and_then(num).or_else(|| items.first().and_then(num));
                [g(0), g(1), g(2)]
            }
            o => [num(o); 3],
        };
        match r {
            [Some(a), Some(b), Some(c)] if a.is_finite() && b.is_finite() && c.is_finite() => Ok([a, b, c]),
            _ => Err("Error: expression selector amount must be a number or an array of numbers".into()),
        }
    }
}

/// Pops the evaluation frame on every exit path.
struct FrameGuard;
impl Drop for FrameGuard {
    fn drop(&mut self) {
        FRAMES.with(|f| f.borrow_mut().pop());
    }
}

/// Evaluate `text` as the expression of `prop` on `layer` at `ctx.time`, given its keyframed
/// `value`. Other properties' expressions are evaluated through `host`.
pub fn evaluate(host: &dyn ExprHost, ctx: &EvalCtx, layer: &Layer, prop: &Property, value: &Value, text: &str) -> Result<Value, String> {
    let out = evaluate_out(host, ctx, layer, prop, value, text, [1.0, 1.0], [100.0; 3])?;
    runtime::to_value(&out, value)
}

/// Run an expression and return the raw script result. `text_sel` / `selector` are the text
/// Expression Selector's `[textIndex, textTotal]` and `selectorValue`.
#[allow(clippy::too_many_arguments)]
fn evaluate_out(
    host: &dyn ExprHost,
    ctx: &EvalCtx,
    layer: &Layer,
    prop: &Property,
    value: &Value,
    text: &str,
    text_sel: [f64; 2],
    selector: [f64; 3],
) -> Result<runtime::Out, String> {
    let depth = FRAMES.with(|f| f.borrow().len());
    if depth >= MAX_DEPTH {
        return Err("Error: expressions reference each other too deeply (circular reference?)".into());
    }
    let ctx = EvalCtx { project: ctx.project, comp_id: ctx.comp_id, comp: ctx.comp, time: ctx.time, expr: Some(host), footage: ctx.footage };
    let comp = ctx.comp_id.0;
    let path = layer.props.path_of(prop.uid).unwrap_or_else(|| format!("@{}", prop.uid));
    let resolver = Resolver { ctx, own: Own { comp, layer, prop, path: path.clone() } };
    let current = value_resp(value, prop_dims(layer, prop));
    let begin = Begin {
        time: ctx.time.seconds(),
        value: &current,
        comp,
        layer: layer.id.0,
        path: &path,
        uid: prop.uid,
        index: ctx.comp.index_of(layer.id).unwrap_or(0) as f64,
        frame_duration: ctx.comp.frame_duration().seconds(),
        text_sel,
        selector,
    };
    // Answer the requests nearly every expression makes up front.
    let mut frame = Frame::default();
    for req in [Req::CompInfo { comp }, Req::LayerInfo { comp, layer: layer.id.0 }, Req::PropInfo { comp, layer: layer.id.0, path: path.clone() }] {
        let r = resolver.resolve(&req);
        frame.memo.insert(req, r);
    }
    FRAMES.with(|f| f.borrow_mut().push(frame));
    let _guard = FrameGuard;
    for _ in 0..MAX_PASSES {
        match runtime::run(text, &begin) {
            Run::Done(r) => return r,
            Run::Pending => {
                let misses = FRAMES.with(|f| f.borrow_mut().get_mut(depth).map(|fr| std::mem::take(&mut fr.misses)).unwrap_or_default());
                for m in misses {
                    // May evaluate other expressions (nested frames above ours).
                    let r = resolver.resolve(&m);
                    FRAMES.with(|f| f.borrow_mut().get_mut(depth).map(|fr| fr.memo.insert(m, r)));
                }
            }
        }
    }
    Err("Error: expression needs too many property reads".into())
}

/// Evaluate `text` as if it were the expression on the property at `path` (e.g.
/// `transform/rotation`) of `layer` in `comp`, at comp time `t` seconds. Other properties'
/// expressions are evaluated too.
pub fn eval_standalone(project: &Project, comp: ItemId, layer: LayerId, path: &str, text: &str, t: f64) -> Result<Value, String> {
    let c = project.comp(comp).ok_or("no such comp")?;
    let l = c.layer(layer).ok_or("no such layer")?;
    let prop = l.props.prop(path).ok_or_else(|| format!("no property at {path}"))?;
    let ctx = EvalCtx::new(project, comp, c, Tick::from_seconds_f64(t));
    let value = prop.value_at(l.layer_time(ctx.time));
    evaluate(&Expressions, &ctx, l, prop, &value, text)
}

/// Evaluate the expression stored on the property at `path`, reporting errors (the renderer
/// silently falls back to the keyframed value instead).
pub fn eval_property(project: &Project, comp: ItemId, layer: LayerId, path: &str, t: f64) -> Result<Value, String> {
    let c = project.comp(comp).ok_or("no such comp")?;
    let l = c.layer(layer).ok_or("no such layer")?;
    let prop = l.props.prop(path).ok_or_else(|| format!("no property at {path}"))?;
    let ctx = EvalCtx::new(project, comp, c, Tick::from_seconds_f64(t));
    let value = prop.value_at(l.layer_time(ctx.time));
    match &prop.expr {
        Some(e) if prop.has_expression() => evaluate(&Expressions, &ctx, l, prop, &value, &e.text),
        _ => Ok(value),
    }
}

/// Syntax-check an expression (after the array-maths rewrite) without evaluating it.
pub fn check_syntax(text: &str) -> Result<(), String> {
    rewrite::check_nesting(text)?;
    runtime::check(text)
}

#[cfg(test)]
mod tests;
