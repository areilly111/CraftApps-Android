//! EffectCraft's own animation preset library (Effects & Presets ▸ * Animation Presets): the
//! Transitions category, original presets built on our Transition effects and layer opacity.
//!
//! Each preset adds effects (with values and keyframes) and/or keys layer properties, starting
//! at the current time, on every selected layer. The library is data
//! (`presets/transitions.json`), written from scratch; nothing comes from After Effects' `.ffx`
//! presets.

use effectcraft_keyframe::Keyframe;
use effectcraft_project::build::Ids;
use effectcraft_project::{Node, Property};
use effectcraft_time::Tick;
use serde_json::{Map, Value, json};

use super::text_anim::json_value;
use super::{CommandSpec, bad, has_layers, layer_mut, layers_p, str_p};
use crate::{EngineError, Result, Session, cmd, query};

/// The original transition presets (`presets/transitions.json`).
pub const TRANSITIONS_JSON: &str = include_str!("../../presets/transitions.json");

/// Keys per animated property, so a hostile inline preset can't allocate without bound.
const MAX_KEYS: usize = 256;

fn presets() -> Vec<Value> {
    serde_json::from_str::<Value>(TRANSITIONS_JSON).ok().and_then(|v| v.get("presets").and_then(Value::as_array).cloned()).unwrap_or_default()
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// (id, name, category) of every built-in animation preset, in library order.
pub fn preset_names() -> Vec<(String, String, String)> {
    presets()
        .iter()
        .filter_map(|p| Some((text(p, "id")?.to_string(), text(p, "name")?.to_string(), text(p, "category").unwrap_or("Transitions").to_string())))
        .collect()
}

fn find_preset(want: &str) -> Option<Value> {
    presets().into_iter().find(|x| text(x, "id") == Some(want) || text(x, "name").is_some_and(|n| n.eq_ignore_ascii_case(want)))
}

/// Set `pr` to `v` (when given) and replace its keys with `keys` (`[[seconds, value], …]`,
/// seconds from `at`). Returns false when nothing in the request fits the property.
fn set_prop(pr: &mut Property, v: Option<&Value>, keys: Option<&Value>, at: Tick, ease: bool) -> bool {
    let mut ok = true;
    if let Some(v) = v {
        match json_value(v, &pr.value) {
            Some(val) => pr.value = val,
            None => ok = false,
        }
    }
    let Some(list) = keys else { return ok };
    let Some(list) = list.as_array() else { return false };
    let mut ks: Vec<Keyframe> = Vec::new();
    for k in list.iter().take(MAX_KEYS) {
        let t = k.get(0).and_then(Value::as_f64).filter(|t| t.is_finite() && t.abs() <= 86_400.0);
        let (Some(t), Some(val)) = (t, k.get(1).and_then(|x| json_value(x, &pr.value))) else { return false };
        let mut key = Keyframe::new(at + Tick::from_seconds_f64(t), val);
        if pr.hold_only {
            key = key.hold();
        } else if ease {
            key = key.eased();
        }
        ks.push(key);
    }
    ks.sort_by_key(|k| k.time);
    ks.dedup_by_key(|k| k.time);
    let Some(first) = ks.first() else { return false };
    pr.value = first.value.clone();
    pr.keys = ks;
    ok
}

/// Apply `set` / `keys` maps to the properties under `root`; returns the paths that don't exist
/// or don't take the given values.
fn apply_maps(root: &mut effectcraft_project::PropGroup, spec: &Value, at: Tick) -> Vec<String> {
    let ease = spec.get("ease").and_then(Value::as_bool).unwrap_or(false);
    let empty = Map::new();
    let set = spec.get("set").and_then(Value::as_object).unwrap_or(&empty);
    let keys = spec.get("keys").and_then(Value::as_object).unwrap_or(&empty);
    let mut skipped = Vec::new();
    for (path, v) in set {
        if !root.prop_mut(path).is_some_and(|pr| set_prop(pr, Some(v), None, at, false)) {
            skipped.push(path.clone());
        }
    }
    for (path, list) in keys {
        if !root.prop_mut(path).is_some_and(|pr| !pr.static_only && set_prop(pr, None, Some(list), at, ease)) {
            skipped.push(path.clone());
        }
    }
    skipped
}

/// `anim.applyBuiltinPreset`: add a built-in preset's effects and keyframes to the layers.
fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "anim.applyBuiltinPreset";
    let want = str_p(p, "preset").ok_or_else(|| bad(C, "`preset` id or name required"))?;
    let preset = find_preset(want).ok_or_else(|| bad(C, format!("no animation preset `{want}`")))?;
    let name = text(&preset, "name").unwrap_or("Preset").to_string();
    let effects: Vec<(&'static effectcraft_effects::EffectSpec, Value)> = preset
        .get("effects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            let id = text(e, "effect").unwrap_or_default();
            effectcraft_effects::lookup(id).map(|spec| (spec, e.clone())).ok_or_else(|| bad(C, format!("preset `{name}` uses unknown effect `{id}`")))
        })
        .collect::<Result<_>>()?;
    let layer_spec = preset.get("layer").cloned();
    let (cid, ids) = layers_p(s, p)?;
    if ids.is_empty() {
        return Err(bad(C, "select a layer"));
    }
    let sizes: Vec<_> = {
        let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
        ids.iter().filter_map(|id| comp.layer(*id)).map(|l| (l.id, effectcraft_render::effect_bounds(&s.project, comp, l).0)).collect()
    };
    let now = s.time();
    let (added, skipped) = s.edit(&format!("Apply Animation Preset {name}"), None, |proj, _| {
        let mut added: Vec<u64> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (lid, size) in &sizes {
            let mut next = proj.next_id;
            let l = layer_mut(proj, cid, *lid)?;
            let at = l.layer_time(now);
            if !effects.is_empty() {
                let fx = l.props.sub_mut("effects").ok_or_else(|| bad(C, "this layer type has no effects"))?;
                for (spec, ej) in &effects {
                    let same = fx.groups().filter(|g| g.match_id == spec.id).count();
                    let inst = if same == 0 { spec.name.to_string() } else { format!("{} {}", spec.name, same.saturating_add(1)) };
                    let mut g = effectcraft_effects::instantiate(spec, &mut Ids(&mut next), &inst, *size);
                    skipped.extend(apply_maps(&mut g, ej, at).into_iter().map(|path| format!("{}/{path}", spec.id)));
                    added.push(g.uid);
                    fx.children.push(Node::Group(g));
                }
            }
            if let Some(ls) = &layer_spec {
                skipped.extend(apply_maps(&mut l.props, ls, at));
            }
            proj.next_id = next;
        }
        Ok((added, skipped))
    })?;
    Ok(json!({"preset": text(&preset, "id"), "effects": added, "skipped": skipped}))
}

/// `anim.builtinPresets`: the library, optionally one category.
fn list(_: &mut Session, p: &Value) -> Result<Value> {
    let cat = str_p(p, "category");
    Ok(Value::Array(
        presets()
            .iter()
            .filter(|x| cat.is_none_or(|c| text(x, "category").is_some_and(|k| k.eq_ignore_ascii_case(c))))
            .map(|x| {
                json!({
                    "id": x.get("id"),
                    "name": x.get("name"),
                    "category": x.get("category"),
                    "direction": x.get("direction"),
                    "description": x.get("description"),
                })
            })
            .collect(),
    ))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("anim.applyBuiltinPreset", "Apply Built-in Animation Preset", [], None, "{preset: id|name (see anim.builtinPresets), layers?}", has_layers, apply),
        query!("anim.builtinPresets", "Built-in Animation Presets", "{category?}", list),
    ]
}
