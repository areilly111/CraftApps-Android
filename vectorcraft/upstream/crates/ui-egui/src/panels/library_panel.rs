//! The library panel: a floating, read-only view of one library (swatch and graphic style
//! libraries; any kind of library implementing [`LibraryKind`]) with thumbnail and list views, a
//! find field, the libraries menu and previous / next arrows. A plain click on an item adds it to
//! the document and applies it; Shift and Cmd/Ctrl-clicks select items (and folders) for Add to ….
//!
//! Which library is open lives in `UiState::library_panel` (`window.swatchLibrary` and
//! `window.graphicStyleLibrary` open one), so agents can open and read it; the view, find text and
//! selection are panel state.

use egui::{CornerRadius, Rect, Sense, Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;

use super::swatches::{View, click_selection, folder_tile, list_row, list_row_name, tile_grid};
use super::{pstate, set_pstate};
use crate::theme::{self, Tokens};
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

/// The library open in the library panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenLibrary {
    /// The kind of library ([`LibraryKind::KIND`], e.g. "swatches").
    pub kind: String,
    /// The library's id.
    pub id: String,
}

/// A library as the libraries menu lists it.
pub(crate) struct LibraryRef {
    pub id: String,
    pub name: String,
    /// The submenu it is listed under (`None`: the top level).
    pub submenu: Option<&'static str>,
}

/// A row of the panel: an item, or the folder of a group of items (`item` None).
pub(crate) struct Row<'a, I> {
    pub name: &'a str,
    pub item: Option<&'a I>,
}

/// One kind of library the panel shows.
pub(crate) trait LibraryKind {
    /// `OpenLibrary::kind`, and the prefix of the panel's state keys.
    const KIND: &'static str;
    /// The UI command that opens a library in the panel (`{library: id}`).
    const OPEN: &'static str;
    /// The libraries menu's tooltip ("Swatch Libraries Menu").
    const MENU: &'static str;
    /// What adding the selection is called ("Add to Swatches").
    const ADD: &'static str;
    /// The view the panel shows until another is picked.
    const VIEW: View = View::MediumThumb;
    /// A library's contents, cheap to clone.
    type Lib;
    type Item;
    /// Every library, in menu order.
    fn list(app: &VectorcraftApp) -> Vec<LibraryRef>;
    /// Library `id`: its name and contents.
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)>;
    /// The rows matching the find field's `query` (lower case; empty: all).
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, Self::Item>>;
    /// An item's thumbnail in `r`.
    fn draw(app: &VectorcraftApp, ui: &Ui, r: Rect, item: &Self::Item, selected: bool, hovered: bool);
    /// The icons at the right of list row `r`.
    fn row_icons(ui: &Ui, r: Rect, item: &Self::Item);
    /// An item described in words, for list tooltips.
    fn describe(item: &Self::Item) -> String;
    /// A plain click on item `name` of library `id`: add it to the document and apply it.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str);
    /// Add the items and folders `names` of library `id` to the document.
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>);
    /// Items heading the libraries menu.
    fn menu_head(_app: &mut VectorcraftApp, _ui: &mut Ui) {}
    /// Items closing the libraries menu.
    fn menu_tail(_app: &mut VectorcraftApp, _ui: &mut Ui) {}
}

/// The state key `key` of `K`'s panel.
fn key<K: LibraryKind>(key: &str) -> String {
    format!("{}-library-{key}", K::KIND)
}

/// The view of `K`'s panel.
fn view<K: LibraryKind>(ctx: &egui::Context) -> View {
    pstate::<Option<View>>(ctx, &key::<K>("view")).unwrap_or(K::VIEW)
}

/// The interaction id of the tile or row of `name`.
pub(crate) fn tile_id<K: LibraryKind>(name: &str) -> egui::Id {
    egui::Id::new(("library-tile", K::KIND, name))
}

/// Draw the open library panel, if any.
pub fn show_window(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(open) = app.ui.library_panel.clone() else { return };
    match open.kind.as_str() {
        super::swatches::SwatchLibraries::KIND => window::<super::swatches::SwatchLibraries>(app, ctx, &open.id),
        super::graphic_styles::GraphicStyleLibraries::KIND => window::<super::graphic_styles::GraphicStyleLibraries>(app, ctx, &open.id),
        _ => app.ui.library_panel = None,
    }
}

/// The submenu of the libraries menu that lists libraries of `category` (`None`: the top level).
pub(crate) fn submenu(category: &str) -> Option<&'static str> {
    match category {
        "gradients" => Some("Gradients"),
        "user" => Some("User Defined"),
        "loaded" => Some("Other Libraries"),
        _ => None,
    }
}

/// Is library `id` built in? A library the user saved to the User Defined folder or loaded from a
/// file (`user/…`, `loaded/…` ids) is named by its file.
fn builtin_library(id: &str) -> bool {
    !(id.starts_with("user/") || id.starts_with("loaded/"))
}

/// Library `id`'s name as shown: a built-in library's in the UI language, any other's as it is.
pub(crate) fn library_name<'a>(id: &str, name: &'a str) -> &'a str {
    super::label_or_name(name, builtin_library(id))
}

/// The opener of library `K`'s libraries (`window.swatchLibrary {library}`): open library `library`
/// (an id or a name; `list`: the command listing them) in the panel, `null` closing it. `found`
/// looks a library up: its id, name and item count. → {open, name, count}
pub(crate) fn open_command<K: LibraryKind>(
    app: &mut VectorcraftApp,
    p: &Value,
    list: &str,
    found: impl FnOnce(&VectorcraftApp, &str) -> Option<(String, String, usize)>,
) -> Result<Value, String> {
    let Some(key) = p.get("library").filter(|v| !v.is_null()) else {
        app.ui.library_panel = None;
        return Ok(json!({ "open": null }));
    };
    let key = key.as_str().ok_or_else(|| format!("`library` is a library id or name (see {list})"))?;
    let (id, name, count) = found(app, key).ok_or_else(|| format!("no library `{key}` (see {list})"))?;
    app.ui.library_panel = Some(OpenLibrary { kind: K::KIND.into(), id: id.clone() });
    Ok(json!({"open": id, "name": name, "count": count}))
}

/// Other Library…: the library file at `path`, else one picked in an open dialog; `None` when the
/// pick is asynchronous (on the web the picked file arrives later through [`crate::io::open_bytes`]).
pub(crate) fn pick_library_file(app: &mut VectorcraftApp, path: Option<String>) -> Result<Option<String>, String> {
    if path.is_none()
        && let Some(f) = app.services.open_async.as_mut()
    {
        f();
        return Ok(None);
    }
    path.or_else(|| {
        let pick = crate::FilePick { filters: vectorcraft_engine::cmd::fileio::open_filters().collect(), ..Default::default() };
        crate::picks::open(app, &pick)
    })
    .map(Some)
    .ok_or_else(|| "cancelled".into())
}

/// The User Defined library a Window menu slot (`prefix` and a number from 1) stands for, of
/// `libraries`.
pub(crate) fn user_slot(id: &str, prefix: &str, libraries: Vec<LibraryInfo>) -> Option<LibraryInfo> {
    let n: usize = id.strip_prefix(prefix)?.parse().ok()?;
    libraries.into_iter().filter(|l| l.category == "user").nth(n.checked_sub(1)?)
}

/// The panel's window: a tab strip with the library's name, the panel menu and a close button, then
/// the body. A library that no longer exists closes it.
fn window<K: LibraryKind>(app: &mut VectorcraftApp, ctx: &egui::Context, id: &str) {
    let Some((name, lib)) = K::get(app, id) else {
        app.ui.library_panel = None;
        return;
    };
    let t = Tokens::get(ctx);
    let mut open = true;
    // Left of the icon panels' pop-out, movable.
    let at = egui::pos2((ctx.content_rect().right() - 880.0).max(8.0), 140.0);
    let area =
        egui::Area::new(egui::Id::new(("library-panel", K::KIND))).order(egui::Order::Foreground).default_pos(at).movable(true).constrain(true);
    area.show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.panel).corner_radius(CornerRadius::same(4)).inner_margin(egui::Margin::ZERO).show(ui, |ui| {
            ui.set_width(256.0);
            let (strip, _) = ui.allocate_exact_size(vec2(256.0, 26.0), Sense::hover());
            ui.painter().rect_filled(strip, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, t.panel_darker);
            let name = library_name(id, &name);
            let label = egui::RichText::new(name).font(theme::semibold(12.0));
            let galley = ui.painter().layout_no_wrap(name.to_string(), theme::semibold(12.0), t.text);
            let tab = Rect::from_min_size(strip.min, vec2((galley.size().x + 24.0).min(190.0), 26.0));
            ui.painter().rect_filled(tab, CornerRadius { nw: 4, ne: 0, sw: 0, se: 0 }, t.panel);
            ui.put(tab.shrink2(vec2(12.0, 0.0)), egui::Label::new(label.color(t.text)).truncate().selectable(false));
            let close = Rect::from_center_size(strip.right_center() - vec2(13.0, 0.0), vec2(14.0, 14.0));
            let cr = ui.interact(close, ui.id().with("close-library"), Sense::click()).on_hover_text(tl!("Close"));
            icons::paint(ui, "x", close, if cr.hovered() { t.text } else { t.text_dim });
            open = !cr.clicked();
            let menu = Rect::from_center_size(strip.right_center() - vec2(34.0, 0.0), vec2(16.0, 16.0));
            let mr = ui.interact(menu, ui.id().with("library-menu"), Sense::click()).on_hover_text(tl!("Panel menu"));
            icons::paint(ui, "menu", menu.shrink(1.0), if mr.hovered() { t.text_strong } else { t.text_dim });
            egui::Popup::menu(&mr).show(|ui| {
                ui.set_min_width(200.0);
                panel_menu::<K>(app, ui, id);
            });
            egui::Frame::NONE.inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(236.0);
                body::<K>(app, ui, id, &lib);
            });
        });
    });
    if !open {
        app.ui.library_panel = None;
    }
}

/// The selected items and folders of library `id` (a selection in another library is dropped).
fn selection<K: LibraryKind>(ctx: &egui::Context, id: &str) -> Vec<String> {
    let (lib, sel): (String, Vec<String>) = pstate(ctx, &key::<K>("selected"));
    if lib == id { sel } else { vec![] }
}

fn set_selection<K: LibraryKind>(ctx: &egui::Context, id: &str, sel: Vec<String>) {
    set_pstate(ctx, &key::<K>("selected"), (id.to_string(), sel));
}

/// Find field, thumbnails or list rows, and the bottom bar.
fn body<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, lib: &K::Lib) {
    let view = view::<K>(ui.ctx());
    let query = if pstate::<bool>(ui.ctx(), &key::<K>("hide-find")) {
        String::new()
    } else {
        widgets::search_field(ui, egui::Id::new(key::<K>("find")), tl!("Find"))
    };
    ui.add_space(4.0);
    let rows = K::rows(lib, &query.trim().to_lowercase());
    let selected = selection::<K>(ui.ctx(), id);
    let is_sel = |n: &str| selected.iter().any(|s| s == n);
    let mut clicked: Option<(String, bool, egui::Modifiers)> = None;
    let mut click = |ui: &Ui, resp: egui::Response, row: &Row<K::Item>, tip: &dyn Fn() -> String| {
        if resp.on_hover_text(tip()).clicked() {
            clicked = Some((row.name.to_string(), row.item.is_none(), ui.input(|i| i.modifiers)));
        }
    };
    widgets::list_box(ui, |ui| {
        let max_height = if view == View::LargeThumb { 280.0 } else { 220.0 };
        egui::ScrollArea::vertical().id_salt(key::<K>("scroll")).max_height(max_height).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if rows.is_empty() {
                widgets::dim_label(ui, if query.trim().is_empty() { tl!("This library is empty.") } else { tl!("No matches.") });
            } else if view.is_list() {
                for row in &rows {
                    let (r, resp, chip) = list_row(ui, tile_id::<K>(row.name), view, is_sel(row.name));
                    match row.item {
                        Some(item) => {
                            K::draw(app, ui, chip, item, false, false);
                            K::row_icons(ui, r, item);
                        }
                        None => super::swatches::draw_folder(ui, chip),
                    }
                    list_row_name(ui, r, chip, row.name);
                    let tip = || row.item.map_or_else(|| row.name.to_string(), |i| format!("{} ({})", row.name, K::describe(i)));
                    click(ui, resp, row, &tip);
                }
            } else {
                for (row, cell) in tile_grid(ui, &rows, view, |r| r.item.is_none()) {
                    let resp = ui.interact(cell, tile_id::<K>(row.name), Sense::click());
                    match row.item {
                        Some(item) => K::draw(app, ui, cell, item, is_sel(row.name), resp.hovered()),
                        None => folder_tile(ui, cell, is_sel(row.name)),
                    }
                    click(ui, resp, row, &|| row.name.to_string());
                }
            }
        });
    });
    if let Some((name, folder, m)) = clicked {
        // A plain click on an item adds and applies it; folders and modifier clicks select.
        let only_select = folder || m.shift || m.command;
        let order: Vec<&str> = rows.iter().map(|r| r.name).collect();
        let sel = if only_select { click_selection(ui, &key::<K>("anchor"), selected.clone(), &order, &name, m) } else { vec![name.clone()] };
        set_selection::<K>(ui.ctx(), id, sel);
        if !only_select {
            K::click(app, ui, id, &name);
        }
    }
    bottom::<K>(app, ui, id);
}

/// The library `step` places after (or before, when negative) `id` in menu order, wrapping.
fn neighbour<K: LibraryKind>(app: &VectorcraftApp, id: &str, step: isize) -> Option<String> {
    let all = K::list(app);
    let i = all.iter().position(|l| l.id == id)? as isize;
    let n = all.len() as isize;
    all.into_iter().nth((i + step).rem_euclid(n) as usize).map(|l| l.id)
}

/// Open library `id` in the panel.
fn open<K: LibraryKind>(app: &mut VectorcraftApp, id: &str) {
    if let Err(e) = app.run(K::OPEN, json!({ "library": id })) {
        app.status(e);
    }
}

/// Libraries menu, previous / next library, and Add to … for the selection.
fn bottom<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let selected = selection::<K>(ui.ctx(), id);
    widgets::bottom_bar(ui, |ui| {
        let r = widgets::icon_button(ui, "library", K::MENU, false, 24.0);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(200.0);
            library_menu::<K>(app, ui);
        });
        for (icon, tip, step) in [("chevron-left", tl!("Previous Library"), -1), ("chevron-right", tl!("Next Library"), 1)] {
            if widgets::icon_button(ui, icon, tip, false, 24.0).clicked()
                && let Some(next) = neighbour::<K>(app, id, step)
            {
                open::<K>(app, &next);
            }
        }
        ui.add_space((ui.available_width() - 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-new-item", K::ADD, false, !selected.is_empty(), 24.0).clicked() {
            K::add(app, id, selected.clone());
        }
    });
}

/// The library panel's (≡) menu.
fn panel_menu<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let selected = selection::<K>(ui.ctx(), id);
    if menu_item(ui, K::ADD, !selected.is_empty(), false) {
        K::add(app, id, selected);
    }
    ui.separator();
    let view = view::<K>(ui.ctx());
    for (v, label) in View::ALL {
        if menu_item(ui, label, true, v == view) {
            set_pstate(ui.ctx(), &key::<K>("view"), Some(v));
        }
    }
    ui.separator();
    let hidden: bool = pstate(ui.ctx(), &key::<K>("hide-find"));
    if menu_item(ui, tl!("Show Find Field"), true, !hidden) {
        set_pstate(ui.ctx(), &key::<K>("hide-find"), !hidden);
    }
    ui.separator();
    for (label, step) in [(tl!("Previous Library"), -1), (tl!("Next Library"), 1)] {
        if menu_item(ui, label, true, false)
            && let Some(next) = neighbour::<K>(app, id, step)
        {
            open::<K>(app, &next);
        }
    }
    if menu_item(ui, tl!("Close Library"), true, false) {
        app.ui.library_panel = None;
    }
}

/// The libraries menu (the panel's library button, the Swatches panel's): every library by
/// submenu, the open one checked, between `K`'s head and tail items.
pub(crate) fn library_menu<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui) {
    K::menu_head(app, ui);
    let current = app.ui.library_panel.as_ref().filter(|o| o.kind == K::KIND).map(|o| o.id.clone());
    if let Some(id) = library_items(ui, &K::list(app), current.as_deref()) {
        open::<K>(app, &id);
    }
    K::menu_tail(app, ui);
}

/// Menu rows for `libs`: the top-level ones, then a submenu per [`LibraryRef::submenu`], with
/// `current` checked. Returns the id of the one clicked.
pub(crate) fn library_items(ui: &mut Ui, libs: &[LibraryRef], current: Option<&str>) -> Option<String> {
    let mut chosen = None;
    let mut item = |ui: &mut Ui, l: &LibraryRef| {
        if widgets::menu_item_name(ui, library_name(&l.id, &l.name), true, current == Some(l.id.as_str())) {
            chosen = Some(l.id.clone());
        }
    };
    for l in libs.iter().filter(|l| l.submenu.is_none()) {
        item(ui, l);
    }
    let mut subs: Vec<&str> = vec![];
    for s in libs.iter().filter_map(|l| l.submenu) {
        if !subs.contains(&s) {
            subs.push(s);
        }
    }
    for s in subs {
        ui.menu_button(tl!(s), |ui| {
            crate::widgets::menu_scroll(ui, |ui| {
                for l in libs.iter().filter(|l| l.submenu == Some(s)) {
                    item(ui, l);
                }
            });
        });
    }
    chosen
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2};
    use serde_json::Value;
    use vectorcraft_color::Paint;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::panels::swatches::SwatchLibraries;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    /// One frame of the library panel with `events` and `modifiers` held.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, modifiers: Modifiers, time: f64) {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show_window(app, ui.ctx()));
        out.textures_delta.clear();
    }

    /// Click the tile or row of `name` with `modifiers` (frames from `time` to `time + 0.5`; a new
    /// window takes a sizing frame first).
    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
        frame(app, ctx, vec![], Modifiers::NONE, time);
        frame(app, ctx, vec![], Modifiers::NONE, time + 0.25);
        let at = ctx.read_response(tile_id::<SwatchLibraries>(name)).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
        frame(app, ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5);
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    #[test]
    fn clicking_a_library_swatch_adds_and_applies_it_in_one_step() {
        let mut app = app();
        let ctx = context();
        let r = app.run("window.swatchLibrary", json!({"library": "Pastels"})).unwrap();
        assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("pastels"), Some(24)));
        assert_eq!(app.ui.library_panel, Some(OpenLibrary { kind: "swatches".into(), id: "pastels".into() }));
        let undo = undo_len(&app);
        click(&mut app, &ctx, "Pastel Teal", Modifiers::NONE, 0.0);
        let d = &app.session.doc().unwrap().doc;
        let added = d.swatch("Pastel Teal").expect("added to the document");
        assert_eq!(app.session.paint.fill, Paint::solid(added.paint.color().unwrap()), "applied to the fill");
        assert_eq!(undo_len(&app), undo + 1);
        // Alt-click paints the stroke (the inactive proxy).
        click(&mut app, &ctx, "Soft Blue", Modifiers::ALT, 2.0);
        let blue = app.session.doc().unwrap().doc.swatch("Soft Blue").and_then(|w| w.paint.color()).unwrap();
        assert_eq!(app.session.paint.stroke.color(), Some(blue));
        assert!(app.session.fill_active, "Alt keeps the active proxy");
        // A library that doesn't exist closes the panel; null closes it.
        assert!(app.run("window.swatchLibrary", json!({"library": "nope"})).is_err());
        app.run("window.swatchLibrary", json!({"library": null})).unwrap();
        assert!(app.ui.library_panel.is_none());
    }

    #[test]
    fn modifier_clicks_select_and_add_to_swatches_adds_the_selection() {
        let mut app = app();
        let ctx = context();
        app.run("window.swatchLibrary", json!({"library": "earth-tones"})).unwrap();
        click(&mut app, &ctx, "Clay 1", Modifiers::COMMAND, 0.0);
        click(&mut app, &ctx, "Ochre", Modifiers::NONE, 2.0);
        assert_eq!(selection::<SwatchLibraries>(&ctx, "earth-tones"), ["Ochre"], "a folder click selects its group");
        click(&mut app, &ctx, "Clay 3", Modifiers::COMMAND, 4.0);
        assert!(app.session.doc().unwrap().doc.swatch("Clay 3").is_none(), "modifier clicks only select");
        let sel = selection::<SwatchLibraries>(&ctx, "earth-tones");
        SwatchLibraries::add(&mut app, "earth-tones", sel);
        let d = &app.session.doc().unwrap().doc;
        assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Ochre").map(|g| g.swatches.len()), Some(6));
        assert!(d.swatches.iter().any(|w| w.name == "Clay 3"));
        // The selection belongs to its library.
        app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap();
        assert!(selection::<SwatchLibraries>(&ctx, "brights").is_empty());
    }

    #[test]
    fn find_filters_list_view_and_arrows_step_through_the_libraries() {
        let mut app = app();
        let ctx = context();
        app.run("window.swatchLibrary", json!({"library": "web-safe-216"})).unwrap();
        set_pstate(&ctx, &key::<SwatchLibraries>("view"), Some(View::SmallList));
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(key::<SwatchLibraries>("find")), "#FF00".to_string()));
        frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.0);
        frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.5);
        assert!(ctx.read_response(tile_id::<SwatchLibraries>("#FF0033")).is_some());
        assert!(ctx.read_response(tile_id::<SwatchLibraries>("#00FF00")).is_none());
        let first = SWATCH_IDS[0];
        let last = SwatchLibraries::list(&app).pop().unwrap().id;
        assert_eq!(last, "radial-glows", "gradient libraries come after the swatch libraries");
        assert_eq!(neighbour::<SwatchLibraries>(&app, first, -1), Some(last), "wraps around");
        assert_eq!(neighbour::<SwatchLibraries>(&app, "web-safe-216", 1).as_deref(), Some(SWATCH_IDS[1]));
    }

    const SWATCH_IDS: [&str; 9] = [
        "web-safe-216",
        "grays-neutrals",
        "earth-tones",
        "skin-tone-ramps",
        "pastels",
        "brights",
        "metallic-gradients",
        "perceptual-scales",
        "harmony-sets",
    ];

    #[test]
    fn the_window_submenu_lists_every_library() {
        let app = app();
        let ids: Vec<Value> = crate::menus::menu_entries(&app)
            .into_iter()
            .filter(|e| e.path == ["Window", "Swatch Libraries"] && e.command.as_deref() == Some("window.swatchLibrary"))
            .map(|e| e.params["library"].clone())
            .collect();
        let all = vectorcraft_engine::cmd::swatchlib::libraries(&app.session);
        let listed = |category: &str| all.iter().filter(|l| l.category == category).map(|l| json!(l.id)).collect::<Vec<_>>();
        assert_eq!(ids, listed("builtIn"));
        assert_eq!(ids, SWATCH_IDS.map(|i| json!(i)));
        let gradients: Vec<Value> = crate::menus::menu_entries(&app)
            .into_iter()
            .filter(|e| e.path == ["Window", "Swatch Libraries", "Gradients"])
            .map(|e| e.params["library"].clone())
            .collect();
        assert!(gradients.len() >= 5);
        assert_eq!(gradients, listed("gradients"));
        let defaults = crate::menus::menu_entries(&app).into_iter().any(|e| e.command.as_deref() == Some("swatch.resetDefaults"));
        assert!(defaults, "Default Swatches is listed");
    }

    #[test]
    fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
        let dir = std::env::temp_dir().join(format!("vc-library-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = app();
        app.session.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
        let user_items = |app: &VectorcraftApp| -> Vec<(String, String)> {
            crate::menus::menu_entries(app)
                .into_iter()
                .filter(|e| e.path == ["Window", "Swatch Libraries", "User Defined"])
                .map(|e| (e.label, e.command.unwrap_or_default()))
                .collect()
        };
        assert!(user_items(&app).is_empty(), "empty slots are hidden");
        app.run("ui.saveSwatchLibrary", json!({"names": ["Brights"]})).unwrap();
        let d = app.ui.dialog.as_mut().expect("Save Swatch Library opens");
        assert_eq!((d.kind.as_str(), d.bool("user")), (crate::dialogs::save_swatch_library::KIND, true));
        d.fields.insert("name".into(), json!("Loud"));
        d.fields.insert("selectedOnly".into(), json!(true));
        let ctx = context();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| crate::dialogs::show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.ui.dialog.is_some(), "the dialog draws and stays open");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(user_items(&app), [("Loud".to_string(), "window.userSwatchLibrary1".to_string())]);
        app.run("window.userSwatchLibrary1", json!({})).unwrap();
        assert_eq!(app.ui.library_panel.as_ref().map(|o| o.id.as_str()), Some("user/Loud.vcswatches"));
        let (_, lib) = SwatchLibraries::get(&app, "user/Loud.vcswatches").unwrap();
        assert_eq!((lib.groups.len(), lib.len()), (1, 5), "only the selected group");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_to_a_file_and_opening_a_palette_file() {
        let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(!d.bool("user") && !d.bool("__user"), "no user folder: a file");
        d.fields.insert("format".into(), json!("gpl"));
        crate::dialogs::confirm(&mut app).unwrap();
        let (path, bytes) = written.borrow()[0].clone();
        assert!(path.ends_with(".gpl"), "{path}");
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.starts_with(
                "GIMP Palette
"
            ) && text.contains(
                "	Red
"
            ),
            "{text}"
        );
        // Opening a palette file opens it in the library panel.
        crate::io::open_bytes(&mut app, "Saved.gpl", text.as_bytes(), None).unwrap();
        let open = app.ui.library_panel.clone().unwrap();
        assert!(open.id.starts_with("loaded/"), "{open:?}");
        assert_eq!(app.session.documents().len(), 1, "not opened as a document");
        let other = SwatchLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
        assert_eq!(other.submenu, Some("Other Libraries"));
        // Built-in libraries' names are interface labels; a file's is shown as it is.
        for l in SwatchLibraries::list(&app) {
            assert_eq!(builtin_library(&l.id), !matches!(l.submenu, Some("User Defined" | "Other Libraries")), "{}", l.id);
        }
        assert!(!builtin_library(&open.id) && builtin_library(vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES));
    }

    #[test]
    fn the_save_dialog_writes_swatch_exchange_files() {
        let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("format".into(), json!("ase"));
        d.fields.insert("name".into(), json!("Brand Kit"));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert!(text.contains("Swatch Exchange (.ase)") && text.contains("Swatch exchange files keep solid colors only."), "{text}");
        crate::dialogs::confirm(&mut app).unwrap();
        let (path, bytes) = written.borrow()[0].clone();
        assert_eq!(path, "/tmp/Brand Kit.ase", "the save panel suggests the name");
        let lib = vectorcraft_color::palette_io::read_bytes(&bytes, "x").unwrap();
        let solid = app.session.doc().unwrap().doc.swatches_iter().filter(|w| w.paint.color().is_some()).count();
        assert_eq!(lib.len(), solid, "every solid color of the document");
        // The web downloads the file under that name.
        let downloads = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let dl = downloads.clone();
        let services =
            crate::Services { download: Some(Box::new(move |name: &str, _: &[u8]| dl.borrow_mut().push(name.to_string()))), ..Default::default() };
        let mut web = VectorcraftApp::new(Session::new(), services);
        web.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        web.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = web.ui.dialog.as_mut().unwrap();
        d.fields.insert("format".into(), json!("ase"));
        d.fields.insert("name".into(), json!(" Brand: Kit. "));
        crate::dialogs::confirm(&mut web).unwrap();
        assert_eq!(*downloads.borrow(), ["Brand- Kit.ase"], "the name made safe as a file name, as for User Defined");
    }

    #[test]
    fn swatch_exchange_files_without_a_path_open_in_the_library_panel() {
        // A file opened on the web has no path; open_bytes passes its bytes as dataBase64.
        let mut app = app();
        crate::io::open_bytes(&mut app, "Brand.ase", &vectorcraft_testkit::ase::sample(), None).unwrap();
        let open = app.ui.library_panel.clone().unwrap();
        assert_eq!(open.id, "loaded/Brand.ase");
        let (_, lib) = SwatchLibraries::get(&app, &open.id).unwrap();
        assert_eq!((lib.len(), lib.groups.len()), (4, 1));
        assert_eq!(app.session.documents().len(), 1, "not opened as a document");
    }
}
