//! Headless frames of the Stroke panel's Dashed Line section.

use serde_json::json;

use super::tests_appearance::{app_with_rect, click, frame_events, run, text_rect};
use super::*;

/// Click the Dashed Line check box and return the stroke's dash afterwards.
fn toggle_dashes(ctx: &egui::Context, app: &mut VectorcraftApp) -> Option<vectorcraft_doc::Dash> {
    let at = text_rect(&frame_events(ctx, app, vec![], stroke::show), "Dashed Line").center();
    click(ctx, app, at, stroke::show);
    current_stroke(app).unwrap().dash
}

#[test]
fn turning_dashes_on_fits_them_to_corners_and_turning_back_on_restores_the_last_ones() {
    let (ctx, mut app) = (egui::Context::default(), app_with_rect());
    let d = toggle_dashes(&ctx, &mut app).expect("dashes on");
    assert_eq!((d.pattern, d.align_corners), (vec![12.0, 12.0], true));
    // Exact 4/2 dashes turned off and on again come back as they were.
    run(&mut app, "stroke.set", json!({"dash": [4, 2], "alignDashes": false}));
    assert!(toggle_dashes(&ctx, &mut app).is_none());
    let d = toggle_dashes(&ctx, &mut app).expect("dashes on");
    assert_eq!((d.pattern, d.align_corners), (vec![4.0, 2.0], false));
}
