//! Puppet Warp pins: editing state the Puppet Warp tool and `object.puppetWarp` share while the
//! same artwork stays selected. It lives in the document (never saved) so Undo and Redo take the
//! pins back with the warps they made.

use std::sync::Arc;

use vectorcraft_geom::Point;

use crate::{Document, Node, NodeId};

/// One pin: where it sits on the rest shape, where it is now, and the turn it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PuppetPin {
    pub rest: Point,
    pub at: Point,
    /// Radians the art is turned around the pin; `None`: free to turn.
    pub angle: Option<f64>,
}

/// The pins on some objects and the shape they warp from.
#[derive(Clone, Debug, PartialEq)]
pub struct PuppetPins {
    /// The warped objects.
    pub ids: Vec<NodeId>,
    /// Their rest shape: the objects as they were when pinning began (the warp always starts
    /// from it, so warps don't stack and dragging a pin back restores the original).
    pub rest: Vec<Arc<Node>>,
    /// The objects as the last warp left them: once anything else edits them, the pins start
    /// again from the objects as they are.
    pub result: Vec<Arc<Node>>,
    pub pins: Vec<PuppetPin>,
}

impl PuppetPins {
    /// The pins are still those of `ids` in `doc`: the same objects, as the last warp left them.
    pub fn holds(&self, doc: &Document, ids: &[NodeId]) -> bool {
        self.ids == ids
            && self.ids.len() == self.result.len()
            && ids.iter().zip(&self.result).all(|(id, r)| doc.node_arc(*id).is_some_and(|a| Arc::ptr_eq(a, r)))
    }
}

impl Document {
    /// The node `id` as the document holds it (shared: comparing pointers says whether it changed).
    pub fn node_arc(&self, id: NodeId) -> Option<&Arc<Node>> {
        fn find(nodes: &[Arc<Node>], id: NodeId) -> Option<&Arc<Node>> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Some(f) = n.children().and_then(|ch| find(ch, id)) {
                    return Some(f);
                }
            }
            None
        }
        find(&self.layers, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Appearance;
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn pins_hold_until_their_objects_change_and_are_never_saved() {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        let node = d.node_arc(id).cloned().unwrap();
        let pin = PuppetPin { rest: Point::new(5.0, 5.0), at: Point::new(6.0, 5.0), angle: Some(0.5) };
        d.puppet = Some(Arc::new(PuppetPins { ids: vec![id], rest: vec![node.clone()], result: vec![node], pins: vec![pin] }));
        let pins = d.puppet.clone().unwrap();
        assert!(pins.holds(&d, &[id]));
        assert!(!pins.holds(&d, &[id, l]), "other objects");
        // Saved and read back without them (editing state).
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("puppet"));
        let back: Document = serde_json::from_str(&json).unwrap();
        assert!(back.puppet.is_none());
        assert_eq!(back.node(id), d.node(id));
        // An edit by anything else: the pins no longer hold.
        d.node_mut(id).unwrap().name = Some("moved".into());
        assert!(!pins.holds(&d, &[id]));
    }
}
