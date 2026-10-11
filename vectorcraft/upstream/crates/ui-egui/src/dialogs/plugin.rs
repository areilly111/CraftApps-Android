//! WebAssembly plug-ins in the menus and their generated dialogs.
//!
//! Object › Plug-ins lists the installed object filters (`plugin.dialog {id}`), then Install
//! Plug-in… and Reload Plug-ins; Effect › Plug-ins lists the live-effect plug-ins, which open the
//! effect dialog (`effect.dialog`). A filter with parameters opens dialog `plugin`: one field per
//! declared parameter (fields named after them; `__plugin` is the plug-in id) with a live
//! Preview, and OK keeps the run as one undo step (`plugin.run`). A filter without parameters runs
//! at once.

use serde_json::{Map, Value, json};
use vectorcraft_plugins::{Kind, registry};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::state::Dialog;

/// The dialog kind of plug-in filters.
pub const KIND: &str = "plugin";

const CMD: &str = "plugin.run";

/// Object › Plug-ins' header while no filter plug-in is installed.
pub const NO_FILTERS: &str = "No filter plug-ins installed";

/// Headed by the plug-in's name, as it is (plug-in text is never translated).
pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |d| d.str("__label"), body, confirm, preview: true, max_width: Some(420.0), ..DialogSpec::FORM };

/// A menu label for plug-in `name`: with an ellipsis when it opens a dialog.
fn label(name: &str, has_params: bool) -> &'static str {
    let dots = if has_params && !name.ends_with('…') { "…" } else { "" };
    registry::intern(&format!("{name}{dots}"))
}

/// Object › Plug-ins: the installed object filters, then Install Plug-in… and Reload Plug-ins.
pub fn object_menu() -> Vec<Item> {
    let mut items: Vec<Item> = registry::list()
        .iter()
        .filter(|p| p.manifest().kind == Kind::Filter)
        .map(|p| Item::Cmd(label(&p.manifest().name, !p.manifest().params.is_empty()), "plugin.dialog", json!({ "id": p.id() })))
        .collect();
    if items.is_empty() {
        items.push(Item::Header(NO_FILTERS));
    }
    items.extend([
        Item::Sep,
        Item::Cmd("Install Plug-in…", "ui.installPlugin", Value::Null),
        Item::Cmd("Reload Plug-ins", "plugin.reload", Value::Null),
    ]);
    items
}

/// Effect › Plug-ins: the live-effect plug-ins (`None` while none is installed). Effects without
/// options apply at once, as built-in ones do.
pub fn effect_menu() -> Option<Item> {
    let items: Vec<Item> = vectorcraft_effects::plugin_effects()
        .into_iter()
        .map(|e| match e.defaults.as_object().is_some_and(|o| o.is_empty()) {
            true => Item::Cmd(e.label, "effect.apply", json!({ "effect": e.id })),
            false => Item::Cmd(e.label, "effect.dialog", json!({ "effect": e.id })),
        })
        .collect();
    (!items.is_empty()).then_some(Item::Sub("Plug-ins", items))
}

/// `plugin.dialog {id, params?}`: run filter `id` at once when it takes no parameters or `params`
/// are given, else open its dialog prefilled with its defaults.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("missing `id` (see plugin.list)")?;
    let plugin = registry::get(id).ok_or_else(|| format!("no plug-in {id:?} is installed"))?;
    let m = plugin.manifest();
    if m.kind != Kind::Filter {
        return Err(format!("{} is a live effect (Effect › Plug-ins)", m.name));
    }
    let given = p.get("params").filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
    if m.params.is_empty() || given.is_some() {
        return app.run(CMD, json!({"id": id, "params": given.cloned().unwrap_or(Value::Null)}));
    }
    let mut fields: Map<String, Value> = m.defaults();
    fields.insert("__plugin".into(), json!(id));
    fields.insert("__label".into(), json!(m.name.trim_end_matches('…')));
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

fn params(d: &Dialog) -> Value {
    json!({"id": d.str("__plugin"), "params": form::params(d)})
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // A plug-in uninstalled while its dialog is open closes it.
    let Some(plugin) = registry::get(&d.str("__plugin")) else { return true };
    let m = plugin.manifest();
    if !m.description.is_empty() {
        ui.label(egui::RichText::new(&m.description).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
        ui.add_space(8.0);
    }
    form::schema_fields(ui, d, &m.params);
    let (label, p) = (d.str("__label"), params(d));
    form::preview(app, ui, d, &label, CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}
