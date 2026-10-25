//! Touch-first Android workspace. Sheets replace desktop sidebars on narrow screens.
use crate::{LeftPanel, PdfCraftApp, QuickTool, RightPanel, canvas, icons, panels, theme::Tokens};

pub(crate) fn enabled(ctx: &egui::Context) -> bool {
    cfg!(target_os = "android") && ctx.content_rect().width() < 900.0
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sheet {
    #[default]
    Document,
    Tools,
    Inspector,
    More,
    Documents,
}

fn action(ui: &mut egui::Ui, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let primary = label == "Open a PDF";
    let text = egui::RichText::new(tl!(label)).color(if primary { egui::Color32::WHITE } else { t.text });
    ui.add_sized(
        [ui.available_width(), 48.0],
        egui::Button::new(text)
            .fill(if primary { t.accent } else { t.card })
            .stroke(egui::Stroke::new(1.0, if primary { t.accent } else { t.border }))
            .corner_radius(8)
            .truncate(),
    )
    .clicked()
}

pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let state_id = egui::Id::new("android-mobile-sheet");
    let mut sheet = ctx.data(|d| d.get_temp::<Sheet>(state_id)).unwrap_or_default();
    let first = ctx.data_mut(|d| {
        let id = state_id.with("initialized");
        let first = !d.get_temp::<bool>(id).unwrap_or(false);
        d.insert_temp(id, true);
        first
    });
    if first {
        app.left_open = false;
        app.choose_right_panel(None);
    }
    // Commands dispatched by the palette can request a tool or inspector.
    if sheet == Sheet::Document {
        if app.right.is_some() {
            sheet = Sheet::Inspector;
        } else if app.left_open {
            sheet = Sheet::Tools;
        }
    }
    if let Some(index) = app.active
        && let Some(view) = app.views.get_mut(index)
    {
        let id = state_id.with(("fitted", view.id));
        if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
            view.set_fit(canvas::Fit::Width);
            view.set_layout(canvas::PageLayout::Single);
            ctx.data_mut(|d| d.insert_temp(id, true));
            app.quick_tool = QuickTool::Hand;
        }
    }
    egui::Panel::top("mobile-header").exact_size(52.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(4)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal_centered(|ui| {
            let back = sheet != Sheet::Document;
            if icons::button(ui, if back { "chevron-left" } else { "panel-left" }, 44.0, false, if back { tl!("Back") } else { tl!("Menu") })
                .clicked()
            {
                if back {
                    sheet = Sheet::Document;
                    app.left_open = false;
                    app.choose_right_panel(None);
                } else {
                    sheet = Sheet::More;
                }
            }
            let title = app.active.and_then(|i| canvas::tab_label(app, i)).unwrap_or_else(|| "PdfCraft".into());
            let title_width = (ui.available_width() - 92.0).max(1.0);
            if ui.add_sized([title_width, 44.0], egui::Button::new(title).truncate().frame(false)).on_hover_text(tl!("Open documents")).clicked() {
                sheet = Sheet::Documents;
            }
            if icons::button(ui, "folder-open", 44.0, false, tl!("Open")).clicked() {
                sheet = Sheet::Document;
                app.left_open = false;
                app.choose_right_panel(None);
                app.run_command("file.open");
            }
            ui.add_enabled_ui(app.active.is_some(), |ui| {
                if icons::button(ui, "save", 44.0, false, tl!("Save")).clicked() {
                    app.run_command("file.save");
                }
            });
        });
    });
    egui::Panel::bottom("mobile-navigation").exact_size(54.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(4)).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = ((ui.available_width() - 12.0) / 4.0).max(1.0);
            for (label, target) in [("Tools", Sheet::Tools), ("Pages", Sheet::Inspector), ("Search", Sheet::Inspector), ("More", Sheet::More)] {
                let on = match label {
                    "Pages" => sheet == target && app.right == Some(RightPanel::Pages),
                    "Search" => sheet == target && app.right == Some(RightPanel::Search),
                    _ => sheet == target,
                };
                let translated = if label == "Tools" { tl_ctx!("mobile-nav", "Tools") } else { tl!(label) };
                let text = egui::RichText::new(translated).size(12.0);
                if ui.add_sized([width, 44.0], egui::Button::new(text).selected(on).truncate()).clicked() {
                    if on {
                        sheet = Sheet::Document;
                        app.left_open = false;
                        app.choose_right_panel(None);
                    } else {
                        sheet = target;
                        app.left_open = target == Sheet::Tools;
                        app.choose_right_panel(match label {
                            "Pages" => Some(RightPanel::Pages),
                            "Search" => Some(RightPanel::Search),
                            _ => None,
                        });
                        if label == "Tools" {
                            app.left = LeftPanel::AllTools;
                        }
                        if label == "Search" && app.active.is_some() {
                            app.run_command("edit.find");
                        }
                    }
                }
            }
        });
    });
    if sheet == Sheet::Document
        && let Some(index) = app.active
    {
        let count = app.views.get(index).and_then(|v| app.session.get(v.id)).map_or(0, |d| d.info.pages.len());
        egui::Panel::bottom("mobile-page-navigation").exact_size(48.0).frame(egui::Frame::NONE.fill(t.chrome).inner_margin(2)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal_centered(|ui| {
                if let Some(view) = app.views.get_mut(index) {
                    if icons::button(ui, "chevron-left", 44.0, false, tl!("Previous page")).clicked() {
                        view.go_to_page(view.current.saturating_sub(1));
                    }
                    let width = (ui.available_width() - 184.0).max(1.0);
                    ui.add_sized([width, 44.0], egui::Label::new(format!("{} / {}", view.current + 1, count)).truncate());
                    if icons::button(ui, "chevron-right", 44.0, false, tl!("Next page")).clicked() {
                        view.go_to_page(view.current.saturating_add(1).min(count.saturating_sub(1)));
                    }
                    if icons::button(ui, "maximize", 44.0, false, tl!("Fit to width")).clicked() {
                        view.set_fit(canvas::Fit::Width);
                    }
                    if icons::button(ui, "zoom-out", 44.0, false, tl!("Zoom out")).clicked() {
                        view.zoom_step(false);
                    }
                    if icons::button(ui, "zoom-in", 44.0, false, tl!("Zoom in")).clicked() {
                        view.zoom_step(true);
                    }
                }
            });
        });
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| {
        ui.spacing_mut().interact_size.y = 44.0;
        match sheet {
            Sheet::Document => {
                if let Some(index) = app.active {
                    canvas::document_area(app, index, ui);
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(18).show(ui, |ui| {
                            ui.add_space(20.0);
                            ui.heading("PdfCraft");
                            ui.label(tl!("Read, edit and organize your PDFs"));
                            ui.add_space(20.0);
                            if action(ui, "Open a PDF") {
                                app.run_command("file.open");
                            }
                            ui.add_space(16.0);
                            ui.label(tl!("Tools"));
                            for group in pdfcraft_engine::catalog::TOOL_GROUPS.iter().take(8) {
                                if action(ui, group.label) {
                                    app.select_tool(group.id);
                                    sheet = Sheet::Tools;
                                }
                            }
                        });
                    });
                }
            }
            Sheet::Tools => {
                let before = app.quick_tool;
                let organizing = app.active.and_then(|i| app.views.get(i)).is_some_and(|v| v.organize);
                panels::left_panel(app, ui);
                let opened_grid = !organizing && app.active.and_then(|i| app.views.get(i)).is_some_and(|v| v.organize);
                if !app.left_open || before != app.quick_tool || app.dialog.is_some() || opened_grid {
                    sheet = Sheet::Document;
                    app.left_open = false;
                }
            }
            Sheet::Inspector => {
                if app.active.is_some() {
                    panels::right_panel(app, ui);
                    if app.right.is_none() {
                        sheet = Sheet::Document;
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(tl!("Open a PDF first"));
                    });
                }
                if let Some(view) = app.active.and_then(|i| app.views.get_mut(i))
                    && let Some(doc) = app.session.get(view.id)
                {
                    view.queue_inspector_search(doc.info.pages.len());
                }
            }
            Sheet::More => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::NONE.inner_margin(12).show(ui, |ui| {
                        ui.heading(tl!("Menu"));
                        for (label, command) in [
                            ("Save as…", "file.save_as"),
                            ("Undo", "edit.undo"),
                            ("Redo", "edit.redo"),
                            ("Preferences…", "app.preferences"),
                            ("Close file", "file.close"),
                        ] {
                            let ready = pdfcraft_engine::commands::command(command).is_some_and(|spec| app.command_enabled(spec));
                            ui.add_enabled_ui(ready, |ui| {
                                if action(ui, label) {
                                    app.run_command(command);
                                    sheet = Sheet::Document;
                                    app.left_open = false;
                                    app.choose_right_panel(None);
                                }
                            });
                        }
                        if action(ui, "Select text") {
                            app.quick_tool = QuickTool::Select;
                            sheet = Sheet::Document;
                        }
                        if action(ui, "Hand") {
                            app.quick_tool = QuickTool::Hand;
                            sheet = Sheet::Document;
                        }
                        if action(ui, "Find tools and commands") {
                            app.palette_open = true;
                            sheet = Sheet::Document;
                        }
                    });
                });
            }
            Sheet::Documents => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::NONE.inner_margin(12).show(ui, |ui| {
                        ui.heading(tl!("Open documents"));
                        for index in 0..app.views.len() {
                            if let Some(name) = canvas::tab_label(app, index)
                                && action(ui, &name)
                            {
                                app.active = Some(index);
                                sheet = Sheet::Document;
                            }
                        }
                        if action(ui, "Open…") {
                            app.run_command("file.open");
                            sheet = Sheet::Document;
                        }
                    });
                });
            }
        }
    });
    // Do not let desktop docking requests steal space from the phone document.
    if sheet != Sheet::Tools {
        app.left_open = false;
    }
    if sheet != Sheet::Inspector {
        app.choose_right_panel(None);
    }
    ctx.data_mut(|d| d.insert_temp(state_id, sheet));
}
