//! File (document-level) and Edit commands, plus document queries.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, Unit};
use vectorcraft_geom::{Affine, Vec2};

use super::clipboard::SwatchChoices;
use super::*;
use crate::{Clipboard, EngineError, inspect};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.new",
            "New…",
            ["File"],
            None,
            "{preset?: a name from file.newPresets (start from it; the other params change it), name?|title?: document name (default Untitled-N), width?: pt=612|\"210 mm\", height?: pt=792, units?: \"Pixels\"|\"Points\"|\"Picas\"|\"Inches\"|\"Millimeters\"|\"Centimeters\"|\"Feet\"|\"Yards\"|\"Meters\"|\"Feet & Inches\" (default: prefs unitsGeneral, as print presets; screen presets: Pixels), orientation?: \"portrait\"|\"landscape\" (swaps width and height to match), artboards?: n (1–1000), artboardLayout?: {layout?: \"gridByRow\"|\"gridByColumn\"|\"row\"|\"column\", columns?: n (default: all in one row), spacing?: pt=20, rightToLeft?: bool}, bleed?: pt|[top, bottom, left, right]|{top?, …} (0–72 pt), backgroundContents?: \"transparent\"|\"white\" (a white artboard background, not an object), colorMode?: \"rgb\"|\"cmyk\" (a CMYK document starts with CMYK swatches and stores the colours applied to it as CMYK), rasterEffectsPpi?: 72|150|300 (1–2400), previewMode?: \"default\"|\"pixel\"|\"overprint\" (overprint turns Overprint Preview on, pixel Pixel Preview in the app; default leaves both), created?: Unix seconds|null (File Info's created date; default now, recorded in the journal so a replay matches)} → {index, previewMode}; the size is remembered in file.newPresets' Recent",
            always,
            super::newdoc::file_new
        ),
        cmd!("file.close", "Close", ["File"], Some("Cmd+W"), "{index?}", has_doc, file_close),
        cmd!("document.activate", "Activate Document", [], None, "{index}", always, doc_activate),
        cmd!(query "document.inspect", "Inspect Document", [], None, "{depth?, childLimit?} → layer tree, artboards, selection, history (a sliced tree reports childCount)", has_doc, |s, p| Ok(inspect::document_opts(s, slice_opts(p, "document.inspect")?))),
        cmd!(query "document.node", "Inspect Object", [], None, "{id, summary?: compact summary, depth?: child levels in the summary (default all), childLimit?: children shown per node (default all; a level that shows fewer reports childCount)} → one object", has_doc, doc_node),
        cmd!(query "document.find", "Find Objects", [], None, "{name?: substring of the Layers panel name, kind?: panel label such as Group, Path, Type or Image (exact), text?: substring of type content, limit?: max matches (default 100, 0 counts only)} → {matches: [{id, name, kind, path}], total}", has_doc, doc_find),
        cmd!(query "document.json", "Document JSON", [], None, "{} → complete document model", has_doc, |s, _| Ok(serde_json::to_value(&*s.doc()?.doc).unwrap_or(Value::Null))),
        cmd!(
            "document.setUnits",
            "Units",
            ["File", "Document Setup"],
            None,
            "{units: \"Points\"|\"Picas\"|\"Inches\"|\"Millimeters\"|\"Centimeters\"|\"Pixels\"|\"Feet & Inches\"|\"Meters\"|\"Yards\"|\"Feet\"} the document's units: every length the UI shows and reads (General)",
            has_doc,
            set_units
        ),
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, undo),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, redo),
        cmd!("edit.cut", "Cut", ["Edit"], Some("Cmd+X"), "{} (the Artboard tool chosen: artboard.cut)", has_selection_or_artboard_tool, cut),
        cmd!("edit.copy", "Copy", ["Edit"], Some("Cmd+C"), "{} (the Artboard tool chosen: artboard.copy)", has_selection_or_artboard_tool, copy),
        cmd!(
            "edit.paste",
            "Paste",
            ["Edit"],
            Some("Cmd+V"),
            "{center?: [x, y], dx?, dy?, swatchConflict?} paste centred on `center` (the app passes the view centre), else offset by dx/dy (default: the Paste Offset preference). Pasting brings the image blobs, symbols, patterns, global and spot swatches (with the tint swatches of the tints used), gradient swatches, graphic styles, character and paragraph styles and brushes the objects use; one of the same name that differs comes in renamed. swatchConflict, for a swatch whose name the document gives another colour (clipboard.conflicts): \"merge\" (default: the objects take the document's swatch) | \"add\" (the pasted swatch comes in renamed) | {name: \"merge\"|\"add\"}. With Paste Remembers Layers on (layer.pasteRemembersLayers), objects go back into the layers they came from (by name; made when missing). An artboard copied with artboard.copy comes with its art, right of the last artboard (the other paste commands but Paste on All Artboards: where it was), its art back in the layers it came from → {ids, artboard: the new artboard's index or null, added: resources added, merged: conflicts merged, renamed: [{kind, from, to}]}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Offset)
        ),
        cmd!(
            "edit.pasteInFront",
            "Paste in Front",
            ["Edit"],
            Some("Cmd+F"),
            "{swatchConflict?, artboard?: index (the active artboard: the objects go where they were on the artboard they were copied from, on this one)} paste in place just above the top selected object, or on top of the current layer when nothing is selected (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Front)
        ),
        cmd!(
            "edit.pasteInBack",
            "Paste in Back",
            ["Edit"],
            Some("Cmd+B"),
            "{swatchConflict?, artboard?: index (as edit.pasteInFront)} paste in place just below the bottom selected object, or at the bottom of the current layer when nothing is selected (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::Back)
        ),
        cmd!(
            "edit.pasteInPlace",
            "Paste in Place",
            ["Edit"],
            Some("Cmd+Shift+V"),
            "{swatchConflict?, artboard?: index (as edit.pasteInFront)} paste where the objects were copied (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::InPlace)
        ),
        cmd!(
            "edit.pasteOnAllArtboards",
            "Paste on All Artboards",
            ["Edit"],
            Some("Cmd+Alt+Shift+V"),
            "{swatchConflict?} paste a copy on every artboard at the offset the objects had to the artboard they were copied from (resources and layers as edit.paste) → {ids, added, merged, renamed}",
            has_clipboard,
            |s, p| paste(s, p, PasteMode::AllArtboards)
        ),
        cmd!(
            "edit.clear",
            "Clear",
            ["Edit"],
            Some("Delete"),
            "{ids?} delete the objects `ids`, else the selection (direct-selected anchors, else objects, else ruler guides)",
            has_selection_or_guides,
            clear
        ),
        cmd!("edit.duplicate", "Duplicate", [], None, "{dx?, dy?} duplicate the selection in place (offset optional)", has_selection, duplicate),
    ]
}

fn file_close(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
    if !s.close_document(i) {
        return Err(bad("file.close", "no such document"));
    }
    ok()
}

fn doc_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad("document.activate", "missing index"))? as usize;
    if s.set_active(i) { ok() } else { Err(bad("document.activate", "no such document")) }
}

fn doc_node(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id").ok_or_else(|| bad("document.node", "missing id"))?;
    let opts = slice_opts(p, "document.node")?;
    let summary = bool_or(p, "summary", false);
    if !summary && (opts.depth.is_some() || opts.child_limit.is_some()) {
        // The full object JSON is never truncated: a slice without `summary` would
        // otherwise silently return the whole subtree.
        return Err(bad("document.node", "`depth` and `childLimit` slice the summary: pass `summary: true`"));
    }
    let n = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
    if summary { Ok(inspect::node_summary_opts(n, opts)) } else { Ok(serde_json::to_value(n).unwrap_or(Value::Null)) }
}

/// `depth` / `childLimit` for a summary read: absent (or null) means no limit.
fn slice_opts(p: &Value, cmd: &str) -> Result<inspect::SummaryOpts> {
    Ok(inspect::SummaryOpts { depth: opt_u64(p, cmd, "depth")?, child_limit: opt_u64(p, cmd, "childLimit")? })
}

/// An optional count: absent (or null) is `None`; a present value that is not a
/// non-negative integer is rejected rather than silently reinterpreted.
fn opt_u64(p: &Value, cmd: &str, key: &str) -> Result<Option<u64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a non-negative integer"))),
    }
}

fn doc_find(s: &mut Session, p: &Value) -> Result<Value> {
    // A filter of the wrong type is an error, not dropped: dropping it would widen the search.
    let needle = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.to_lowercase()).filter(|v| !v.is_empty())),
        Some(_) => Err(bad("document.find", format!("`{key}` must be a string"))),
    };
    let filter = inspect::FindFilter { name: needle("name")?, kind: needle("kind")?, text: needle("text")? };
    if filter.is_empty() {
        return Err(bad("document.find", "give at least one of `name`, `kind`, `text`"));
    }
    let limit = opt_u64(p, "document.find", "limit")?.unwrap_or(100);
    let (matches, total) = inspect::find_nodes(&s.doc()?.doc, &filter, limit);
    Ok(json!({"matches": matches, "total": total}))
}

/// Document Setup's units (also `document.setup {units}`).
fn set_units(s: &mut Session, p: &Value) -> Result<Value> {
    let u = str_param(p, "units").and_then(Unit::named).ok_or_else(|| bad("document.setUnits", "unknown units"))?;
    s.set_document_units(u)?;
    ok()
}

/// A typing session in progress (the Type tool previews the whole session as one interaction).
pub(crate) fn typing_in_progress(s: &Session) -> bool {
    s.active().and_then(|d| d.interaction.as_ref()).is_some_and(|it| it.label == "Typing" && it.preview.is_some())
}

fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    // What is still in progress (typing, a drag) is the step to undo: keep it first, so Undo takes
    // back only that and Redo brings it back, instead of dropping it and undoing the step before.
    // A typing session also ends in the Type tool, whose caret and marked text (IME) must follow
    // the document.
    let typing = typing_in_progress(s);
    s.commit_interaction()?;
    if typing {
        s.set_tool_option("endTyping", &Value::Bool(true));
    }
    let st = s.doc_mut()?;
    let e = st.history.undo.pop().ok_or_else(|| EngineError::Other("nothing to undo".into()))?;
    let label = e.label.clone();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    st.history.redo.push(crate::HistoryEntry {
        label: e.label,
        doc: std::mem::replace(&mut st.doc, e.doc),
        selection: std::mem::replace(&mut st.selection, e.selection),
    });
    st.revision += 1;
    st.selection.prune(&st.doc);
    super::maskedit::follow_history(st, editing);
    Ok(json!({ "undone": label }))
}

fn redo(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    let e = st.history.redo.pop().ok_or_else(|| EngineError::Other("nothing to redo".into()))?;
    let label = e.label.clone();
    let editing = st.doc.mask_edit.map(|m| m.layer);
    st.history.undo.push(crate::HistoryEntry {
        label: e.label,
        doc: std::mem::replace(&mut st.doc, e.doc),
        selection: std::mem::replace(&mut st.selection, e.selection),
    });
    st.revision += 1;
    st.selection.prune(&st.doc);
    super::maskedit::follow_history(st, editing);
    Ok(json!({ "redone": label }))
}

/// Selected top-level objects in paint order, dropping any whose ancestor is also selected.
pub(crate) fn selected_roots(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(roots_of(&st.doc, st.selection.in_paint_order(&st.doc)))
}

/// The top-level objects among `ids` (in paint order): compound-path members stand for their
/// compound, and ids whose ancestor is also listed, and layers, are dropped.
pub(crate) fn roots_of(doc: &Document, ids: Vec<NodeId>) -> Vec<NodeId> {
    // A compound-path member acts as its compound (compounds are one object).
    let mut sel: Vec<NodeId> = ids
        .into_iter()
        .map(|id| match doc.parent_of(id).and_then(|p| doc.node(p).map(|n| (p, n))) {
            Some((p, n)) if matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. }) => p,
            _ => id,
        })
        .collect();
    sel.dedup();
    sel.iter()
        .copied()
        .filter(|id| {
            let anc = doc.ancestry(*id).unwrap_or_default();
            !anc[..anc.len().saturating_sub(1)].iter().any(|a| sel.contains(a))
        })
        .filter(|id| doc.node(*id).is_some_and(|n| !n.is_layer()))
        .collect()
}

fn copy(s: &mut Session, _: &Value) -> Result<Value> {
    // The Artboard tool copies its artboard (and the art on it).
    if s.tool_id() == "artboard" {
        return super::panelcmds::artboard_copy(s, &json!({}), false);
    }
    let roots = selected_roots(s)?;
    s.clipboard = Clipboard::copy(s.doc()?, &roots);
    Ok(json!({ "copied": s.clipboard.nodes.len() }))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    if s.tool_id() == "artboard" {
        return super::panelcmds::artboard_copy(s, &json!({}), true);
    }
    // Copy promotes a compound member to its compound. Cut must remove the same objects it
    // copied, while Clear on its own deletes exactly the selected members.
    let ids = selected_roots(s)?;
    copy(s, p)?;
    if s.doc()?.selection.anchors.is_empty() { clear(s, &json!({ "ids": ids })) } else { clear(s, &json!({})) }
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    let explicit = if p.get("ids").is_some() { Some(checked_ids_param(s, p, "ids", "edit.clear")?) } else { None };
    // Direct-selected anchors: delete those anchors instead of whole objects.
    if explicit.is_none() && !s.doc()?.selection.anchors.is_empty() {
        return super::path::delete_anchors(s, p);
    }
    // Selected ruler guides (selected on their own).
    if explicit.is_none() && s.doc()?.selection.is_empty() {
        return super::docmenu::guide_remove(s, &json!({}));
    }
    let st = s.doc()?;
    // Guides selected with the art go with it (not when ids are given).
    let with_guides = explicit.is_none();
    let targets = explicit.unwrap_or_else(|| st.selection.in_paint_order(&st.doc));
    // Keep the selected ids themselves, without promoting compound members. Delete a selected
    // ancestor just once; its selected descendants leave with it. Layers themselves are kept.
    let targets: BTreeSet<_> = targets.into_iter().filter(|id| st.doc.node(*id).is_some_and(|n| !n.is_layer())).collect();
    let ids: Vec<_> = targets
        .iter()
        .copied()
        .filter(|id| !st.doc.ancestry(*id).unwrap_or_default().iter().rev().skip(1).any(|ancestor| targets.contains(ancestor)))
        .collect();
    if ids.is_empty() {
        return ok();
    }
    s.edit("Clear", |d, sel| {
        for id in &ids {
            d.remove(*id)?;
        }
        if with_guides {
            let gone = sel.guides.clone();
            d.retain_guides(sel, |i, _| !gone.contains(&i));
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PasteMode {
    Offset,
    Front,
    Back,
    InPlace,
    AllArtboards,
    /// A graphic from the Libraries panel: centred where it is placed, as Paste centres.
    Library,
}

impl PasteMode {
    fn label(self) -> &'static str {
        match self {
            PasteMode::Front => "Paste in Front",
            PasteMode::Back => "Paste in Back",
            PasteMode::InPlace => "Paste in Place",
            PasteMode::AllArtboards => "Paste on All Artboards",
            PasteMode::Offset => "Paste",
            PasteMode::Library => "Place from Library",
        }
    }
}

fn paste(s: &mut Session, p: &Value, mode: PasteMode) -> Result<Value> {
    let choices = SwatchChoices::parse(mode.label(), p)?;
    // Taken out of the session for the edit (and put back whatever happens).
    let clip = std::mem::take(&mut s.clipboard);
    let r = paste_clip(s, p, mode, &clip, &choices);
    s.clipboard = clip;
    r
}

/// Place the objects of `clip` (a library graphic) into the active document centred on `center`,
/// as one undo step: their resources come along as a paste's do, a swatch whose name the document
/// gives another colour merged into the document's.
pub(crate) fn place_clip(s: &mut Session, clip: &Clipboard, center: vectorcraft_geom::Point) -> Result<Value> {
    let choices = SwatchChoices::parse(PasteMode::Library.label(), &json!({}))?;
    paste_clip(s, &json!({ "center": [center.x, center.y] }), PasteMode::Library, clip, &choices)
}

/// Paste in Place, in Front or in Back onto artboard `artboard` (the active one, #693): where the
/// objects were on the artboard they were copied from, on that one; without it (or a copy from no
/// artboard), where they were.
fn onto_artboard(doc: &Document, p: &Value, clip: &Clipboard) -> Affine {
    let target = p.get("artboard").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).and_then(|i| doc.artboards.get(i));
    match (target, clip.source_artboard) {
        (Some(t), Some(src)) => Affine::translate(t.rect.origin() - src.origin()),
        _ => Affine::IDENTITY,
    }
}

fn paste_clip(s: &mut Session, p: &Value, mode: PasteMode, clip: &Clipboard, choices: &SwatchChoices) -> Result<Value> {
    let off = s.prefs.paste_offset;
    let st = s.doc()?;
    let same_doc = clip.source_doc == Some(st.uid);
    let parent = st.target_parent()?;
    // A copied artboard comes back with its art: right of the last artboard, or where it was
    // (Paste in Place, in Front, in Back). Paste on All Artboards pastes only the art.
    let board = clip
        .artboard
        .as_ref()
        .filter(|_| mode != PasteMode::AllArtboards)
        .map(|a| (a, if mode == PasteMode::Offset { super::panelcmds::beside_artboards(&st.doc, a.rect) } else { Vec2::ZERO }));
    // Paste Remembers Layers (not while isolating a group: pastes stay in it); an artboard's art
    // always goes back into its layers.
    let remember = (st.doc.paste_remembers_layers || board.is_some()) && parent.is_some_and(|p| st.doc.node(p).is_some_and(Node::is_layer));
    // Front/back: relative to the selection (top-most / bottom-most selected object); with
    // nothing selected, the top / bottom of the current layer.
    let anchor = match mode {
        PasteMode::Front | PasteMode::Back => {
            let order = st.selection.in_paint_order(&st.doc);
            let a = if mode == PasteMode::Front { order.last() } else { order.first() };
            a.and_then(|id| st.doc.position(*id))
        }
        _ => None,
    };
    let placements: Vec<Affine> = match (mode, board) {
        (_, Some((_, dv))) => vec![Affine::translate(dv)],
        (PasteMode::Offset | PasteMode::Library, None) => vec![match point_param(p, "center") {
            Some(c) => clip.bounds().map_or(Affine::IDENTITY, |b| Affine::translate(c - b.center())),
            None => Affine::translate((f64_or(p, "dx", off), f64_or(p, "dy", off))),
        }],
        (PasteMode::AllArtboards, None) => {
            let src = clip.source_artboard.or_else(|| st.doc.artboards.first().map(|a| a.rect)).map(|r| r.origin()).unwrap_or_default();
            st.doc.artboards.iter().map(|a| Affine::translate(a.rect.origin() - src)).collect()
        }
        // In place, in front, in back: onto the active artboard when the UI names it.
        _ => vec![onto_artboard(&st.doc, p, clip)],
    };
    let (ids, imported, artboard) = s.edit(mode.label(), |d, sel| {
        let imported = clip.import_into(d, choices, same_doc);
        let artboard = board.map(|(a, dv)| super::panelcmds::push_artboard_copy(d, a, a.rect + dv));
        let mut layers: BTreeMap<&str, NodeId> = BTreeMap::new();
        // Objects pasted into each parent so far (keeps their order in front and back pastes).
        let mut placed: BTreeMap<Option<NodeId>, usize> = BTreeMap::new();
        let mut new_ids = vec![];
        for xf in placements {
            for (k, n) in clip.nodes.iter().enumerate() {
                let mut c = d.reid(n);
                imported.apply(&mut c);
                if xf != Affine::IDENTITY {
                    c.transform(xf, false);
                }
                let layer = match clip.source_layers.get(k) {
                    Some(Some(name)) if remember => Some(*layers.entry(name).or_insert_with(|| layer_named(d, name))),
                    _ => None,
                };
                let (par, idx) = match (anchor, layer) {
                    // Next to the anchor, unless the object goes back to another layer.
                    (Some((par, i, _)), l) if l.is_none() || l == par => {
                        let k = placed.get(&par).copied().unwrap_or(0);
                        (par, if mode == PasteMode::Front { i + 1 + k } else { i + k })
                    }
                    (_, l) => {
                        let par = l.or(parent);
                        (par, if mode == PasteMode::Back { placed.get(&par).copied().unwrap_or(0) } else { usize::MAX })
                    }
                };
                *placed.entry(par).or_default() += 1;
                new_ids.push(d.insert(par, idx, c)?);
            }
        }
        sel.set(new_ids.iter().copied());
        // Pasted type arrives without the layout bounds cache: fill it so boxes and alignment are
        // right away (opening the file later would, but paste must not depend on a reopen).
        super::typecmd::refresh_bounds_of(d, &new_ids);
        Ok((new_ids, imported, artboard))
    })?;
    // The Artboard tool takes the pasted artboard.
    if let Some(i) = artboard.filter(|_| s.tool_id() == "artboard") {
        s.set_tool_option("active", &json!(i));
    }
    Ok(json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        "artboard": artboard,
        "added": imported.added,
        "merged": imported.merged,
        "renamed": imported.renamed,
    }))
}

/// The first unlocked layer or sublayer called `name` (topmost first), or a new top layer.
fn layer_named(d: &mut Document, name: &str) -> NodeId {
    let mut found = None;
    for l in d.layers.iter().rev() {
        l.walk(&mut |n| {
            if found.is_none() && n.is_layer() && !n.locked && n.name.as_deref() == Some(name) {
                found = Some(n.id);
            }
        });
    }
    found.unwrap_or_else(|| d.add_layer(Some(name)))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let dx = f64_or(p, "dx", 0.0);
    let dy = f64_or(p, "dy", 0.0);
    let ids = s.edit("Duplicate", |d, sel| duplicate_in(d, sel, &roots, Affine::translate((dx, dy))))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Duplicate `roots` directly above each original, transform the copies, select them.
pub(crate) fn duplicate_in(d: &mut Document, sel: &mut vectorcraft_doc::Selection, roots: &[NodeId], xf: Affine) -> Result<Vec<NodeId>> {
    let mut out = vec![];
    for id in roots {
        let Some((par, idx, _)) = d.position(*id) else { continue };
        let Some(n) = d.node(*id).cloned() else { continue };
        let mut c = d.reid(&n);
        c.transform(xf, true);
        out.push(d.insert(par, idx + 1, c)?);
    }
    sel.set(out.iter().copied());
    Ok(out)
}

#[allow(dead_code)]
pub(crate) fn arc_nodes(v: Vec<Node>) -> Vec<Arc<Node>> {
    v.into_iter().map(Arc::new).collect()
}
