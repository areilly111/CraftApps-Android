//! History panel: the document's states; click one to go back (or forward) to it.

use egui::{Sense, Ui, vec2};
use serde_json::json;

use crate::theme::Tokens;
use crate::widgets::menu_item;
use crate::{VectorcraftApp, icons};

/// Undo (negative) / redo (positive) steps to reach row `target` when `current` is the current
/// row (row 0 = the document as opened).
pub fn steps_to(current: usize, target: usize) -> i64 {
    target as i64 - current as i64
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "history", tl!("No document"), tl!("Open a document to see its history."));
        return;
    };
    let mut rows: Vec<(String, bool)> = vec![("Open".to_string(), false)];
    rows.extend(st.history.undo.iter().map(|h| (h.label.clone(), false)));
    let current = rows.len() - 1;
    rows.extend(st.history.redo.iter().rev().map(|h| (h.label.clone(), true)));
    let mut steps: Option<i64> = None;
    crate::widgets::list_box(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("hist").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for (i, (label, future)) in rows.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if i == current {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let icon = if i == 0 { "file-plus" } else { "history" };
                icons::paint(
                    ui,
                    icon,
                    egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0)),
                    if *future { t.text_disabled } else { t.icon },
                );
                let col = if *future { t.text_disabled } else { t.text };
                ui.painter().text(r.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, tl!(label), egui::FontId::proportional(12.5), col);
                if resp.clicked() && i != current {
                    steps = Some(steps_to(current, i));
                }
            }
        });
    });
    if let Some(s) = steps {
        let (cmd, n) = if s < 0 { ("edit.undo", -s) } else { ("edit.redo", s) };
        for _ in 0..n {
            if app.run(cmd, json!({})).is_err() {
                break;
            }
        }
    }
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    menu_item(ui, tl!("History Options…"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps() {
        assert_eq!(steps_to(5, 2), -3);
        assert_eq!(steps_to(2, 4), 2);
        assert_eq!(steps_to(3, 3), 0);
    }
}
