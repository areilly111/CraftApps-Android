//! The expression error bar: a strip at the bottom of the Composition viewer listing failing
//! expressions of the comp at the current time (`expr.errors`), with ◀ ▶ to step through them.
//! Clicking the message selects the layer and reveals the property in the timeline.
//! Automation ids: `viewer.exprErrors`, `viewer.exprErrors.prev`, `viewer.exprErrors.next`.

use crate::i18n::tr;
use effectcraft_engine::commands::expr_tools::{self, ExprError};
use effectcraft_engine::project::ItemId;
use egui::{Align2, Color32, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::EffectcraftApp;
use crate::theme::Tokens;

/// Height of the bar.
pub const HEIGHT: f32 = 22.0;

#[derive(Clone, Default)]
struct Cache {
    key: (u64, u64, i64),
    errors: Vec<ExprError>,
    /// Shown error (index).
    current: usize,
}

fn cache_id() -> egui::Id {
    egui::Id::new("expr-error-bar")
}

/// The comp's failing expressions (recomputed when the project or the time changes; kept while
/// playing).
fn errors(app: &EffectcraftApp, ctx: &egui::Context, cid: ItemId) -> Cache {
    let t = app.session.time_of(cid);
    let key = (app.session.revision, cid.0, t.0);
    let mut c: Cache = ctx.data(|d| d.get_temp(cache_id())).unwrap_or_default();
    let stale = c.key != key && !(app.playback.playing && c.key.1 == cid.0);
    if stale {
        c.errors = expr_tools::errors(&app.session, cid, t);
        c.key = key;
        c.current = c.current.min(c.errors.len().saturating_sub(1));
        ctx.data_mut(|d| d.insert_temp(cache_id(), c.clone()));
    }
    c
}

/// Height the bar needs for `cid` (0 when every expression evaluates).
pub fn height(app: &EffectcraftApp, ctx: &egui::Context, cid: ItemId) -> f32 {
    if app.session.expr.is_none() || errors(app, ctx, cid).errors.is_empty() { 0.0 } else { HEIGHT }
}

pub fn draw(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect, cid: ItemId) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let mut c = errors(app, &ctx, cid);
    if c.errors.is_empty() {
        return;
    }
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, Color32::from_rgb(0x4a, 0x3a, 0x12));
    let warn = Color32::from_rgb(0xf5, 0xb0, 0x30);
    p.text(pos2(rect.min.x + 10.0, rect.center().y), Align2::LEFT_CENTER, "⚠", Tokens::ui(13.0), warn);
    let n = c.errors.len();
    let i = c.current.min(n - 1);
    // ◀ i/n ▶ on the right.
    let next = Rect::from_center_size(pos2(rect.max.x - 14.0, rect.center().y), vec2(16.0, 16.0));
    let count = Rect::from_center_size(pos2(rect.max.x - 44.0, rect.center().y), vec2(40.0, 16.0));
    let prev = Rect::from_center_size(pos2(rect.max.x - 74.0, rect.center().y), vec2(16.0, 16.0));
    for (r, label, auto, d) in [(prev, "◀", "viewer.exprErrors.prev", n - 1), (next, "▶", "viewer.exprErrors.next", 1)] {
        let resp = ui.interact(r, egui::Id::new(("expr-bar", auto)), Sense::click());
        p.text(r.center(), Align2::CENTER_CENTER, label, Tokens::ui(11.0), if resp.hovered() { Color32::WHITE } else { t.text });
        app.auto.add(auto, r, label);
        if resp.clicked() {
            c.current = (i + d) % n;
            ctx.data_mut(|dd| dd.insert_temp(cache_id(), c.clone()));
        }
    }
    p.text(count.center(), Align2::CENTER_CENTER, format!("{}/{n}", i + 1), Tokens::ui(11.0), t.text);
    let comp_name = app.session.project.item(cid).map(|it| it.name.clone()).unwrap_or_default();
    let e = c.errors[i].clone();
    let msg_rect = Rect::from_min_max(pos2(rect.min.x + 26.0, rect.min.y), pos2(prev.min.x - 6.0, rect.max.y));
    let resp = ui.interact(msg_rect, egui::Id::new("expr-bar-msg"), Sense::click()).on_hover_text(tr("Click to reveal the expression in the timeline"));
    p.with_clip_rect(msg_rect).text(
        pos2(msg_rect.min.x, rect.center().y),
        Align2::LEFT_CENTER,
        e.describe(&comp_name),
        Tokens::ui(11.5),
        if resp.hovered() { Color32::WHITE } else { t.text },
    );
    app.auto.add("viewer.exprErrors", msg_rect, &e.describe(&comp_name));
    if resp.clicked() {
        let _ = app.session.execute("layer.select", json!({"comp": cid.0, "layers": [e.layer]}));
        let _ = app.session.execute("prop.select", json!({"comp": cid.0, "layer": e.layer, "prop": e.prop, "selectKeys": false}));
        let _ = crate::menus::frontend(app, &ctx, "timeline.revealProps", json!({"props": [{"layer": e.layer, "prop": e.prop}]}));
        app.ui.timeline.expr_closed.remove(&e.prop);
    }
}
