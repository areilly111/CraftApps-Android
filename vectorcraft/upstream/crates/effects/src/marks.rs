//! Effect → Crop Marks: live trim marks in the Registration colour around the object, which follow
//! it as it moves or changes size. The renderer, the exporters ([`crate::bake_document`]) and
//! Expand Appearance draw the object as [`crop_marks_art`].

use std::sync::Arc;

use vectorcraft_doc::Node;
use vectorcraft_doc::marks::{MarkStyle, TrimMarks};

/// The effect id.
pub const CROP_MARKS: &str = "cropMarks";

/// Does `n` carry a visible Crop Marks effect?
pub fn has_crop_marks(n: &Node) -> bool {
    n.appearance.effects.iter().any(|e| e.visible && e.id == CROP_MARKS)
}

/// The art of an object with Crop Marks: a group (with the object's id, name and transparency) of
/// the object itself, without the effect, and the marks around its visual bounds (the last Crop
/// Marks effect's style). `None` without a visible Crop Marks effect.
pub fn crop_marks_art(n: &Node) -> Option<Node> {
    let fx = n.appearance.effects.iter().rev().find(|e| e.visible && e.id == CROP_MARKS)?;
    let style = fx.params.get("style").and_then(|v| v.as_str()).and_then(MarkStyle::parse).unwrap_or_default();
    let bounds = n.visual_bounds()?;
    let marks = TrimMarks::of(style).group(bounds, &mut || n.id);
    let mut object = n.clone();
    object.appearance.effects.retain(|e| e.id != CROP_MARKS);
    // The group takes over the object's identity, transparency, mask and link.
    let mut g = Node::group(n.id, vec![]);
    (g.visible, g.locked) = (n.visible, n.locked);
    g.name = object.name.take();
    g.opacity = std::mem::replace(&mut object.opacity, 1.0);
    g.blend = std::mem::take(&mut object.blend);
    g.isolate = std::mem::take(&mut object.isolate);
    g.mask = object.mask.take();
    g.attrs = object.attrs.take();
    if let Some(ch) = g.children_mut() {
        *ch = vec![Arc::new(object), Arc::new(marks)];
    }
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::{Appearance, Effect, NodeId};
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn the_marks_surround_the_object_and_follow_it() {
        let mut n = Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0)), Appearance::default_art());
        assert!(crop_marks_art(&n).is_none());
        n.opacity = 0.5;
        n.appearance.effects.push(Effect { id: CROP_MARKS.into(), params: json!({"style": "japanese"}), visible: true });
        let art = crop_marks_art(&n).unwrap();
        assert_eq!((art.id, art.opacity), (NodeId(7), 0.5));
        let [object, marks] = art.children().unwrap().as_slice() else { panic!() };
        assert!(object.appearance.effects.is_empty() && object.opacity == 1.0);
        assert_eq!(marks.children().unwrap().len(), 24, "Japanese marks");
        // Around the stroked bounds (0.5 outside the path), by the marks' reach.
        let reach = TrimMarks::of(MarkStyle::Japanese).reach() + 0.5;
        let b = art.visual_bounds().unwrap();
        assert!((b.x0 + reach).abs() < 2.0 && (b.x1 - 100.0 - reach).abs() < 2.0, "{b:?}");
        // Moved, the marks move along.
        n.transform(vectorcraft_geom::Affine::translate((40.0, 0.0)), false);
        let b2 = crop_marks_art(&n).unwrap().visual_bounds().unwrap();
        assert!((b2.x0 - b.x0 - 40.0).abs() < 1e-6);
    }
}
