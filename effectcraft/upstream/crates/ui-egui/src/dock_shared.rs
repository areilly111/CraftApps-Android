//! Convert the persisted workspace schema to shared docking transactions.

use craft_ui::docking::{Action, Floating as SharedFloating, Layout as SharedLayout, Node, Placement, StackEntry as SharedStackEntry};
use craft_ui::layout::{SplitAxis, SplitSize as SharedSplitSize};

use crate::dock::{DockNode, Floating, Layout, PanelKind, SplitSize, StackEntry};

const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 4096;
const MAX_PANELS: usize = 128;

fn encode_node(node: &DockNode, depth: usize, count: &mut usize) -> Result<Node<PanelKind>, String> {
    *count = count.saturating_add(1);
    if depth > MAX_DEPTH || *count > MAX_NODES {
        return Err("workspace dock tree exceeds supported limits".into());
    }
    Ok(match node {
        DockNode::Tabs { panels, active } => {
            if panels.len() > MAX_PANELS {
                return Err("too many workspace tabs".into());
            }
            Node::Tabs { panels: panels.clone(), active: *active }
        }
        DockNode::Stack { entries } => {
            if entries.len() > MAX_PANELS {
                return Err("too many workspace stack entries".into());
            }
            if entries.iter().any(|entry| entry.panels.is_empty() || entry.active >= entry.panels.len() || entry.panels.len() > MAX_PANELS) {
                return Err("invalid stacked tab selection".into());
            }
            Node::Stack {
                entries: entries
                    .iter()
                    .flat_map(|entry| entry.panels.iter().map(move |panel| SharedStackEntry { panel: *panel, open: entry.open, height: entry.height }))
                    .collect(),
            }
        }
        DockNode::Split { vertical, size, a, b } => Node::Split {
            axis: if *vertical { SplitAxis::Vertical } else { SplitAxis::Horizontal },
            size: match *size {
                SplitSize::Ratio(value) => SharedSplitSize::Ratio(value),
                SplitSize::FixedA(value) => SharedSplitSize::FixedFirst(value),
                SplitSize::FixedB(value) => SharedSplitSize::FixedSecond(value),
            },
            first: Box::new(encode_node(a, depth.saturating_add(1), count)?),
            second: Box::new(encode_node(b, depth.saturating_add(1), count)?),
        },
    })
}

fn decode_node(node: Node<PanelKind>) -> DockNode {
    match node {
        Node::Tabs { panels, active } => DockNode::Tabs { panels, active },
        Node::Stack { entries } => {
            DockNode::Stack { entries: entries.into_iter().map(|entry| StackEntry::new(entry.panel, entry.open, entry.height)).collect() }
        }
        Node::Split { axis, size, first, second } => DockNode::Split {
            vertical: axis == SplitAxis::Vertical,
            size: match size {
                SharedSplitSize::Ratio(value) => SplitSize::Ratio(value),
                SharedSplitSize::FixedFirst(value) => SplitSize::FixedA(value),
                SharedSplitSize::FixedSecond(value) => SplitSize::FixedB(value),
            },
            a: Box::new(decode_node(*first)),
            b: Box::new(decode_node(*second)),
        },
    }
}

pub(crate) fn encode(root: &DockNode, floating: &[Floating]) -> Result<SharedLayout<PanelKind>, String> {
    if floating.len() > MAX_PANELS || floating.iter().any(|group| group.panels.len() > MAX_PANELS) {
        return Err("too many floating workspace panels".into());
    }
    let root = if matches!(root, DockNode::Tabs { panels, active: 0 } if panels.is_empty()) { None } else { Some(encode_node(root, 0, &mut 0)?) };
    Ok(SharedLayout {
        root,
        floating: floating.iter().map(|group| SharedFloating { panels: group.panels.clone(), active: group.active, rect: group.rect }).collect(),
    })
}

pub(crate) fn decode(layout: SharedLayout<PanelKind>) -> Layout {
    Layout {
        root: layout.root.map(decode_node).unwrap_or_else(|| DockNode::Tabs { panels: vec![], active: 0 }),
        floating: layout.floating.into_iter().map(|group| Floating { panels: group.panels, active: group.active, rect: group.rect }).collect(),
    }
}

/// Restore app-owned accordion tab frames after shared validation/transactions.
/// The shared model represents individual stack panels; grouping stays in the persisted app schema.
fn restore_frames(node: &mut DockNode, original: &DockNode) {
    fn collect(node: &DockNode, out: &mut Vec<StackEntry>) {
        match node {
            DockNode::Split { a, b, .. } => {
                collect(a, out);
                collect(b, out);
            }
            DockNode::Stack { entries } => out.extend(entries.iter().cloned()),
            DockNode::Tabs { .. } => {}
        }
    }
    fn restore(node: &mut DockNode, frames: &[StackEntry]) {
        match node {
            DockNode::Split { a, b, .. } => {
                restore(a, frames);
                restore(b, frames);
            }
            DockNode::Stack { entries } => {
                let mut result: Vec<StackEntry> = Vec::new();
                for entry in std::mem::take(entries) {
                    let Some(panel) = entry.shown() else { continue };
                    let old = frames.iter().find(|frame| frame.contains(panel));
                    if let Some(last) = result.last_mut()
                        && old.is_some_and(|frame| last.panels.iter().all(|p| frame.contains(*p)))
                    {
                        last.panels.push(panel);
                        if old.and_then(StackEntry::shown) == Some(panel) {
                            last.active = last.panels.len().saturating_sub(1);
                        }
                    } else {
                        result.push(entry);
                    }
                }
                for entry in &mut result {
                    if let Some(old) = frames.iter().find(|frame| entry.panels.iter().all(|panel| frame.contains(*panel))) {
                        entry.active = old
                            .shown()
                            .and_then(|shown| entry.panels.iter().position(|panel| *panel == shown))
                            .unwrap_or(old.active.min(entry.panels.len().saturating_sub(1)));
                    }
                }
                *entries = result;
            }
            DockNode::Tabs { .. } => {}
        }
    }
    let mut frames = Vec::new();
    collect(original, &mut frames);
    restore(node, &frames);
}

pub(crate) fn apply(layout: &mut Layout, action: Action<PanelKind>) -> Result<(), String> {
    let mut shared = encode(&layout.root, &layout.floating)?;
    // Validate the same shared command first. Native stack insertion then retains tabs inside
    // their accordion frame, including the existing before/after insertion policy.
    shared.apply(action.clone()).map_err(|error| error.to_string())?;
    let mut proposed = decode(shared);
    restore_frames(&mut proposed.root, &layout.root);
    match action {
        Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } if layout.root.contains(anchor) => {
            fn stacked(node: &DockNode, p: PanelKind) -> bool {
                match node {
                    DockNode::Split { a, b, .. } => stacked(a, p) || stacked(b, p),
                    DockNode::Stack { entries } => entries.iter().any(|e| e.contains(p)),
                    _ => false,
                }
            }
            if stacked(&layout.root, anchor) {
                let mut native = layout.clone();
                native.unfloat(panel);
                let applied = match placement {
                    Placement::Tab { before } => native.root.insert_tab(panel, anchor, before),
                    Placement::Split(zone) => native.root.dock_panel(
                        panel,
                        anchor,
                        match zone {
                            craft_ui::docking::Zone::Left => crate::dock::Zone::Left,
                            craft_ui::docking::Zone::Right => crate::dock::Zone::Right,
                            craft_ui::docking::Zone::Top => crate::dock::Zone::Top,
                            craft_ui::docking::Zone::Bottom => crate::dock::Zone::Bottom,
                            craft_ui::docking::Zone::Center => crate::dock::Zone::Center,
                        },
                    ),
                };
                if !applied {
                    return Err("stacked panel move failed".into());
                }
                proposed = native;
            }
        }
        Action::Activate { panel } => {
            proposed.root.activate(panel);
        }
        Action::SetStackOpen { panel, open } => {
            fn set(node: &mut DockNode, p: PanelKind, open: bool) {
                match node {
                    DockNode::Split { a, b, .. } => {
                        set(a, p, open);
                        set(b, p, open);
                    }
                    DockNode::Stack { entries } => {
                        if let Some(e) = entries.iter_mut().find(|e| e.contains(p)) {
                            e.open = open;
                        }
                    }
                    _ => {}
                }
            }
            set(&mut proposed.root, panel, open);
        }
        Action::ResizeStack { panel, height } => {
            fn set(node: &mut DockNode, p: PanelKind, h: Option<f32>) {
                match node {
                    DockNode::Split { a, b, .. } => {
                        set(a, p, h);
                        set(b, p, h);
                    }
                    DockNode::Stack { entries } => {
                        if let Some(e) = entries.iter_mut().find(|e| e.contains(p)) {
                            e.height = h;
                        }
                    }
                    _ => {}
                }
            }
            set(&mut proposed.root, panel, height);
        }
        _ => {}
    }
    encode(&proposed.root, &proposed.floating)?.validate().map_err(|error| error.to_string())?;
    *layout = proposed;
    Ok(())
}

pub(crate) fn apply_tree(root: &mut DockNode, action: Action<PanelKind>) -> bool {
    if !valid_tree(root) {
        return false;
    }
    let mut layout = Layout { root: root.clone(), floating: vec![] };
    if apply(&mut layout, action).is_err() {
        return false;
    }
    *root = layout.root;
    true
}

pub(crate) fn valid_tree(root: &DockNode) -> bool {
    encode(root, &[]).is_ok_and(|shared| shared.validate().is_ok())
}

pub(crate) fn valid_for_close(root: &DockNode) -> bool {
    fn normalize(node: &mut Node<PanelKind>) {
        match node {
            Node::Tabs { panels, active } => *active = (*active).min(panels.len().saturating_sub(1)),
            Node::Split { first, second, .. } => {
                normalize(first);
                normalize(second);
            }
            Node::Stack { .. } => {}
        }
    }
    let Ok(mut shared) = encode(root, &[]) else { return false };
    if let Some(root) = &mut shared.root {
        normalize(root);
    }
    shared.validate().is_ok()
}

pub(crate) fn unfloat(layout: &mut Layout, panel: PanelKind) -> bool {
    let Ok(mut shared) = encode(&layout.root, &layout.floating) else { return false };
    let mut found = false;
    for group in &mut shared.floating {
        if group.panels.contains(&panel) {
            group.active = group.active.min(group.panels.len().saturating_sub(1));
            found = true;
        }
    }
    if !found || shared.apply(Action::Close { panel }).is_err() {
        return false;
    }
    let mut proposed = decode(shared);
    restore_frames(&mut proposed.root, &layout.root);
    *layout = proposed;
    true
}

#[cfg(test)]
mod stacked_frame_tests {
    use super::*;
    use crate::dock::Zone;
    #[test]
    fn stacked_tabs_survive_shared_resize_activate_float_and_close() {
        let mut root = crate::dock::workspace("Default");
        assert!(root.dock_panel(PanelKind::Info, PanelKind::Properties, Zone::Center));
        fn group(node: &DockNode) -> Option<&StackEntry> {
            match node {
                DockNode::Split { a, b, .. } => group(a).or_else(|| group(b)),
                DockNode::Stack { entries } => entries.iter().find(|e| e.contains(PanelKind::Properties)),
                _ => None,
            }
        }
        let before = group(&root).unwrap().clone();
        assert!(before.contains(PanelKind::Info));
        let mut layout = Layout { root, floating: vec![] };
        layout.apply(Action::ResizeSplit { path: vec![], size: SharedSplitSize::Ratio(0.6) }).unwrap();
        assert_eq!(group(&layout.root).unwrap(), &before);
        layout.apply(Action::Activate { panel: PanelKind::Properties }).unwrap();
        assert_eq!(group(&layout.root).unwrap().shown(), Some(PanelKind::Properties));
        let original_frame = group(&layout.root).unwrap().clone();
        layout.apply(Action::OpenAt { panel: PanelKind::History, anchor: PanelKind::Properties, placement: Placement::Tab { before: None } }).unwrap();
        let opened = group(&layout.root).unwrap();
        assert!(opened.contains(PanelKind::History));
        assert!(opened.contains(PanelKind::Info));
        assert_eq!(opened.shown(), Some(PanelKind::History));
        assert_eq!(opened.height, original_frame.height);
        layout.apply(Action::Close { panel: PanelKind::History }).unwrap();
        layout.apply(Action::Float { panel: PanelKind::Info, rect: [20.0, 20.0, 300.0, 300.0] }).unwrap();
        assert_eq!(group(&layout.root).unwrap().panels, vec![PanelKind::Properties]);
        layout.apply(Action::Close { panel: PanelKind::Info }).unwrap();
        assert!(layout.floating.is_empty());
        assert_eq!(group(&layout.root).unwrap().panels, vec![PanelKind::Properties]);
    }
}
