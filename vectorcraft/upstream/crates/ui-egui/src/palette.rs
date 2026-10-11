//! The command palette: fuzzy search over every command and tool.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

/// Everything the palette searches: (label, command id or `tool:<id>`, shortcut).
pub fn items() -> Vec<(String, String, String)> {
    let mut items = vec![];
    // Aliases kept for older scripts run a command that is listed already.
    for c in vectorcraft_engine::command_specs().iter().filter(|c| !vectorcraft_engine::cmd::is_alias(c.id)) {
        let path = c.menu.join(" › ");
        let label = if path.is_empty() { c.label.to_string() } else { format!("{path} › {}", c.label) };
        items.push((label, c.id.to_string(), menus::shortcut_of(c.id).unwrap_or("").to_string()));
    }
    for c in menus::UI_COMMANDS {
        items.push((c.1.to_string(), c.0.to_string(), menus::shortcut_of(c.0).unwrap_or("").to_string()));
    }
    for tool in vectorcraft_tools::catalog::all_tools() {
        items.push((tool.label.to_string(), format!("tool:{}", tool.id), crate::shortcut_editor::tool_shortcut(tool.id).unwrap_or("").to_string()));
    }
    items
}

/// A palette label in `lang`: each `›`-separated menu segment and the command label translate on
/// their own.
fn shown_label(lang: crate::i18n::Lang, label: &str) -> String {
    label.split(" › ").map(|s| crate::i18n::tr(lang, s)).collect::<Vec<_>>().join(" › ")
}

/// One palette entry, with what it shows and what a query matches worked out once.
struct Entry {
    id: String,
    shortcut: String,
    /// The label in the UI language.
    shown: String,
    /// The English label (what agents document), the shown label and the id, lowercased, one per
    /// line: a query word (which has no line break) matches within one of them.
    haystack: String,
}

/// The palette's entries in `lang`. They are built when the language, the installed plug-ins or
/// the keyboard shortcuts (shown beside each entry) change, not on every frame while a query is
/// typed.
/// What the entries depend on: the language, the installed plug-ins and the shortcuts.
fn cache_key(lang: crate::i18n::Lang) -> (&'static str, u64, u64) {
    (lang.code(), menus::plugin_revision(), crate::shortcut_editor::GENERATION.load(std::sync::atomic::Ordering::Relaxed))
}

fn entries(ctx: &egui::Context, lang: crate::i18n::Lang) -> Arc<Vec<Entry>> {
    type Cached = ((&'static str, u64, u64), Arc<Vec<Entry>>);
    let key = cache_key(lang);
    let id = egui::Id::new("palette-entries");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && k == key
    {
        return v;
    }
    let built: Arc<Vec<Entry>> = Arc::new(
        items()
            .into_iter()
            .map(|(label, id, shortcut)| {
                let shown = shown_label(lang, &label);
                let haystack = format!("{}\n{}\n{}", label.to_lowercase(), shown.to_lowercase(), id.to_lowercase());
                Entry { id, shortcut, shown, haystack }
            })
            .collect(),
    );
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, built.clone())));
    built
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        return;
    }
    let t = Tokens::get(ctx);
    let q = app.ui.palette_query.to_lowercase();
    let entries = entries(ctx, crate::i18n::current());
    let matches: Vec<&Entry> = entries.iter().filter(|e| q.split_whitespace().all(|w| e.haystack.contains(w))).take(14).collect();
    let mut run: Option<String> = None;
    egui::Area::new(egui::Id::new("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.panel).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(520.0);
            let r = ui.add(egui::TextEdit::singleline(&mut app.ui.palette_query).hint_text(tl!("Search commands and tools…")).desired_width(500.0));
            r.request_focus();
            ui.add_space(6.0);
            for (i, e) in matches.iter().enumerate() {
                let resp = ui.add(
                    egui::Button::new(e.shown.as_str())
                        .shortcut_text(menus::pretty_shortcut(&e.shortcut))
                        .min_size(egui::vec2(500.0, 24.0))
                        .selected(i == 0),
                );
                if resp.clicked() {
                    run = Some(e.id.clone());
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(first) = matches.first()
            {
                run = Some(first.id.clone());
            }
        });
    });
    if let Some(id) = run {
        app.ui.palette_open = false;
        if let Some(tool) = id.strip_prefix("tool:") {
            app.select_tool(tool);
        } else {
            menus::invoke(app, &id, json!({}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn entries_are_built_once_per_language_and_match_english_shown_text_and_ids() {
        let ctx = egui::Context::default();
        let zh = Lang::from_code("zh-hant").unwrap();
        // Tests running alongside edit shortcuts and install plug-ins, which rebuilds the entries:
        // look twice until nothing changed in between.
        let (en, again) = (0..100)
            .find_map(|_| {
                let key = cache_key(Lang::EN);
                let pair = (entries(&ctx, Lang::EN), entries(&ctx, Lang::EN));
                (cache_key(Lang::EN) == key).then_some(pair)
            })
            .unwrap();
        assert!(Arc::ptr_eq(&en, &again), "kept while the language stays");
        let zh_entries = entries(&ctx, zh);
        assert!(!Arc::ptr_eq(&en, &zh_entries), "rebuilt for another language");
        // Editing a shortcut rebuilds them, so the palette shows the new one.
        crate::shortcut_editor::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(!Arc::ptr_eq(&zh_entries, &entries(&ctx, zh)), "rebuilt after a shortcut edit");
        let group = zh_entries.iter().find(|e| e.id == "object.group").unwrap();
        assert!(group.shown.ends_with(crate::i18n::tr(zh, "Group")), "{}", group.shown);
        // A query matches the English label, the shown label or the id.
        for word in ["group", crate::i18n::tr(zh, "Group"), "object.group"] {
            assert!(group.haystack.contains(&word.to_lowercase()), "{word}");
        }
    }
}
