//! Content-Aware Fill panel (Window ▸ Content-Aware Fill): Fill Target, Alpha Expansion, Fill
//! Method (Object / Surface / Edge Blend), Lighting Correction, Range (Work Area / Entire
//! Duration), a reference frame from a layer of the comp, and Generate Fill Layer. The settings
//! live in `EditorState::content_fill` (`contentFill.set`); the button runs
//! `contentFill.generate` as a background job (Window ▸ Progress).

use crate::i18n::tr;
use egui::{Align2, Rect, pos2, vec2};
use serde_json::json;

use super::panel_kit as kit;
use crate::EffectcraftApp;
use crate::theme::Tokens;

const ROW: f32 = 28.0;

pub fn show(app: &mut EffectcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, t.panel_bg);
    let x0 = rect.min.x + 12.0;
    let xv = x0 + 130.0;
    let w = (rect.max.x - xv - 12.0).clamp(90.0, 220.0);
    let mut y = rect.min.y + 10.0;
    let st = app.session.state.content_fill.clone();
    let comp = app.session.active_comp_arc();
    let layer = comp.as_ref().and_then(|c| app.session.state.selected_layers.first().and_then(|l| c.layer(*l)).cloned());
    // Fill Target.
    p.text(pos2(x0, y + 9.0), Align2::LEFT_CENTER, tr("Fill Target"), Tokens::semibold(12.0), t.text);
    y += ROW - 6.0;
    let target = match &layer {
        Some(l) => {
            let masks = l
                .masks()
                .map(|m| {
                    m.groups()
                        .filter(|g| {
                            matches!(
                                g.kind,
                                effectcraft_engine::project::GroupKind::Mask { mode: effectcraft_engine::project::MaskMode::Subtract, .. }
                                    | effectcraft_engine::project::GroupKind::Mask { inverted: true, .. }
                            )
                        })
                        .count()
                })
                .unwrap_or(0);
            format!("{} — {masks} subtracting mask(s)", l.name)
        }
        None => "Select a layer with a Subtract mask".into(),
    };
    p.text(pos2(x0, y + 9.0), Align2::LEFT_CENTER, &target, Tokens::ui(11.5), if layer.is_some() { t.text_dim } else { t.text_faint });
    app.auto.add("contentFill.target", Rect::from_min_size(pos2(x0, y), vec2(rect.width() - 24.0, 18.0)), &target);
    y += ROW;
    let mut set = serde_json::Map::new();
    // Alpha Expansion.
    kit::label(ui, pos2(x0, y), "Alpha Expansion:", &t);
    let a = kit::number(app, ui, pos2(xv, y), "contentFill.alphaExpansion", st.alpha_expansion, 0.2, (0.0, 100.0), 0, " px");
    if (a - st.alpha_expansion).abs() > 1e-9 {
        set.insert("alphaExpansion".into(), json!(a.round()));
    }
    y += ROW;
    // Fill Method.
    kit::label(ui, pos2(x0, y), "Fill Method:", &t);
    let methods = ["object", "surface", "edgeBlend"];
    let cur = methods.iter().position(|m| *m == st.method).unwrap_or(0);
    if let Some(i) = kit::dropdown(app, ui, Rect::from_min_size(pos2(xv, y), vec2(w, 20.0)), "contentFill.method", &["Object", "Surface", "Edge Blend"], cur) {
        set.insert("method".into(), json!(methods[i]));
    }
    y += ROW;
    // Lighting Correction (Object / Surface).
    kit::label(ui, pos2(x0, y), "Lighting Correction:", &t);
    let lights = ["none", "subtle", "moderate", "strong"];
    let cur = lights.iter().position(|m| *m == st.lighting_correction).unwrap_or(0);
    if st.method != "edgeBlend"
        && let Some(i) =
            kit::dropdown(app, ui, Rect::from_min_size(pos2(xv, y), vec2(w, 20.0)), "contentFill.lighting", &["None", "Subtle", "Moderate", "Strong"], cur)
    {
        set.insert("lightingCorrection".into(), json!(lights[i]));
    }
    y += ROW;
    // Range.
    kit::label(ui, pos2(x0, y), "Range:", &t);
    let cur = usize::from(st.range == "entire");
    if let Some(i) = kit::dropdown(app, ui, Rect::from_min_size(pos2(xv, y), vec2(w, 20.0)), "contentFill.range", &["Work Area", "Entire Duration"], cur) {
        set.insert("range".into(), json!(if i == 1 { "entire" } else { "workArea" }));
    }
    y += ROW;
    // Reference frame: a layer of the comp at a time.
    kit::label(ui, pos2(x0, y), "Reference Frame:", &t);
    if let Some(c) = &comp {
        let mut names = vec!["None".to_string()];
        names.extend(c.layers.iter().map(|l| l.name.clone()));
        let cur = st.reference_layer.and_then(|r| c.layers.iter().position(|l| l.id == r)).map(|i| i + 1).unwrap_or(0);
        let opts: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Some(i) = kit::dropdown(app, ui, Rect::from_min_size(pos2(xv, y), vec2(w, 20.0)), "contentFill.referenceLayer", &opts, cur) {
            set.insert("referenceLayer".into(), if i == 0 { json!(null) } else { json!(c.layers[i - 1].id.0) });
        }
        y += ROW;
        if st.reference_layer.is_some() {
            let at = st.reference_time.unwrap_or(0.0);
            p.text(pos2(xv, y + 9.0), Align2::LEFT_CENTER, format!("at {}", kit::seconds(at)), Tokens::ui(11.5), t.text_dim);
            if kit::button(app, ui, Rect::from_min_size(pos2(xv + 80.0, y), vec2(120.0, 20.0)), "contentFill.referenceHere", "Use Current Time", false) {
                set.insert("referenceTime".into(), json!(app.session.time().seconds()));
            }
            y += ROW;
        }
    } else {
        y += ROW;
    }
    if !set.is_empty() {
        kit::exec(app, "contentFill.set", serde_json::Value::Object(set));
    }
    // Generate, or the running job.
    y += 6.0;
    let running = app.session.jobs().into_iter().find(|j| j.kind == "contentFill");
    match running {
        Some(j) => {
            let frac = j.fraction.unwrap_or(0.0) as f32;
            let bar = Rect::from_min_size(pos2(x0, y + 7.0), vec2((rect.width() - 110.0).max(60.0), 6.0));
            p.rect_filled(bar, 3.0, t.field_bg);
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, 6.0)), 3.0, t.accent);
            p.text(pos2(x0, y + 24.0), Align2::LEFT_CENTER, format!("{} {:.0} %", j.message, frac * 100.0), Tokens::ui(11.0), t.text_dim);
            if kit::button(app, ui, Rect::from_min_size(pos2(bar.max.x + 10.0, y), vec2(70.0, 20.0)), "contentFill.cancel", "Cancel", false) {
                kit::exec(app, "jobs.cancel", json!({"job": j.id}));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        None => {
            if kit::button(app, ui, Rect::from_min_size(pos2(x0, y), vec2(160.0, 24.0)), "contentFill.generate", "Generate Fill Layer", true) {
                match &layer {
                    Some(l) => {
                        kit::exec(app, "contentFill.generate", json!({"layer": l.id.0}));
                    }
                    None => app.ui.status = "Select the layer to fill first".into(),
                }
            }
        }
    }
}
