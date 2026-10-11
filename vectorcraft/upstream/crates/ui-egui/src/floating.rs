//! Floating panels: panels dragged out of the dock float inside the window as groups of tabs.
//!
//! A dock tab, an icon of the icon column or a popped-out panel's title dragged out of the dock
//! floats that panel; the strip right of the dock's tabs floats the whole tabbed group. A floating
//! group moves by its title bar (or the strip right of its tabs, or the tab of a lone panel), and a
//! tab dragged out of a group floats on its own. Dropped on the dock, which lights up while the
//! pointer is over it, a group's panels go back to where they live in the dock (the tabbed group or
//! the icon column); dropped on another group's title bar or tabs, they stack with that group. The
//! × of a group puts its panels back in the dock too. The Tools panel floats the same way by its
//! title bar ([`crate::toolbar`]) and docks again on the window's left edge.
//!
//! `window.panel.float` / `window.panel.dock` do the same for agents. The groups and the Tools
//! panel's position are kept with the preferences and in user workspaces, and a position saved on
//! a bigger window is clamped into this one when drawn.

use egui::{CornerRadius, Id, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::state::{DockTab, FloatingPanels, UiState, all_panels};
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, dock, icons, panels};

/// Height of a floating group's title bar (its grip and ×).
pub const TITLE: f32 = 14.0;
/// Height of a floating group's tab strip.
pub const STRIP: f32 = 26.0;
/// Where `window.panel.float` floats a group given no position; each further group 24 points
/// lower right.
const FLOAT_AT: [f32; 2] = [420.0, 120.0];
/// Where `window.panel.float {panel: "tools"}` floats the Tools panel given no position.
const TOOLS_AT: [f32; 2] = [60.0, 120.0];
/// Points the pointer travels from its press before a drop counts (a click on a title bar that
/// overlaps the dock doesn't dock its group).
const TRAVEL: f32 = 4.0;

/// What a move carries.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Moving {
    /// The floating group holding this panel.
    Panel(&'static str),
    /// The floating Tools panel.
    Tools,
}

/// A move in progress: what moves, where the pointer holds it from its top-left corner, and where
/// the press began.
#[derive(Clone, Copy, Debug)]
struct Move {
    what: Moving,
    grab: Vec2,
    from: Pos2,
}

/// Where a moved group or Tools panel lands.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drop {
    /// Back in the dock (the Tools panel: at the window's left edge).
    Dock,
    /// Stacked with the floating group holding this panel.
    Stack(&'static str),
}

fn move_id() -> Id {
    Id::new("floating-move")
}

/// The dock (tabbed group and icon column) as last drawn: a group dropped there docks.
pub(crate) fn dock_rect_id() -> Id {
    Id::new("floating-dock-rect")
}

/// The strip along the window's left edge where the Tools panel docks, as last laid out.
/// The icon column's bounds (egui temp memory, one frame old), where panels without a tab in the
/// tabbed group go back to when docked.
pub(crate) fn icons_rect_id() -> Id {
    Id::new("floating-icons-rect")
}

pub(crate) fn tools_zone_id() -> Id {
    Id::new("floating-tools-zone")
}

/// The area of the floating group whose first panel is `first`.
pub fn area_id(first: &str) -> Id {
    Id::new(("floating-panels", first))
}

/// Panel `id` (`window.panel`) as a static string and its English label, if it is a panel.
fn panel(id: &str) -> Option<(&'static str, &'static str)> {
    all_panels().find(|(p, _)| *p == id)
}

/// The floating group holding panel `id`.
pub fn group_of(ui: &UiState, id: &str) -> Option<usize> {
    ui.floating_panels.iter().position(|g| g.panels.iter().any(|p| p == id))
}

/// The panels of group `g`, as static ids.
fn ids_of(g: &FloatingPanels) -> Vec<&'static str> {
    g.panels.iter().filter_map(|p| panel(p)).map(|(id, _)| id).collect()
}

/// The dock tabs left in the dock, in tab order.
pub fn docked_tabs(ui: &UiState) -> impl Iterator<Item = DockTab> + '_ {
    DockTab::ALL.into_iter().filter(|t| group_of(ui, t.info().0).is_none())
}

/// The dock tab the tabbed group shows: the chosen one, or the first left when it floats.
pub fn shown_tab(ui: &UiState) -> Option<DockTab> {
    docked_tabs(ui).find(|t| *t == ui.dock_tab).or_else(|| docked_tabs(ui).next())
}

/// Take `ids` out of the floating groups, dropping the groups left empty.
fn take(ui: &mut UiState, ids: &[&str]) {
    ui.floating_panels.retain_mut(|g| {
        let active = g.panels.get(g.active).filter(|p| !ids.contains(&p.as_str())).cloned();
        g.panels.retain(|p| !ids.contains(&p.as_str()));
        g.active = active.and_then(|a| g.panels.iter().position(|p| *p == a)).unwrap_or(0);
        !g.panels.is_empty()
    });
}

/// Float `ids` (out of the dock or of the group they were in) as a new group with its top-left
/// corner at `pos`, showing `active`.
pub fn float(ui: &mut UiState, ids: &[&str], active: &str, pos: Pos2) {
    take(ui, ids);
    if ui.open_panel.as_deref().is_some_and(|p| ids.contains(&p)) {
        ui.open_panel = None;
    }
    let panels: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
    let active = panels.iter().position(|p| p == active).unwrap_or(0);
    ui.floating_panels.push(FloatingPanels { panels, active, pos: [pos.x, pos.y], width: None });
    // The tabbed group shows another of its tabs when the one it showed floats.
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// Stack `ids` with the floating group holding `onto` (not one of them), showing `active`.
pub fn stack(ui: &mut UiState, ids: &[&str], active: &str, onto: &str) {
    take(ui, ids);
    if ui.open_panel.as_deref().is_some_and(|p| ids.contains(&p)) {
        ui.open_panel = None;
    }
    if let Some(g) = group_of(ui, onto).and_then(|i| ui.floating_panels.get_mut(i)) {
        g.panels.extend(ids.iter().map(|id| id.to_string()));
        g.active = g.panels.iter().position(|p| p == active).unwrap_or(g.active);
    }
    if let Some(tab) = shown_tab(ui) {
        ui.dock_tab = tab;
    }
}

/// Put `ids` back in the dock: the dock tabs in the tabbed group (showing `active`, or the first of
/// them), the other panels in the icon column.
pub fn dock(ui: &mut UiState, ids: &[&str], active: &str) {
    take(ui, ids);
    if let Some(tab) = DockTab::from_id(active).or_else(|| ids.iter().find_map(|id| DockTab::from_id(id))) {
        ui.dock_tab = tab;
    }
    ui.dock = true;
}

/// `pos`, a saved top-left corner, moved so that a `size` box there is inside `screen` (its
/// top-left corner first when it doesn't fit). Pulled in by whole points: egui rounds an area's
/// position, which would push a box of fractional size half a point past the edge.
pub fn clamp(pos: [f32; 2], size: Vec2, screen: Rect) -> Pos2 {
    let [x, y] = pos;
    let (x, y) = if x.is_finite() && y.is_finite() { (x, y) } else { (screen.left() + 60.0, screen.top() + 100.0) };
    let fit = |v: f32, max: f32| if v > max { max.floor() } else { v };
    pos2(fit(x, screen.right() - size.x).max(screen.left()), fit(y, screen.bottom() - size.y).max(screen.top()))
}

/// Start moving `what` with the pointer, held `grab` from its top-left corner.
fn start(ctx: &egui::Context, what: Moving, grab: Vec2) {
    let Some(from) = ctx.input(|i| i.pointer.press_origin().or(i.pointer.interact_pos())) else { return };
    ctx.data_mut(|d| d.insert_temp(move_id(), Move { what, grab, from }));
}

/// Where the pointer holds a group torn off by a press `left` points right of where the group's
/// tab strip will begin (under its title bar, past its 1-point border).
pub(crate) fn strip_grab(ctx: &egui::Context, left: f32) -> Vec2 {
    let x = ctx.input(|i| i.pointer.press_origin()).map_or(12.0, |p| p.x - left);
    vec2(1.0 + x.max(0.0), 1.0 + TITLE + STRIP / 2.0)
}

/// Float `ids` under the pointer, held `grab` from the new group's top-left corner, and carry on
/// moving them with the pointer (a tab or group dragged out of the dock or out of its group).
pub(crate) fn tear(app: &mut VectorcraftApp, ctx: &egui::Context, ids: &[&'static str], active: &'static str, grab: Vec2) {
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    float(&mut app.ui, ids, active, at - grab);
    start(ctx, Moving::Panel(active), grab);
}

/// The Tools panel's title bar was pressed (`title`, the panel's bounds `bounds`): docked, dragging
/// it out of the panel floats the panel under the pointer; floating at `floating`, a drag moves it.
pub(crate) fn drag_tools(app: &mut VectorcraftApp, ctx: &egui::Context, title: &egui::Response, bounds: Rect, floating: Option<Pos2>) {
    let Some(at) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    match floating {
        Some(pos) if title.drag_started() => start(ctx, Moving::Tools, at - pos),
        None if title.dragged() && !bounds.contains(at) => {
            // Held where the press grabbed the title bar.
            let grab = ctx.input(|i| i.pointer.press_origin()).map_or(vec2(10.0, 7.0), |p| p - bounds.min);
            let p = at - grab;
            app.ui.toolbar_pos = Some([p.x, p.y]);
            start(ctx, Moving::Tools, grab);
        }
        _ => {}
    }
}

/// Where the pointer at `at` would drop `what`, and the zone that lights up for it.
fn drop_target(app: &VectorcraftApp, ctx: &egui::Context, what: Moving, at: Pos2) -> Option<(Drop, Rect)> {
    let zone = |id: Id| ctx.data(|d| d.get_temp::<Rect>(id)).filter(|r| r.contains(at));
    let Moving::Panel(id) = what else { return zone(tools_zone_id()).map(|r| (Drop::Dock, r)) };
    let own = group_of(&app.ui, id);
    // Another group's title bar and tabs stack with it.
    for (i, g) in app.ui.floating_panels.iter().enumerate() {
        let Some((first, _)) = g.panels.first().and_then(|p| panel(p)).filter(|_| Some(i) != own) else { continue };
        let head = ctx.memory(|m| m.area_rect(area_id(first))).map(|r| Rect::from_min_size(r.min, vec2(r.width(), 1.0 + TITLE + STRIP)));
        if let Some(head) = head.filter(|h| h.contains(at)) {
            return Some((Drop::Stack(first), head));
        }
    }
    let dock = zone(dock_rect_id()).filter(|_| app.ui.dock && app.ui.screen_mode < 3)?;
    // Panels that aren't tabs of the tabbed group (or all of them, with it collapsed) go back to
    // their icons: light up the icon column, where they land, rather than the tabs.
    let tabbed = !app.ui.dock_collapsed
        && own.and_then(|i| app.ui.floating_panels.get(i)).is_some_and(|g| ids_of(g).into_iter().any(|p| DockTab::from_id(p).is_some()));
    let lit = if tabbed { dock } else { ctx.data(|d| d.get_temp::<Rect>(icons_rect_id())).unwrap_or(dock) };
    Some((Drop::Dock, lit))
}

/// Light up a drop zone: over the docked panels, under the floating ones (the moved one too).
fn highlight(ctx: &egui::Context, zone: Rect) {
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, Id::new("floating-drop-zone")));
    painter.rect(zone.shrink(1.0), CornerRadius::same(2), t.accent.gamma_multiply(0.2), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
}

/// Carry a move on (at the start of a frame, before the panels are drawn): the group or Tools panel
/// follows the pointer and the zone it would drop on lights up; the release drops it there.
pub fn track(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(m) = ctx.data(|d| d.get_temp::<Move>(move_id())) else { return };
    let at = ctx.input(|i| i.pointer.interact_pos());
    let target = at.filter(|at| at.distance(m.from) > TRAVEL).and_then(|at| drop_target(app, ctx, m.what, at));
    if ctx.input(|i| i.pointer.primary_down()) {
        if let Some(p) = at.map(|at| at - m.grab)
            && p.x.is_finite()
            && p.y.is_finite()
        {
            match m.what {
                Moving::Panel(id) => {
                    if let Some(g) = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get_mut(i)) {
                        g.pos = [p.x, p.y];
                    }
                }
                Moving::Tools if app.ui.toolbar_pos.is_some() => app.ui.toolbar_pos = Some([p.x, p.y]),
                Moving::Tools => {}
            }
        }
        if let Some((_, zone)) = target {
            highlight(ctx, zone);
        }
        return;
    }
    ctx.data_mut(|d| d.remove::<Move>(move_id()));
    let Some((drop, _)) = target else { return };
    let Moving::Panel(id) = m.what else {
        app.ui.toolbar_pos = None;
        return;
    };
    let Some(g) = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get(i)) else { return };
    let ids = ids_of(g);
    let active = ids.get(g.active).copied().unwrap_or(id);
    match drop {
        Drop::Dock => {
            dock(&mut app.ui, &ids, active);
            // One that went back to its icon pops out of it, so it doesn't seem to vanish.
            if app.ui.dock_collapsed || DockTab::from_id(active).is_none() {
                app.ui.open_panel = Some(active.to_string());
            }
        }
        Drop::Stack(onto) => stack(&mut app.ui, &ids, active, onto),
    }
}

/// What a press on a floating group did.
enum Action {
    /// Show the group's `k`th tab.
    Activate(usize, usize),
    /// Its × puts the group's panels back in the dock.
    Close(usize),
    /// Move the group holding this panel, drawn at this corner.
    Move(&'static str, Pos2),
    /// Float this panel out of its group; its tab began this far right.
    Tear(&'static str, f32),
}

/// The floating groups, each a stack of tabs under a title bar. Hidden with the dock (Tab,
/// Presentation Mode).
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.floating_panels.is_empty() || !app.ui.dock || app.ui.screen_mode >= 3 {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let mut action = None;
    for gi in 0..app.ui.floating_panels.len() {
        let Some(g) = app.ui.floating_panels.get(gi) else { break };
        let ids = ids_of(g);
        let Some(&first) = ids.first() else { continue };
        let active = ids.get(g.active).copied().unwrap_or(first);
        let given = g.width.map(|w| w.min(screen.width() - 16.0));
        let area = area_id(first);
        let size = ctx.memory(|m| m.area_rect(area)).map_or(vec2(dock::panel_width(active), 200.0), |r| r.size());
        let pos = clamp(g.pos, size, screen);
        // The tabbed group's panels fill (Layers) or scroll (Properties) to near the window's bottom,
        // leaving room for their margins (a group that only just fitted would creep up a little each
        // frame).
        let tall = (screen.bottom() - pos.y - TITLE - STRIP - 60.0).clamp(120.0, 560.0);
        let lone = ids.len() == 1;
        egui::Area::new(area).order(egui::Order::Middle).fixed_pos(pos).show(ctx, |ui| {
            let frame = egui::Frame::popup(ui.style()).fill(t.panel).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::ZERO);
            frame.show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let tabs: Vec<_> = ids
                    .iter()
                    .map(|id| {
                        let label = panel(id).map_or(*id, |(_, label)| label);
                        let color = if *id == active { t.text_strong } else { t.text_dim };
                        ui.painter().layout_no_wrap(tl!(label).to_string(), theme::semibold(12.0), color)
                    })
                    .collect();
                let tabs_width: f32 = tabs.iter().map(|g| g.size().x + 24.0).sum();
                let width = ids.iter().map(|id| dock::panel_width(id)).fold(tabs_width + 32.0, f32::max).max(given.unwrap_or(0.0));
                ui.set_width(width);
                // Title bar: a grip that moves the group, and the × that docks it.
                let (bar, _) = ui.allocate_exact_size(vec2(width, TITLE), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, t.tab_strip);
                let close = Rect::from_center_size(bar.right_center() - vec2(10.0, 0.0), vec2(TITLE, TITLE));
                let grip = ui.interact(Rect::from_min_max(bar.min, pos2(close.left(), bar.bottom())), ui.id().with("title"), Sense::drag());
                for k in 0..2 {
                    let y = bar.center().y - 1.5 + k as f32 * 3.0;
                    ui.painter().line_segment([pos2(bar.center().x - 9.0, y), pos2(bar.center().x + 9.0, y)], Stroke::new(0.6, t.text_disabled));
                }
                if grip.drag_started() {
                    action = Some(Action::Move(active, pos));
                }
                grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to move"));
                let x = ui.interact(close, ui.id().with("close"), Sense::click());
                icons::paint(ui, "x", close.shrink(3.0), if x.hovered() { t.text_strong } else { t.text_dim });
                if x.on_hover_text(tl!("Put back in the dock")).clicked() {
                    action = Some(Action::Close(gi));
                }
                // Tab strip: a click shows a tab, a drag out of the strip floats it on its own.
                let (strip, _) = ui.allocate_exact_size(vec2(width, STRIP), Sense::hover());
                ui.painter().rect_filled(strip, 0.0, t.panel_darker);
                let mut left = strip.left();
                for (k, (&id, galley)) in ids.iter().zip(tabs).enumerate() {
                    let r = Rect::from_min_size(pos2(left, strip.top()), vec2(galley.size().x + 24.0, STRIP));
                    left = r.right();
                    let resp = ui.interact(r, ui.id().with(("tab", id)), Sense::click_and_drag());
                    if id == active {
                        ui.painter().rect_filled(r, 0.0, t.panel);
                    }
                    ui.painter().galley(pos2(r.left() + 12.0, r.center().y - galley.size().y / 2.0), galley, t.text);
                    if resp.clicked() {
                        action = Some(Action::Activate(gi, k));
                    } else if lone && resp.drag_started() {
                        action = Some(Action::Move(id, pos));
                    } else if !lone && resp.dragged() && pointer.is_some_and(|p| !strip.expand(4.0).contains(p)) {
                        action = Some(Action::Tear(id, r.left()));
                    }
                }
                let menu = Rect::from_center_size(strip.right_center() - vec2(14.0, 0.0), vec2(16.0, 16.0));
                let rest = Rect::from_min_max(pos2(left, strip.top()), pos2(menu.left() - 4.0, strip.bottom()));
                if rest.width() > 0.0 {
                    let resp = ui.interact(rest, ui.id().with("strip"), Sense::drag());
                    if resp.drag_started() {
                        action = Some(Action::Move(active, pos));
                    }
                    resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tl!("Drag to move"));
                }
                panels::panel_menu(app, ui, active, menu);
                dock::panel_body(app, ui, active, width, tall);
            });
        });
    }
    match action {
        Some(Action::Activate(gi, k)) => {
            if let Some(g) = app.ui.floating_panels.get_mut(gi) {
                g.active = k;
            }
        }
        Some(Action::Close(gi)) => {
            if let Some(g) = app.ui.floating_panels.get(gi) {
                let ids = ids_of(g);
                let active = ids.get(g.active).copied().unwrap_or_default();
                dock(&mut app.ui, &ids, active);
            }
        }
        Some(Action::Move(id, pos)) => {
            // From where it is drawn (a position saved on a bigger window was clamped into this one).
            if let Some(g) = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get_mut(i)) {
                g.pos = [pos.x, pos.y];
            }
            if let Some(at) = pointer {
                start(ctx, Moving::Panel(id), at - pos);
            }
        }
        Some(Action::Tear(id, left)) => tear(app, ctx, &[id], id, strip_grab(ctx, left)),
        None => {}
    }
}

/// `window.panel.float` and `window.panel.dock`; `None` for other commands.
pub fn command(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let float = match id {
        "window.panel.float" => true,
        "window.panel.dock" => false,
        _ => return None,
    };
    Some(float_or_dock(app, float, p))
}

/// The `x`, `y` params: a top-left corner in screen points, if given.
fn position(p: &Value) -> Result<Option<Pos2>, String> {
    let coord = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Some(None),
        Some(v) => v.as_f64().filter(|f| f.abs() < 1e6).map(|f| Some(f as f32)),
    };
    match (coord("x"), coord("y")) {
        (Some(Some(x)), Some(Some(y))) => Ok(Some(pos2(x, y))),
        (Some(None), Some(None)) => Ok(None),
        _ => Err("x and y must both be numbers".into()),
    }
}

fn float_or_dock(app: &mut VectorcraftApp, float_it: bool, p: &Value) -> Result<Value, String> {
    let raw = p.get("panel").and_then(Value::as_str).unwrap_or_default().trim();
    let group = crate::menus::opt_bool(p, "group")?.unwrap_or(false);
    let at = position(p)?;
    if raw.eq_ignore_ascii_case("tools") || raw.eq_ignore_ascii_case("toolbar") {
        app.ui.toolbar_pos = float_it.then(|| at.map_or(TOOLS_AT, |p| [p.x, p.y]));
        app.ui.toolbar |= float_it;
        return Ok(json!({ "panel": "tools", "floating": float_it, "pos": app.ui.toolbar_pos }));
    }
    let Some(id) = crate::menus::normalize_panel(raw) else { return Err(format!("unknown panel `{raw}`")) };
    let own = group_of(&app.ui, id);
    // The panel, or with `group` its whole group: its floating group, or the tabs left in the dock.
    let ids: Vec<&'static str> = match (group, own.and_then(|i| app.ui.floating_panels.get(i))) {
        (true, Some(g)) => ids_of(g),
        (true, None) if DockTab::from_id(id).is_some() => docked_tabs(&app.ui).map(|t| t.info().0).collect(),
        _ => vec![id],
    };
    app.ui.dock = true;
    if !float_it {
        dock(&mut app.ui, &ids, id);
        return Ok(json!({ "panel": id, "floating": false }));
    }
    match p.get("onto").and_then(Value::as_str) {
        Some(onto) => {
            let onto = crate::menus::normalize_panel(onto)
                .filter(|o| !ids.contains(o) && group_of(&app.ui, o).is_some_and(|g| Some(g) != own))
                .ok_or("onto must name a panel floating in another group")?;
            stack(&mut app.ui, &ids, id, onto);
        }
        // Its own group already: moved there (or left where it is), showing the panel.
        None => match own.and_then(|i| app.ui.floating_panels.get_mut(i)).filter(|g| g.panels.len() == ids.len()) {
            Some(g) => {
                g.active = g.panels.iter().position(|p| p == id).unwrap_or(g.active);
                if let Some(at) = at {
                    g.pos = [at.x, at.y];
                }
            }
            None => {
                let n = app.ui.floating_panels.len() as f32;
                let at = at.unwrap_or(pos2(FLOAT_AT[0] + 24.0 * n, FLOAT_AT[1] + 24.0 * n));
                float(&mut app.ui, &ids, id, at);
            }
        },
    }
    let g = group_of(&app.ui, id).and_then(|i| app.ui.floating_panels.get(i));
    Ok(json!({ "panel": id, "floating": true, "group": g.map(|g| &g.panels), "pos": g.map(|g| g.pos) }))
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton};
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    const SCREEN: Vec2 = vec2(1400.0, 900.0);

    /// Frames of the whole window, with pointer input one event a frame (as the control
    /// channel's `ui.drag` sends it).
    struct Harness {
        app: VectorcraftApp,
        ctx: egui::Context,
        time: f64,
        screen: Vec2,
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    impl Harness {
        fn new() -> Self {
            Self::on(SCREEN)
        }

        /// The same window at another size. The icon column scrolls when it is too short for
        /// every icon, which hides the ones past the bottom — a test that counts them needs
        /// a window tall enough to show them all.
        fn on(screen: Vec2) -> Self {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            let mut h = Self { app, ctx: egui::Context::default(), time: 0.0, screen };
            h.settle();
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let screen = Rect::from_min_size(Pos2::ZERO, self.screen);
            let raw = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), events, ..Default::default() };
            let app = &mut self.app;
            self.ctx
                .run_ui(raw, |ui| {
                    app.logic(ui.ctx());
                    app.ui(ui);
                })
                .textures_delta
                .clear();
        }

        fn settle(&mut self) {
            for _ in 0..4 {
                self.frame(vec![]);
            }
        }

        /// Press at `from` and move to `to` in steps, the button still down.
        fn hold(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![button(from, true)]);
            for k in 1..=8 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 8.0))]);
            }
        }

        fn release(&mut self, at: Pos2) {
            self.frame(vec![button(at, false)]);
            self.settle();
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.hold(from, to);
            self.release(to);
        }

        fn click(&mut self, at: Pos2) {
            self.frame(vec![Event::PointerMoved(at), button(at, true)]);
            self.release(at);
        }

        fn temp_rect(&self, id: Id) -> Rect {
            self.ctx.data(|d| d.get_temp::<Rect>(id)).unwrap()
        }

        /// The floating group whose first panel is `first`, as drawn.
        fn group(&self, first: &str) -> Rect {
            self.ctx.memory(|m| m.area_rect(area_id(first))).unwrap()
        }

        /// A point on a floating group's title bar (left of its ×).
        fn title(&self, first: &str) -> Pos2 {
            let r = self.group(first);
            pos2(r.left() + 30.0, r.top() + 1.0 + TITLE / 2.0)
        }

        /// The rects of the widgets that sense a drag and pass `keep`, top to bottom, left to right.
        fn widgets(&self, keep: impl Fn(Rect) -> bool) -> Vec<Rect> {
            let mut r: Vec<Rect> = self.ctx.viewport(|vp| {
                vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.sense.senses_drag() && keep(w.rect)).map(|w| w.rect).collect()
            });
            r.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
            r
        }

        /// The dock's tabs, left to right.
        fn dock_tabs(&self) -> Vec<Rect> {
            let dock = self.temp_rect(dock_rect_id());
            self.widgets(|r| r.height() == 32.0 && dock.contains_rect(r))
        }

        /// The icon column's panel icons, top to bottom.
        fn icons(&self) -> Vec<Rect> {
            let dock = self.temp_rect(dock_rect_id());
            self.widgets(|r| r.size() == vec2(30.0, 30.0) && dock.contains_rect(r))
        }

        fn groups(&self) -> Vec<Vec<&str>> {
            self.app.ui.floating_panels.iter().map(|g| g.panels.iter().map(String::as_str).collect()).collect()
        }
    }

    #[test]
    fn a_dock_tab_dragged_out_floats_and_dropped_on_the_dock_docks_again() {
        let mut h = Harness::new();
        let tabs = h.dock_tabs();
        assert_eq!(tabs.len(), 3, "Properties, Layers, Libraries: {tabs:?}");
        let to = pos2(600.0, 400.0);
        h.drag(tabs[1].center(), to);
        assert_eq!(h.groups(), [["layers"]], "Layers floats");
        assert!(h.group("layers").contains(to), "under the pointer: {:?}", h.group("layers"));
        assert_eq!(h.dock_tabs().len(), 2, "its tab left the dock");
        assert_eq!(h.app.ui.dock_tab, DockTab::Properties);
        // A click on its title bar moves nothing, even over the dock.
        let dock = h.temp_rect(dock_rect_id());
        h.app.run("window.panel.float", json!({"panel": "layers", "x": dock.left() + 10.0, "y": 300})).unwrap();
        h.settle();
        h.click(h.title("layers"));
        assert_eq!(h.groups(), [["layers"]], "a click doesn't dock it");
        // Dragged by its title bar onto the dock, the dock lights up, and the release docks it.
        h.app.run("window.panel.float", json!({"panel": "layers", "x": 500, "y": 300})).unwrap();
        h.settle();
        let over = dock.center();
        h.hold(h.title("layers"), over);
        assert!(h.group("layers").contains(over), "the group follows the pointer");
        let target = drop_target(&h.app, &h.ctx, Moving::Panel("layers"), over);
        assert_eq!(target, Some((Drop::Dock, dock)), "the dock is the drop zone, lit whole: its tab comes back");
        h.release(over);
        assert!(h.groups().is_empty(), "docked: {:?}", h.groups());
        assert_eq!((h.dock_tabs().len(), h.app.ui.dock_tab), (3, DockTab::Layers), "back in the dock, shown");
    }

    #[test]
    fn the_strip_right_of_the_tabs_floats_the_whole_group_and_its_close_box_docks_it() {
        let mut h = Harness::new();
        let tabs = h.dock_tabs();
        let rest = tabs[2].right_center() + vec2(30.0, 0.0);
        h.drag(rest, pos2(500.0, 300.0));
        assert_eq!(h.groups(), [["properties", "layers", "libraries"]]);
        assert!(h.dock_tabs().is_empty(), "the tabbed group left the dock");
        let column = h.temp_rect(dock_rect_id());
        assert!(column.width() < 40.0, "only the icon column is left: {column:?}");
        // Its × puts them back.
        let g = h.group("properties");
        h.click(pos2(g.right() - 11.0, g.top() + 1.0 + TITLE / 2.0));
        assert!(h.groups().is_empty());
        assert_eq!(h.dock_tabs().len(), 3);
    }

    #[test]
    fn icons_dragged_out_float_stack_onto_another_group_and_tear_out_of_it() {
        // Tall enough that the icon column shows every icon, so counting them says what it
        // means: one leaves the column, rather than one more scrolling into view.
        let mut h = Harness::on(vec2(1400.0, 1600.0));
        // Color, Color Guide, Swatches… top to bottom.
        let icons = h.icons();
        let count = icons.len();
        assert_eq!(count, crate::state::ICON_PANEL_GROUPS.iter().map(|g| g.len()).sum::<usize>(), "every group's icons are drawn");
        h.drag(icons[2].center(), pos2(500.0, 250.0));
        assert_eq!(h.groups(), [["swatches"]]);
        assert_eq!(h.icons().len(), count - 1, "its icon left the column");
        h.drag(h.icons()[0].center(), pos2(500.0, 650.0));
        assert_eq!(h.groups(), [["swatches"], ["color"]]);
        // Color's title bar dropped on the Swatches group's tabs stacks them.
        let onto = h.group("swatches").left_top() + vec2(150.0, 1.0 + TITLE + STRIP / 2.0);
        h.hold(h.title("color"), onto);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("color"), onto).map(|t| t.0), Some(Drop::Stack("swatches")));
        h.release(onto);
        assert_eq!(h.groups(), [["swatches", "color"]]);
        assert_eq!(h.app.ui.floating_panels[0].active, 1, "showing the panel dropped on it");
        // A click shows a tab; a tab dragged out of the strip floats on its own again.
        let g = h.group("swatches");
        let tabs = h.widgets(|r| r.height() == STRIP && g.contains_rect(r));
        assert_eq!(tabs.len(), 3, "two tabs and the strip right of them: {tabs:?}");
        h.click(tabs[0].center());
        assert_eq!(h.app.ui.floating_panels[0].active, 0);
        let to = pos2(900.0, 500.0);
        h.drag(tabs[1].center(), to);
        assert_eq!(h.groups(), [["swatches"], ["color"]]);
        assert!(h.group("color").contains(to));
        // Window › Color shows it where it floats; Dock Panel puts it back.
        assert_eq!(h.app.run("window.panel", json!({"panel": "Color"})).unwrap(), json!({"floating": ["color"]}));
        assert_eq!(crate::menus::checked(&h.app, "window.panel", &json!({"panel": "color"})), Some(true));
        h.app.run("window.panel.dock", json!({"panel": "color"})).unwrap();
        h.settle();
        assert_eq!(h.icons().len(), count - 1, "Color is back in the column");
    }

    #[test]
    fn a_panel_dropped_on_the_dock_lights_up_and_pops_out_of_its_icon() {
        let mut h = Harness::new();
        let count = h.icons().len();
        h.drag(h.icons()[0].center(), pos2(500.0, 400.0));
        assert_eq!(h.groups(), [["color"]]);
        // Over the tabbed group: Color has no tab there, so the icon column lights up, not the tabs.
        let (dock, column) = (h.temp_rect(dock_rect_id()), h.temp_rect(icons_rect_id()));
        let over = h.dock_tabs()[1].center();
        h.hold(h.title("color"), over);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Panel("color"), over), Some((Drop::Dock, column)));
        assert!(column.width() < dock.width(), "the column, not the whole dock: {column:?} {dock:?}");
        // Dropped, it's back in the column and pops out of its icon rather than vanishing.
        h.release(over);
        assert!(h.groups().is_empty());
        assert_eq!((h.icons().len(), h.app.ui.open_panel.as_deref()), (count, Some("color")));
    }

    #[test]
    fn a_popped_out_panel_dragged_by_its_title_floats() {
        let mut h = Harness::new();
        h.app.run("window.panel", json!({"panel": "swatches"})).unwrap();
        h.settle();
        let flyout = h.ctx.memory(|m| m.area_rect(Id::new("icon-panel"))).unwrap();
        let to = pos2(500.0, 400.0);
        h.drag(flyout.left_top() + vec2(20.0, 13.0), to);
        assert_eq!((h.groups(), h.app.ui.open_panel.as_deref()), (vec![vec!["swatches"]], None));
        assert!(h.group("swatches").contains(to), "under the pointer: {:?}", h.group("swatches"));
        // Its panel menu docks it again.
        h.app.run("window.panel.dock", json!({"panel": "swatches"})).unwrap();
        assert!(h.groups().is_empty());
    }

    #[test]
    fn the_tools_panel_floats_by_its_title_bar_and_docks_on_the_left_edge() {
        let mut h = Harness::new();
        let zone = h.temp_rect(tools_zone_id());
        let title = zone.left_top() + vec2(30.0, 7.0);
        let to = pos2(400.0, 300.0);
        h.drag(title, to);
        let pos = h.app.ui.toolbar_pos.expect("the Tools panel floats");
        let float = h.ctx.memory(|m| m.area_rect(Id::new("toolbar-floating"))).unwrap();
        assert!(float.contains(to) && (pos2(pos[0], pos[1]) - float.min).length() < 1.0, "under the pointer: {float:?}");
        // Back on the window's left edge.
        let edge = pos2(10.0, 400.0);
        h.hold(float.left_top() + vec2(30.0, 7.0), edge);
        assert_eq!(drop_target(&h.app, &h.ctx, Moving::Tools, edge).map(|t| t.0), Some(Drop::Dock));
        h.release(edge);
        assert_eq!(h.app.ui.toolbar_pos, None, "docked");
        // A click on the title bar still switches the columns.
        let double = h.app.ui.toolbar_double;
        h.click(title);
        assert_ne!(h.app.ui.toolbar_double, double);
    }

    #[test]
    fn floating_panels_are_kept_in_workspaces_and_preferences_and_clamped_into_the_window() {
        let mut h = Harness::new();
        let app = &mut h.app;
        // Saved on a much bigger screen.
        app.run("window.panel.float", json!({"panel": "layers", "x": 5000, "y": 4000})).unwrap();
        let r = app.run("window.panel.float", json!({"panel": "stroke", "onto": "Layers"})).unwrap();
        assert_eq!(r["group"], json!(["layers", "stroke"]));
        app.run("window.panel.float", json!({"panel": "tools", "x": 300, "y": 200})).unwrap();
        app.run("window.workspace.new", json!({"name": "Floating"})).unwrap();
        let saved = app.ui.floating_panels.clone();
        app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
        assert!(app.ui.floating_panels.is_empty() && app.ui.toolbar_pos.is_none(), "Essentials docks everything");
        app.run("window.workspace", json!({"name": "Floating"})).unwrap();
        assert_eq!((&app.ui.floating_panels, app.ui.toolbar_pos), (&saved, Some([300.0, 200.0])));
        let prefs: UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let prefs = prefs.sanitized();
        assert_eq!((&prefs.floating_panels, prefs.toolbar_pos), (&saved, Some([300.0, 200.0])));
        // Unknown and repeated panels (a hand-edited file) are dropped.
        let edited: UiState = serde_json::from_value(json!({
            "floating_panels": [{"panels": ["nope", "layers"], "active": 7, "pos": [1, 2]}, {"panels": ["layers"], "pos": [3, 4]}],
        }))
        .unwrap();
        let edited = edited.sanitized();
        assert_eq!(edited.floating_panels, [FloatingPanels { panels: vec!["layers".into()], active: 0, pos: [1.0, 2.0], width: None }]);
        h.settle();
        let g = h.group("layers");
        let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
        assert!(screen.contains_rect(g), "clamped into the window: {g:?}");
        assert!(h.dock_tabs().len() == 2 && h.app.ui.dock_tab == DockTab::Properties);
    }

    #[test]
    fn float_and_dock_commands_check_their_params() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(app.run("window.panel.float", json!({"panel": "nope"})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "x": "left", "y": 2})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "x": 2})).is_err(), "both x and y");
        assert!(app.run("window.panel.float", json!({"panel": "layers", "group": "yes"})).is_err());
        assert!(app.run("window.panel.float", json!({"panel": "layers", "onto": "color"})).is_err(), "Color isn't floating");
        assert!(app.ui.floating_panels.is_empty());
        // The tabbed group's tabs float together.
        let r = app.run("window.panel.float", json!({"panel": "layers", "group": true})).unwrap();
        assert_eq!(r["group"], json!(["properties", "layers", "libraries"]));
        assert_eq!(app.ui.floating_panels[0].active, 1);
        assert!(app.run("window.panel.float", json!({"panel": "layers", "onto": "properties"})).is_err(), "not onto its own group");
        // Floated out of its group; a lone panel moves.
        app.run("window.panel.float", json!({"panel": "layers", "x": 10, "y": 20})).unwrap();
        assert_eq!((app.ui.floating_panels.len(), floating_pos(&app, "layers")), (2, [10.0, 20.0]));
        app.run("window.panel.float", json!({"panel": "layers", "x": 30, "y": 40})).unwrap();
        assert_eq!((app.ui.floating_panels.len(), floating_pos(&app, "layers")), (2, [30.0, 40.0]));
        app.run("window.panel.dock", json!({"panel": "properties", "group": true})).unwrap();
        assert_eq!(app.ui.floating_panels.len(), 1);
        assert_eq!(app.ui.dock_tab, DockTab::Properties);
        assert_eq!(app.run("window.panel.dock", json!({"panel": "tools"})).unwrap()["floating"], json!(false));
        app.ui.toolbar = false;
        app.run("window.panel.float", json!({"panel": "Toolbar"})).unwrap();
        assert!(app.ui.toolbar && app.ui.toolbar_pos == Some(TOOLS_AT), "floating shows the Tools panel");
    }

    fn floating_pos(app: &VectorcraftApp, id: &str) -> [f32; 2] {
        group_of(&app.ui, id).map(|i| app.ui.floating_panels[i].pos).unwrap()
    }
}
