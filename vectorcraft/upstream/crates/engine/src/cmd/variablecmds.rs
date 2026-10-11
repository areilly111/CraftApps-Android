//! Variables (data merge): named values bound to objects, applied per dataset.
//!
//! One template document produces many variants: bind a type object to a `text`
//! variable and anything to a `visibility` variable, put a row of values in each
//! dataset, and `dataset.select` swaps the whole document to that row in one undo
//! step. Agents reach every command through `run_command`, like all engine commands.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use vectorcraft_doc::{DataSet, DataValue, NodeId, NodeKind, Variable, VariableKind, Variables};

use super::edit::selected_roots;
use super::textedit::set_plain_text;
use super::*;

/// The longest variable or dataset name, in characters (names arrive from MCP and files).
const MAX_NAME: usize = 255;

fn var_name(p: &Value, cmd: &str) -> Result<String> {
    named(str_param(p, "name"), "name", cmd)
}

/// `key`'s value, as a name: trimmed, not empty and at most [`MAX_NAME`] characters.
fn named(v: Option<&str>, key: &str, cmd: &str) -> Result<String> {
    match v.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.chars().count() > MAX_NAME => Err(bad(cmd, format!("`{key}` is longer than {MAX_NAME} characters"))),
        Some(v) => Ok(v.to_string()),
        None => Err(bad(cmd, format!("missing `{key}`"))),
    }
}

/// `key`'s value as a name, or `None` when it is missing or empty.
fn optional_name(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match str_param(p, key).map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => named(Some(v), key, cmd).map(Some),
        None => Ok(None),
    }
}

/// The `names` of a delete, or without them `fallback` (what the panel acts on) as the one name.
fn names_param(p: &Value, cmd: &str, fallback: Option<String>, nothing: &str) -> Result<Vec<String>> {
    let Some(given) = p.get("names") else {
        return fallback.map(|n| vec![n]).ok_or_else(|| bad(cmd, nothing.to_string()));
    };
    match given.as_array() {
        Some(names) if !names.is_empty() => names.iter().map(|n| named(n.as_str(), "names", cmd)).collect(),
        _ => Err(bad(cmd, "`names` must be a list of names")),
    }
}

/// The objects a bind acts on: `ids` when given (every one checked), else the selection.
fn target_ids(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    if p.get("ids").is_some() { checked_ids_param(s, p, "ids", cmd) } else { selected_roots(s) }
}

/// Refuse what a variable of `kind` can't drive: art that is locked or on a hidden or locked
/// layer is not the document's to rewrite, and a text variable only drives type.
fn check_bindable(doc: &vectorcraft_doc::Document, ids: &[NodeId], kind: VariableKind, cmd: &str) -> Result<()> {
    for id in ids {
        match doc.node(*id) {
            None => return Err(EngineError::NoNode(*id)),
            Some(_) if !doc.variable_target(*id) => return Err(bad(cmd, format!("node {} is locked", id.0))),
            Some(n) if kind == VariableKind::Text && !matches!(n.kind, NodeKind::Text(_)) => {
                return Err(bad(cmd, format!("node {} is not type", id.0)));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn dataset_value(cmd: &str, var: &str, kind: VariableKind, v: &Value) -> Result<DataValue> {
    match kind {
        VariableKind::Text => v.as_str().map(|t| DataValue::Text(t.to_string())),
        VariableKind::Visibility => v.as_bool().map(DataValue::Visible),
    }
    .ok_or_else(|| bad(cmd, format!("value for `{var}` must match its variable kind")))
}

/// The `values` object of `dataset.new` / `dataset.set`, each entry checked against its
/// variable's kind. Every name must be a variable the document defines: a value nothing
/// reads is a typo, not a value to keep.
fn values_param(vars: &Variables, p: &Value, cmd: &str) -> Result<BTreeMap<String, DataValue>> {
    let mut out = BTreeMap::new();
    let Some(given) = p.get("values") else { return Ok(out) };
    let Some(obj) = given.as_object() else { return Err(bad(cmd, "`values` must be an object")) };
    for (var, v) in obj {
        let kind = vars.variable(var).map(|d| d.kind).ok_or_else(|| bad(cmd, format!("no variable named `{var}`")))?;
        out.insert(var.clone(), dataset_value(cmd, var, kind, v)?);
    }
    Ok(out)
}

fn define(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.define";
    let name = var_name(p, C)?;
    let kind = str_param(p, "kind").and_then(VariableKind::parse).ok_or_else(|| bad(C, "`kind` must be `text` or `visibility`"))?;
    if s.doc()?.doc.variables.variable(&name).is_some() {
        return Err(bad(C, format!("variable `{name}` is already defined")));
    }
    s.edit("New Variable", |d, _| {
        d.variables.variables.push(Variable { name: name.clone(), kind });
        Ok(())
    })?;
    Ok(json!({"name": name, "kind": kind.label()}))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.rename";
    let from = var_name(p, C)?;
    let to = named(p.get("newName").and_then(Value::as_str), "newName", C)?;
    let st = s.doc()?;
    if st.doc.variables.variable(&to).is_some() {
        return Err(bad(C, format!("variable `{to}` already exists")));
    }
    s.edit("Rename Variable", |d, _| {
        let v = d.variables.variables.iter_mut().find(|v| v.name == from).ok_or_else(|| bad(C, format!("no variable named `{from}`")))?;
        v.name = to.clone();
        for bound in d.variables.bindings.values_mut() {
            if bound.remove(&from) {
                bound.insert(to.clone());
            }
        }
        for ds in &mut d.variables.datasets {
            if let Some(value) = ds.values.remove(&from) {
                ds.values.insert(to.clone(), value);
            }
        }
        Ok(())
    })?;
    // The highlight names a variable, so it follows the rename rather than going stale.
    let st = s.doc_mut()?;
    if st.variables_highlight.as_deref() == Some(from.as_str()) {
        st.variables_highlight = Some(to.clone());
    }
    st.revision += 1;
    Ok(json!({"from": from, "name": to}))
}

/// Make Text Dynamic / Make Visibility Dynamic: define a variable of that kind and bind the
/// selection to it in one step, naming it after what it drives — the panel's buttons.
fn make_dynamic(s: &mut Session, kind: VariableKind, stem: &str) -> Result<Value> {
    const C: &str = "variable.makeDynamic";
    let ids = selected_roots(s)?;
    if ids.is_empty() {
        return Err(bad(C, "select the objects to bind"));
    }
    let doc = &s.doc()?.doc;
    check_bindable(doc, &ids, kind, C)?;
    // Named after the first object it drives, as the reference panel does; a second variable
    // over the same art takes a number rather than colliding.
    let shown = ids.first().and_then(|id| doc.node(*id)).map(|n| n.display_name()).unwrap_or_default();
    let base: String = match shown.trim() {
        "" => stem.to_string(),
        b => b.chars().take(MAX_NAME - 8).collect(),
    };
    let mut name = base.clone();
    let mut n = 2;
    while doc.variables.variable(&name).is_some() {
        name = format!("{base} {n}");
        n += 1;
    }
    s.edit("Make Dynamic", |d, _| {
        d.variables.variables.push(Variable { name: name.clone(), kind });
        for id in &ids {
            d.variables.bind(*id, &name);
        }
        Ok(())
    })?;
    Ok(json!({"name": name, "kind": kind.label(), "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    // Without `names`, the highlighted variable: what the panel's bin and the menu item delete.
    let highlighted = s.doc()?.variables_highlight.clone();
    let names = names_param(p, "variable.delete", highlighted, "no variable highlighted")?;
    let n = s.edit("Delete Variables", |d, _| Ok(d.variables.prune(&names)))?;
    // A highlight of a variable that is gone highlights nothing.
    let st = s.doc_mut()?;
    if st.variables_highlight.as_ref().is_some_and(|h| st.doc.variables.variable(h).is_none()) {
        st.variables_highlight = None;
    }
    st.revision += 1;
    Ok(json!({"deleted": n}))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let variables: Vec<Value> = st
        .doc
        .variables
        .variables
        .iter()
        .map(|v| {
            let bindings = st.doc.variables.objects_of(&v.name).len();
            json!({"name": v.name, "kind": v.kind.label(), "bindings": bindings})
        })
        .collect();
    Ok(json!({"variables": variables}))
}

fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.bind";
    let name = named(str_param(p, "variable"), "variable", C)?;
    let ids = target_ids(s, p, C)?;
    if ids.is_empty() {
        return Err(bad(C, "nothing to bind"));
    }
    let doc = &s.doc()?.doc;
    let kind = doc.variables.variable(&name).map(|v| v.kind).ok_or_else(|| bad(C, format!("no variable named `{name}`")))?;
    check_bindable(doc, &ids, kind, C)?;
    // Binding again is not a second binding, and says so rather than counting as a change.
    let already: Vec<u64> = ids.iter().filter(|id| doc.variables.objects_of(&name).contains(id)).map(|id| id.0).collect();
    let n = s.edit("Bind Variable", |d, _| Ok(ids.iter().filter(|id| d.variables.bind(**id, &name)).count()))?;
    Ok(json!({"variable": name, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "bound": n, "already": already}))
}

fn unbind(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.unbind";
    let ids = target_ids(s, p, C)?;
    let name = defined_variable(s, p, C)?;
    let n = s.edit("Unbind Variable", |d, _| {
        let mut n = 0;
        for id in &ids {
            // With `variable` only that one binding goes; without it, every one the object has.
            match &name {
                Some(v) => n += usize::from(d.variables.unbind(*id, v)),
                None => n += usize::from(d.variables.bindings.remove(id).is_some()),
            }
        }
        Ok(n)
    })?;
    Ok(json!({"unbound": n}))
}

/// The `variable` a command names, which must be defined; `None` when it names none.
fn defined_variable(s: &Session, p: &Value, cmd: &str) -> Result<Option<String>> {
    let name = optional_name(p, "variable", cmd)?;
    if let Some(v) = &name
        && s.doc()?.doc.variables.variable(v).is_none()
    {
        return Err(bad(cmd, format!("no variable named `{v}`")));
    }
    Ok(name)
}

/// Highlight one Variables panel row: what the panel's Delete, Options… and Select Bound
/// Object act on. Panel state, not art selection: not saved, not an undo step (an empty
/// or missing `variable` clears it), like `layer.highlight`.
fn highlight(s: &mut Session, p: &Value) -> Result<Value> {
    let name = defined_variable(s, p, "variable.highlight")?;
    let st = s.doc_mut()?;
    st.variables_highlight = name.clone();
    st.revision += 1;
    Ok(json!({"variable": name}))
}

fn dataset_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.new";
    let name = var_name(p, C)?;
    let st = s.doc()?;
    // A dataset and a variable are separate namespaces, so a dataset may share a variable's
    // name; it is its own row of values either way.
    let values = values_param(&st.doc.variables, p, C)?;
    if st.doc.variables.dataset(&name).is_some() {
        return Err(bad(C, format!("dataset `{name}` already exists")));
    }
    s.edit("New Data Set", |d, _| {
        d.variables.datasets.push(DataSet { name: name.clone(), values: values.clone() });
        Ok(())
    })?;
    Ok(json!({"name": name}))
}

/// Replace a dataset's values. A variable left out of `values` loses its value in this row,
/// so the dataset ends up holding exactly what the call asked for.
fn dataset_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.set";
    let name = var_name(p, C)?;
    let values = values_param(&s.doc()?.doc.variables, p, C)?;
    let (set, removed) = s.edit("Edit Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|x| x.name == name).ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        let removed: Vec<String> = ds.values.keys().filter(|k| !values.contains_key(*k)).cloned().collect();
        ds.values = values.clone();
        Ok((values.len(), removed))
    })?;
    Ok(json!({"name": name, "values": set, "removed": removed}))
}

fn dataset_rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.rename";
    let from = var_name(p, C)?;
    let to = named(p.get("newName").and_then(Value::as_str), "newName", C)?;
    let st = s.doc()?;
    if st.doc.variables.dataset(&to).is_some() {
        return Err(bad(C, format!("dataset `{to}` already exists")));
    }
    s.edit("Rename Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|d| d.name == from).ok_or_else(|| bad(C, format!("no dataset named `{from}`")))?;
        ds.name = to.clone();
        if d.variables.active_dataset.as_deref() == Some(from.as_str()) {
            d.variables.active_dataset = Some(to.clone());
        }
        Ok(())
    })?;
    Ok(json!({"from": from, "name": to}))
}

/// Capture Data Set: record what the bound objects hold right now as a new row. This, rather
/// than typing values, is how rows are made — change the art, then capture it.
fn dataset_capture(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.capture";
    let st = s.doc()?;
    let values = st.doc.captured_values();
    if values.is_empty() {
        return Err(bad(C, "bind something to a variable first"));
    }
    let name = match optional_name(p, "name", C)? {
        Some(n) => n,
        // "Data Set 1", the first free number, as the reference panel's rows are named.
        None => (1..).map(|i| format!("Data Set {i}")).find(|n| st.doc.variables.dataset(n).is_none()).unwrap_or_default(),
    };
    if s.doc()?.doc.variables.dataset(&name).is_some() {
        return Err(bad(C, format!("dataset `{name}` already exists")));
    }
    s.edit("Capture Data Set", |d, _| {
        d.variables.datasets.push(DataSet { name: name.clone(), values: values.clone() });
        d.variables.active_dataset = Some(name.clone());
        Ok(())
    })?;
    Ok(json!({"name": name, "values": values.len()}))
}

/// Update Data Set: write what the bound objects hold now into the row that is active. Without
/// an active row there is nothing to update, so say so rather than picking one.
fn dataset_update(s: &mut Session, _p: &Value) -> Result<Value> {
    const C: &str = "dataset.update";
    let values = s.doc()?.doc.captured_values();
    let Some(name) = s.doc()?.doc.variables.active_dataset.clone() else {
        return Err(bad(C, "no active data set"));
    };
    if values.is_empty() {
        return Err(bad(C, "bind something to a variable first"));
    }
    s.edit("Update Data Set", |d, _| {
        let ds = d.variables.datasets.iter_mut().find(|d| d.name == name).ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        ds.values = values.clone();
        Ok(())
    })?;
    Ok(json!({"name": name, "values": values.len()}))
}

fn dataset_delete(s: &mut Session, p: &Value) -> Result<Value> {
    // Without `names`, the active dataset: what the panel's Delete Data Set deletes.
    let active = s.doc()?.doc.variables.active_dataset.clone();
    let names = names_param(p, "dataset.delete", active, "no active data set")?;
    let n = s.edit("Delete Data Sets", |d, _| {
        let before = d.variables.datasets.len();
        d.variables.datasets.retain(|d| !names.contains(&d.name));
        // The active dataset's own name is the one that goes, not a variable's.
        if d.variables.active_dataset.as_ref().is_some_and(|a| names.contains(a)) {
            d.variables.active_dataset = None;
        }
        Ok(before - d.variables.datasets.len())
    })?;
    Ok(json!({"deleted": n}))
}

fn dataset_list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let current = st.doc.captured_values();
    let datasets: Vec<Value> = st
        .doc
        .variables
        .datasets
        .iter()
        // `matches` says whether the art still holds what the row says, which is what the
        // panel shows in italics.
        .map(|d| json!({"name": d.name, "values": d.values, "matches": d.matches(&current)}))
        .collect();
    Ok(json!({"datasets": datasets, "active": st.doc.variables.active_dataset}))
}

fn apply_dataset(s: &mut Session, name: &str) -> Result<Value> {
    const C: &str = "dataset.select";
    let (applied, skipped) = s.edit("Apply Data Set", |d, _| {
        let ds = d.variables.dataset(name).cloned().ok_or_else(|| bad(C, format!("no dataset named `{name}`")))?;
        // One object may be driven by several variables; walk them in definition order, so two
        // text variables on the same object land the later one's value and both are counted.
        let mut pairs: Vec<(NodeId, String)> = Vec::new();
        for v in &d.variables.variables {
            pairs.extend(d.variables.objects_of(&v.name).into_iter().map(|id| (id, v.name.clone())));
        }
        let mut applied = 0usize;
        let mut skipped = 0usize;
        for (id, var) in pairs {
            // A binding whose variable is gone, whose value this row doesn't carry, or whose
            // value doesn't fit the variable's kind, is left alone and counted, never
            // silently reinterpreted.
            let (kind, value) = match (d.variables.variable(&var).map(|v| v.kind), ds.values.get(&var)) {
                (Some(kind), Some(value)) if value.fits(kind) => (kind, value.clone()),
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            // Locked art, art on a locked or hidden layer, and text whose object is gone, can't
            // be rewritten: skipped and counted, like the rest. A hidden object is rewritten: a
            // row that hid it shows it again.
            if !d.variable_target(id) {
                skipped += 1;
                continue;
            }
            let done = match (kind, &value) {
                // The Type tool's own whole-range replace, so a dataset's text is styled and
                // laid out exactly as typing it would be.
                (VariableKind::Text, DataValue::Text(text)) => set_plain_text(d, id, text).is_ok(),
                (VariableKind::Visibility, DataValue::Visible(show)) => d.node_mut(id).is_some_and(|n| {
                    n.visible = *show;
                    true
                }),
                _ => false,
            };
            if done {
                applied += 1;
            } else {
                skipped += 1;
            }
        }
        d.variables.active_dataset = Some(name.to_string());
        Ok((applied, skipped))
    })?;
    Ok(json!({"name": name, "applied": applied, "skipped": skipped}))
}

fn dataset_select(s: &mut Session, p: &Value) -> Result<Value> {
    apply_dataset(s, &var_name(p, "dataset.select")?)
}

fn dataset_step(s: &mut Session, cmd: &str, forward: bool) -> Result<Value> {
    let st = s.doc()?;
    let n = st.doc.variables.datasets.len();
    if n == 0 {
        return Err(bad(cmd, "no datasets"));
    }
    let at = st.doc.variables.active_dataset.as_deref().and_then(|a| st.doc.variables.datasets.iter().position(|d| d.name == a));
    let next = match (at, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    let name = st.doc.variables.datasets.get(next).map(|d| d.name.clone()).ok_or_else(|| bad(cmd, "no datasets"))?;
    apply_dataset(s, &name)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("variable.define", "New Variable…", ["Window", "Variables"], None, "{name, kind: text|visibility} define a variable", has_doc, define),
        cmd!(
            "variable.delete",
            "Delete Variable",
            ["Window", "Variables"],
            None,
            "{names?} delete variables with their bindings and values (default: the highlighted one)",
            has_doc,
            delete
        ),
        cmd!(
            "variable.rename",
            "Rename Variable",
            ["Window", "Variables"],
            None,
            "{name, newName} rename a variable, with its bindings and its values in every dataset",
            has_doc,
            rename
        ),
        cmd!(query "variable.list", "List Variables", [], None, "{} → variables with kinds and binding counts", has_doc, list),
        cmd!(
            "variable.makeTextDynamic",
            "Make Text Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a text variable over the selection, named after it and bound to it",
            has_selection,
            |s, _| make_dynamic(s, VariableKind::Text, "Text")
        ),
        cmd!(
            "variable.makeVisibilityDynamic",
            "Make Visibility Dynamic",
            ["Window", "Variables"],
            None,
            "{} define a visibility variable over the selection, named after it and bound to it",
            has_selection,
            |s, _| make_dynamic(s, VariableKind::Visibility, "Visibility")
        ),
        cmd!(
            "variable.bind",
            "Bind Variable",
            ["Window", "Variables"],
            None,
            "{variable, ids?} bind objects (default: the selection) to a variable; text needs type, locked art is refused. An object may hold several variables, and binding it to one it already has changes nothing → {bound, already}",
            has_doc,
            bind
        ),
        cmd!(
            "variable.unbind",
            "Unbind Variable",
            ["Window", "Variables"],
            None,
            "{ids?, variable?} drop bindings (default: the selection): `variable` drops that one, else every variable the object has",
            has_doc,
            unbind
        ),
        cmd!(
            "variable.highlight",
            "Highlight Variable",
            [],
            None,
            "{variable?} highlight one Variables panel row (a click); what the panel's Delete, Options… and Select Bound Object act on. An empty or missing `variable` clears it. Not an undo step → {variable}",
            has_doc,
            highlight
        ),
        cmd!("dataset.new", "New Data Set…", ["Window", "Variables"], None, "{name, values?: {variable: value}} add a dataset", has_doc, dataset_new),
        cmd!(
            "dataset.set",
            "Edit Data Set…",
            ["Window", "Variables"],
            None,
            "{name, values?: {variable: value}} replace a dataset's values; a variable left out loses its value in this row → {values, removed}",
            has_doc,
            dataset_set
        ),
        cmd!("dataset.rename", "Rename Data Set", ["Window", "Variables"], None, "{name, newName} rename a dataset", has_doc, dataset_rename),
        cmd!(
            "dataset.capture",
            "Capture Data Set",
            ["Window", "Variables"],
            None,
            "{name?} record what the bound objects hold right now as a new dataset (Data Set 1…), and make it the active one",
            has_doc,
            dataset_capture
        ),
        cmd!(
            "dataset.update",
            "Update Data Set",
            ["Window", "Variables"],
            None,
            "{} write what the bound objects hold now into the active dataset",
            has_doc,
            dataset_update
        ),
        cmd!(
            "dataset.delete",
            "Delete Data Set",
            ["Window", "Variables"],
            None,
            "{names?} delete datasets (default: the active one)",
            has_doc,
            dataset_delete
        ),
        cmd!(
            query "dataset.list",
            "List Data Sets",
            [],
            None,
            "{} → datasets with their values, the active one, and which rows the art still matches",
            has_doc,
            dataset_list
        ),
        cmd!(
            "dataset.select",
            "Select Data Set",
            ["Window", "Variables"],
            None,
            "{name} apply a dataset in one undo step → {applied, skipped}",
            has_doc,
            dataset_select
        ),
        cmd!("dataset.next", "Next Data Set", ["Window", "Variables"], None, "{} apply the next dataset, wrapping", has_doc, |s, _| dataset_step(
            s,
            "dataset.next",
            true
        )),
        cmd!("dataset.prev", "Previous Data Set", ["Window", "Variables"], None, "{} apply the previous dataset, wrapping", has_doc, |s, _| {
            dataset_step(s, "dataset.prev", false)
        }),
    ]
}
