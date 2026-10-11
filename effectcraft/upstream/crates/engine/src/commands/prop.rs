//! Properties, keyframes and expressions (Animation menu + timeline/Effect Controls gestures).

use effectcraft_keyframe::{Ease, Interp, Keyframe, easy_ease, key_at, set_key};
use effectcraft_project::{GroupKind, ItemId, Layer, LayerId, Property, Uid};
use effectcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, comp_id, f_p, has_comp, has_keys, has_layers, layer_mut, layer_p, layers_p, merge_p, selected_leaf_props, str_p};
use crate::{EngineError, KeyRef, Result, Session, VertexRef, cmd, query};

/// Resolve `{layer, path}` (or `{layer, prop: uid}`) to (comp, layer, prop uid).
pub(crate) fn prop_ref(s: &Session, p: &Value, cmd: &str) -> Result<(ItemId, LayerId, Uid)> {
    let (cid, lid) = layer_p(s, p, cmd)?;
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    if let Some(u) = p.get("prop").and_then(Value::as_u64) {
        return layer.props.find(u).map(|pr| (cid, lid, pr.uid)).ok_or_else(|| {
            if layer.props.find_group(u).is_some() {
                bad(cmd, format!("@{u} is a property group; use layer.tree (MCP: get_layer) to inspect its children, then select a leaf property"))
            } else {
                bad(cmd, format!("no property @{u}"))
            }
        });
    }
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "missing `path` (e.g. transform/position) or `prop` uid"))?;
    let pr = layer.props.prop(path).ok_or_else(|| {
        let group = layer.props.group(path).or_else(|| path.trim().strip_prefix('@')?.parse().ok().and_then(|uid| layer.props.find_group(uid)));
        if group.is_some() {
            bad(cmd, format!("`{path}` is a property group; use layer.tree (MCP: get_layer) to inspect its children, then select a leaf property"))
        } else {
            bad(cmd, format!("no property `{path}`"))
        }
    })?;
    Ok((cid, lid, pr.uid))
}

fn with_prop<T>(
    s: &mut Session,
    label: &str,
    merge: Option<&str>,
    cid: ItemId,
    lid: LayerId,
    uid: Uid,
    f: impl FnOnce(&mut Property, Tick) -> Result<T>,
) -> Result<T> {
    let t = s.time_of(cid);
    let linear = s.prefs.general.default_spatial_linear;
    s.edit(label, merge, |proj, _| {
        let l = layer_mut(proj, cid, lid)?;
        let lt = l.layer_time(t);
        let pr = l.props.find_mut(uid).ok_or_else(|| bad("prop", "property vanished"))?;
        let before: Vec<Tick> = pr.keys.iter().map(|k| k.time).collect();
        let r = f(pr, lt)?;
        // Settings ▸ General ▸ Default Spatial Interpolation to Linear: new keys of spatial
        // properties get straight motion paths instead of Auto Bezier.
        if linear && pr.spatial {
            for k in pr.keys.iter_mut().filter(|k| !before.contains(&k.time)) {
                k.spatial_auto = false;
                k.spatial_in = [0.0; 3];
                k.spatial_out = [0.0; 3];
            }
        }
        Ok(r)
    })
}

/// Explicit command key times default to layer time; readers and the CTI use comp time.
#[derive(Clone, Copy)]
pub(crate) enum KeyTimeBase {
    Layer,
    Comp,
}

impl KeyTimeBase {
    pub(crate) fn from_params(p: &Value, cmd: &str) -> Result<Self> {
        match p.get("timeBase") {
            None => Ok(Self::Layer),
            Some(Value::String(b)) if b == "layer" => Ok(Self::Layer),
            Some(Value::String(b)) if b == "comp" => Ok(Self::Comp),
            Some(_) => Err(bad(cmd, "`timeBase` must be layer or comp")),
        }
    }
}

/// Convert a supplied time to stored layer ticks, snapping writes in the comp's frame grid.
pub(crate) fn key_time(layer: &Layer, frame_rate: FrameRate, p: &Value, param: &str, base: KeyTimeBase, cmd: &str, snap: bool) -> Result<Option<Tick>> {
    let Some(value) = p.get(param) else { return Ok(None) };
    let seconds = value
        .as_f64()
        .filter(|t| t.is_finite() && *t >= Tick::MIN.seconds() && *t <= Tick::MAX.seconds())
        .ok_or_else(|| bad(cmd, format!("`{param}` must be finite seconds within the supported time range")))?;
    if !layer.stretch.is_finite() || layer.stretch == 0.0 {
        return Err(bad(cmd, "the layer has an invalid time stretch"));
    }
    let t = Tick::from_seconds_f64(seconds);
    let conversion_error = || bad(cmd, format!("`{param}` converts outside the supported time range"));
    let in_range = |ticks: i64| (Tick::MIN.0..=Tick::MAX.0).contains(&ticks);
    // Validate before Layer's saturating conversion: otherwise a huge stretch can silently
    // clamp an intended key to a different comp time while still producing a valid Tick.
    let scaled_tick = |ticks: f64| -> Result<Tick> {
        if !ticks.is_finite() || ticks < Tick::MIN.0 as f64 || ticks > Tick::MAX.0 as f64 {
            return Err(conversion_error());
        }
        let ticks = ticks as i64;
        if !in_range(ticks) {
            return Err(conversion_error());
        }
        Ok(Tick(ticks))
    };
    let normal = (layer.stretch - 100.0).abs() < 1e-9;
    let ct = match base {
        KeyTimeBase::Layer => {
            let offset = if normal { t } else { scaled_tick(t.0 as f64 * layer.stretch / 100.0)? };
            Tick(layer.start_time.0.checked_add(offset.0).filter(|t| in_range(*t)).ok_or_else(conversion_error)?)
        }
        KeyTimeBase::Comp => t,
    };
    let ct = if snap { frame_rate.snap_nearest(ct) } else { ct };
    if !in_range(ct.0) {
        return Err(conversion_error());
    }
    let offset = ct.0.checked_sub(layer.start_time.0).filter(|t| in_range(*t)).ok_or_else(conversion_error)?;
    let lt = if normal { Tick(offset) } else { scaled_tick(offset as f64 * 100.0 / layer.stretch)? };
    // Unsnapped legacy layer times remain exact, rather than making a float round-trip.
    Ok(Some(if !snap && matches!(base, KeyTimeBase::Layer) { t } else { lt }))
}

pub(crate) fn key_timing(layer: &Layer, t: Tick) -> Value {
    let ct = layer.comp_time(t);
    let mut out = json!({"time": t.seconds(), "layerTime": t.seconds(), "compTime": ct.seconds()});
    if !layer.is_active_at(ct) {
        out["warning"] = json!(format!(
            "Keyframe at comp time {} s is outside the layer's active range [{} s, {} s).",
            ct.seconds(),
            layer.in_point.seconds(),
            layer.out_point.seconds()
        ));
    }
    out
}

/// Omitted bases keep the original JSON reply; explicit bases opt in to timing metadata.
fn key_reply(p: &Value, mut timing: Value, field: &str, result: Value) -> Value {
    if p.get("timeBase").is_some() {
        timing[field] = result;
        timing
    } else {
        result
    }
}

fn snapped_key_time(s: &Session, cid: ItemId, lid: LayerId, p: &Value, cmd: &str) -> Result<Option<Tick>> {
    let base = KeyTimeBase::from_params(p, cmd)?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?;
    let layer = comp.layer(lid).ok_or(EngineError::NoComp)?;
    key_time(layer, comp.frame_rate, p, "time", base, cmd, true)
}

fn property_reply(p: &Value, layer: &Layer, pr: &Property, time: Tick, value: Value) -> Value {
    if p.get("timeBase").is_some() {
        let mut timing = key_timing(layer, time);
        timing["animated"] = json!(pr.is_animated());
        key_reply(p, timing, "value", value)
    } else {
        value
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.set")?;
    // Position with Separate Dimensions: write the X/Y/Z Position properties instead.
    if let Some(l) = s.project.comp(cid).and_then(|c| c.layer(lid))
        && let Some(tr) = l.transform()
        && tr.get("position").is_some_and(|pr| pr.uid == uid)
        && tr.get("positionX").is_some()
        && let Some(Value::Array(a)) = p.get("value")
    {
        let mut out = vec![];
        let three = l.is_3d();
        for (d, m) in ["positionX", "positionY", "positionZ"].iter().enumerate() {
            if d == 2 && (!three || a.len() < 3) {
                continue;
            }
            if let Some(v) = a.get(d) {
                let mut q = p.clone();
                q["path"] = json!(format!("transform/{m}"));
                q["layer"] = json!(lid.0);
                if let Some(o) = q.as_object_mut() {
                    o.remove("prop");
                }
                q["value"] = v.clone();
                out.push(set(s, &q)?);
            }
        }
        return Ok(json!(out));
    }
    let v = p.get("value").ok_or_else(|| bad("prop.set", "missing `value`"))?.clone();
    let at = snapped_key_time(s, cid, lid, p, "prop.set")?;
    let out = with_prop(s, "Change Property", merge_p(p), cid, lid, uid, |pr, lt| {
        let cur = pr.value_at(lt);
        let mut nv = cur.coerce_json(&v).ok_or_else(|| bad("prop.set", format!("can't use {v} for a {} property", cur.kind_name())))?;
        // Clamp to slider ranges.
        if let effectcraft_project::ParamUi::Slider { min, max, .. } = pr.ui
            && let effectcraft_keyframe::Value::Scalar(x) = nv
        {
            nv = effectcraft_keyframe::Value::Scalar(x.clamp(min, max));
        }
        pr.set_value_at(at.unwrap_or(lt), nv.clone());
        Ok(nv.to_json())
    })?;
    if p.get("timeBase").is_none() {
        return Ok(out);
    }
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    let pr = layer.props.find(uid).ok_or_else(|| bad("prop.set", "property vanished"))?;
    Ok(property_reply(p, layer, pr, at.unwrap_or_else(|| layer.layer_time(s.time_of(cid))), out))
}

/// Read one property: value at `time` (comp seconds, default the CTI), keyframes and expression.
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.get")?;
    let l = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    let pr = l.props.find(uid).ok_or_else(|| bad("prop.get", "property vanished"))?;
    let t = f_p(p, "time").map(Tick::from_seconds_f64).unwrap_or_else(|| s.time_of(cid));
    let keys: Vec<Value> = pr
        .keys
        .iter()
        .map(|k| {
            let mut info = key_timing(l, k.time);
            info["value"] = k.value.to_json();
            info["in"] = json!(k.in_interp.label());
            info["out"] = json!(k.out_interp.label());
            info
        })
        .collect();
    let raw = pr.value_at(l.layer_time(t));
    let mut out = json!({
        "layer": lid.0, "uid": pr.uid, "match": pr.match_id, "name": pr.name, "type": pr.value.kind_name(),
        "time": t.seconds(), "value": raw.to_json(), "animated": !pr.keys.is_empty(),
        "keys": keys, "expression": pr.expr.as_ref().map(|e| e.text.clone()),
    });
    // With an expression, `value` is the keyframed (pre-expression) value; also report what the
    // expression makes of it (what renders), or why it fails. A disabled one renders `value`; one
    // disabled by its syntax error (prop.setExpression) reports that error.
    if let Some(e) = pr.expr.as_ref().filter(|e| !e.enabled && !e.text.trim().is_empty()) {
        out["evaluated"] = raw.to_json();
        if let Some(err) = s.expr_check.and_then(|check| check(&e.text).err()) {
            out["expressionError"] = json!(err);
        }
    } else if pr.has_expression()
        && let (Some(h), Some(comp)) = (s.expr.as_deref(), s.project.comp(cid))
    {
        let mut ctx = crate::render::EvalCtx::new(&s.project, cid, comp, t);
        ctx.expr = Some(h);
        match h.eval(&ctx, l, pr, &raw) {
            Ok(v) => out["evaluated"] = v.to_json(),
            Err(e) => {
                out["evaluated"] = raw.to_json();
                out["expressionError"] = json!(e);
            }
        }
    }
    Ok(out)
}

fn toggle_anim(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.toggleAnimation")?;
    let on = b_p(p, "value");
    with_prop(s, "Toggle Animation", merge_p(p), cid, lid, uid, |pr, lt| {
        if pr.static_only {
            return Err(bad("prop.toggleAnimation", "this property can't be animated"));
        }
        let target = on.unwrap_or(pr.keys.is_empty());
        pr.set_animated(target, lt);
        Ok(json!(target))
    })
}

fn add_key(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.addKey")?;
    let at = snapped_key_time(s, cid, lid, p, "prop.addKey")?;
    let value = p.get("value").cloned();
    let count = with_prop(s, "Add Keyframe", None, cid, lid, uid, |pr, lt| {
        let t = at.unwrap_or(lt);
        let cur = pr.value_at(t);
        let v = match &value {
            Some(j) => cur.coerce_json(j).ok_or_else(|| bad("prop.addKey", "bad value"))?,
            None => cur,
        };
        let mut k = Keyframe::new(t, v);
        if pr.hold_only {
            k = k.hold();
        }
        set_key(&mut pr.keys, k);
        Ok(json!(pr.keys.len()))
    })?;
    if p.get("timeBase").is_none() {
        return Ok(count);
    }
    let layer = s.project.comp(cid).and_then(|c| c.layer(lid)).ok_or(EngineError::NoComp)?;
    Ok(key_reply(p, key_timing(layer, at.unwrap_or_else(|| layer.layer_time(s.time_of(cid)))), "count", count))
}

fn toggle_key(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.toggleKey")?;
    with_prop(s, "Add/Remove Keyframe", None, cid, lid, uid, |pr, lt| Ok(json!(toggle_key_at(pr, lt))))
}

/// Add a keyframe at layer time `lt` (true), or remove the one there (false).
fn toggle_key_at(pr: &mut Property, lt: Tick) -> bool {
    if let Some(i) = key_at(&pr.keys, lt) {
        if pr.keys.len() == 1 {
            pr.value = pr.keys[0].value.clone();
        }
        pr.keys.remove(i);
        false
    } else {
        let mut k = Keyframe::new(lt, pr.value_at(lt));
        if pr.hold_only {
            k = k.hold();
        }
        set_key(&mut pr.keys, k);
        true
    }
}

/// Alt+Shift+A / P / S / R / T: add or remove a keyframe at the current time on that Transform
/// property of every selected layer, in one undo step. Position covers separated X / Y / Z;
/// Rotation on a 3D layer covers Orientation and X / Y / Z Rotation.
fn toggle_transform_key(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "keys.toggleTransform";
    let which = str_p(p, "prop").ok_or_else(|| bad(C, "missing `prop` (anchor | position | scale | rotation | opacity)"))?.to_string();
    let (cid, layers) = layers_p(s, p)?;
    let t = s.time_of(cid);
    let mut added = vec![];
    s.edit("Add/Remove Keyframe", None, |proj, _| {
        for lid in &layers {
            let l = layer_mut(proj, cid, *lid)?;
            let lt = l.layer_time(t);
            let three = l.is_3d();
            let Some(tr) = l.props.sub_mut("transform") else { continue };
            let ids: Vec<&str> = match which.as_str() {
                "anchor" => vec!["anchor"],
                "position" if tr.get("positionX").is_some() => vec!["positionX", "positionY", "positionZ"],
                "position" => vec!["position"],
                "scale" => vec!["scale"],
                "rotation" if three => vec!["orientation", "rotationX", "rotationY", "rotation"],
                "rotation" => vec!["rotation"],
                "opacity" => vec!["opacity"],
                other => return Err(bad(C, format!("unknown prop `{other}` (anchor | position | scale | rotation | opacity)"))),
            };
            for id in ids.into_iter().filter(|id| three || *id != "positionZ") {
                if let Some(pr) = tr.get_mut(id) {
                    added.push(json!({"layer": lid.0, "prop": id, "key": toggle_key_at(pr, lt)}));
                }
            }
        }
        Ok(())
    })?;
    Ok(Value::Array(added))
}

/// Add, toggle or set one property's expression (`def` is the starting text of a new one).
fn apply_expr(pr: &mut Property, text: &Option<String>, enabled: Option<bool>, def: &str) {
    match (text, enabled) {
        (Some(t), _) if t.trim().is_empty() => pr.expr = None,
        (Some(t), e) => pr.expr = Some(effectcraft_project::Expression { text: t.clone(), enabled: e.unwrap_or(true) }),
        (None, Some(e)) => {
            if let Some(x) = &mut pr.expr {
                x.enabled = e;
            } else if e {
                pr.expr = Some(effectcraft_project::Expression { text: def.to_string(), enabled: true });
            }
        }
        (None, None) => {
            pr.expr = if pr.expr.is_some() { None } else { Some(effectcraft_project::Expression { text: def.to_string(), enabled: true }) };
        }
    }
}

/// `{layer?, path|prop, ...}` for one property, or (Animation > Add Expression) no `path`/`prop`
/// for the properties selected in the timeline, all in one undo step.
fn set_expr(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "prop.setExpression";
    let explicit = p.get("path").is_some() || p.get("prop").is_some();
    let targets: Vec<(ItemId, LayerId, Uid)> = if explicit {
        vec![prop_ref(s, p, C)?]
    } else {
        let cid = comp_id(s, p)?;
        selected_leaf_props(s).into_iter().map(|(l, u)| (cid, l, u)).collect()
    };
    if targets.is_empty() {
        return Err(bad(C, "select a property in the timeline (or give `layer` and `path`)"));
    }
    let text = p.get("expression").and_then(Value::as_str).map(str::to_string);
    let mut enabled = b_p(p, "enabled");
    // A syntax error keeps the text but disables the expression (AE shows the warning bar).
    let error = match (&text, s.expr_check) {
        (Some(t), Some(check)) if !t.trim().is_empty() => check(t).err(),
        _ => None,
    };
    if error.is_some() {
        enabled = Some(false);
    }
    // Alt-click on a stopwatch starts with the property's own reference (`transform.opacity`).
    let defs: Vec<String> = targets
        .iter()
        .map(|(cid, lid, uid)| {
            s.project.comp(*cid).and_then(|c| c.layer(*lid).and_then(|l| super::link::reference(c, l, l, *uid, true))).unwrap_or_else(|| "value".into())
        })
        .collect();
    let results = s.edit("Expression", merge_p(p), |proj, _| {
        let mut out = vec![];
        for ((cid, lid, uid), def) in targets.iter().zip(&defs) {
            let pr = layer_mut(proj, *cid, *lid)?.props.find_mut(*uid).ok_or_else(|| bad(C, "property vanished"))?;
            apply_expr(pr, &text, enabled, def);
            out.push(json!(pr.expr.as_ref().map(|e| e.text.clone())));
        }
        Ok(out)
    })?;
    let r = if explicit { results.into_iter().next().unwrap_or(Value::Null) } else { Value::Array(results) };
    if let Some(e) = error {
        s.toast(format!("Expression disabled: {e}"));
        return Ok(json!({"expression": r, "error": e}));
    }
    Ok(r)
}

/// A property's default value, when it has a known one: an effect parameter's, or a Transform
/// property's (not a camera's or light's).
fn default_value(s: &Session, cid: ItemId, lid: LayerId, uid: Uid) -> Option<effectcraft_keyframe::Value> {
    let comp = s.project.comp(cid)?;
    let layer = comp.layer(lid)?;
    let pr = layer.props.find(uid)?;
    let effect = layer.props.node_chain(uid)?.into_iter().find_map(|n| match n {
        effectcraft_project::Node::Group(g) => match &g.kind {
            GroupKind::Effect { effect } => Some(effect.clone()),
            _ => None,
        },
        _ => None,
    });
    if let Some(effect) = effect {
        let ps = effectcraft_effects::find(&effect)?.params.iter().find(|ps| ps.id == pr.match_id)?;
        return Some(effectcraft_effects::default_value(ps, effectcraft_render::effect_bounds(&s.project, comp, layer).0));
    }
    if matches!(layer.source, effectcraft_project::LayerSource::Camera | effectcraft_project::LayerSource::Light { .. })
        || !layer.transform()?.props().any(|t| t.uid == uid)
    {
        return None;
    }
    let (w, h) = effectcraft_render::source_size(&s.project, layer);
    super::layer::transform_default(&pr.match_id, (w as f64, h as f64), (comp.width as f64, comp.height as f64))
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.reset")?;
    let def = p.get("default").cloned();
    let own = default_value(s, cid, lid, uid);
    with_prop(s, "Reset Property", None, cid, lid, uid, |pr, _| {
        pr.keys.clear();
        pr.expr = None;
        if let Some(d) = def.as_ref().and_then(|d| pr.value.coerce_json(d)).or(own) {
            pr.value = d;
        }
        Ok(Value::Null)
    })
}

fn select_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, lid) = layer_p(s, p, "prop.select")?;
    let uid = match p.get("prop").and_then(Value::as_u64) {
        Some(u) => u,
        None => prop_ref(s, p, "prop.select")?.2,
    };
    let add = b_p(p, "add").unwrap_or(false);
    if !add {
        s.state.selected_props.clear();
        s.state.selected_vertices.clear();
    }
    if !s.state.selected_layers.contains(&lid) {
        s.state.selected_layers = vec![lid];
    }
    s.state.selected_props.push((lid, uid));
    // Selecting a mask (or its Mask Path, or a shape's Path) selects all its points, so the
    // viewer drags the whole mask (as in AE, #203).
    let t = s.time();
    if let Some((mask, n)) = s.active_comp().and_then(|c| c.layer(lid)).and_then(|l| path_points(l, uid, l.layer_time(t))) {
        s.state.selected_vertices.retain(|v| !(v.layer == lid && v.mask == mask));
        s.state.selected_vertices.extend((0..n).map(|index| VertexRef { layer: lid, mask, index }));
    }
    // Selecting a property selects all its keys (as in AE).
    if b_p(p, "selectKeys").unwrap_or(true)
        && let Some(pr) = s.active_comp().and_then(|c| c.layer(lid)).and_then(|l| l.props.find(uid))
    {
        let keys: Vec<KeyRef> = pr.keys.iter().map(|k| KeyRef { layer: lid, prop: uid, time: k.time }).collect();
        if !add {
            s.state.selected_keys.clear();
        }
        s.state.selected_keys.extend(keys);
    }
    Ok(Value::Null)
}

/// Deselect only the named property, preserving unrelated property, layer, key and
/// mask-vertex selections. Selected keys also imply selectedProperties in the script API.
fn deselect_prop(s: &mut Session, params: &Value) -> Result<Value> {
    // Groups (masks, effects) are selected by uid too, so resolve the uid as prop.select does.
    let (_, lid) = layer_p(s, params, "prop.deselect")?;
    let uid = match params.get("prop").and_then(Value::as_u64) {
        Some(u) => u,
        None => prop_ref(s, params, "prop.deselect")?.2,
    };
    let mask = s.active_comp().and_then(|c| c.layer(lid)).and_then(|l| path_points(l, uid, l.layer_time(s.time()))).map(|(mask, _)| mask);
    s.state.selected_props.retain(|(layer, prop)| *layer != lid || *prop != uid);
    s.state.selected_keys.retain(|key| key.layer != lid || key.prop != uid);
    if let Some(mask) = mask {
        s.state.selected_vertices.retain(|v| v.layer != lid || v.mask != mask);
    }
    Ok(Value::Null)
}

/// The mask or shape Path item that `uid` (the group, or its Path property) is, with its number
/// of points at layer time `lt`.
fn path_points(l: &Layer, uid: Uid, lt: Tick) -> Option<(Uid, usize)> {
    let owner = match l.props.find_group(uid) {
        Some(g) => g,
        None => l.props.parent_of(uid).filter(|g| g.get("path").is_some_and(|p| p.uid == uid))?,
    };
    let in_contents = owner.match_id == "path" && l.props.sub("contents").is_some_and(|c| c.find_group(owner.uid).is_some());
    if !matches!(owner.kind, GroupKind::Mask { .. }) && !in_contents {
        return None;
    }
    match owner.get("path")?.value_at(lt) {
        effectcraft_keyframe::Value::Path(p) => Some((owner.uid, p.vertices.len())),
        _ => None,
    }
}

// ---------------------------------------------------------------- keyframes

fn select_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let base = KeyTimeBase::from_params(p, "keys.select")?;
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let mut sel = vec![];
    if let Some(Value::Array(keys)) = p.get("keys") {
        for k in keys {
            // `{layer, prop: uid | path, time}` (or `path`); anything that matches nothing is an
            // error rather than a silently empty selection.
            let lv = k.get("layer").ok_or_else(|| bad("keys.select", format!("{k}: missing `layer`")))?;
            let l = super::resolve_layer(comp, lv).ok_or_else(|| bad("keys.select", format!("no layer {lv}")))?;
            let ly = comp.layer(l).ok_or(EngineError::NoComp)?;
            let pr = match (k.get("prop"), k.get("path").and_then(Value::as_str)) {
                (Some(Value::Number(n)), _) => n.as_u64().and_then(|u| ly.props.find(u)),
                (Some(Value::String(path)), _) => ly.props.prop(path),
                (None, Some(path)) => ly.props.prop(path),
                _ => return Err(bad("keys.select", format!("{k}: give `prop` (uid) or `path`"))),
            }
            .ok_or_else(|| bad("keys.select", format!("{k}: no such property on layer {lv}")))?;
            let t = key_time(ly, comp.frame_rate, k, "time", base, "keys.select", false)?.ok_or_else(|| bad("keys.select", format!("{k}: missing `time`")))?;
            // Snap to the stored key time.
            let kt = pr
                .keys
                .iter()
                .map(|k| k.time)
                .min_by_key(|kt| (kt.0 - t.0).abs())
                .ok_or_else(|| bad("keys.select", format!("`{}` has no keyframes", pr.name)))?;
            sel.push(KeyRef { layer: l, prop: pr.uid, time: kt });
        }
    }
    // Keys of locked layers can't be selected (so nothing edits them).
    sel.retain(|k| comp.layer(k.layer).is_some_and(|l| !l.switches.locked));
    if b_p(p, "toggle").unwrap_or(false) {
        // Shift+click: in and out of the selection.
        for k in sel {
            match s.state.selected_keys.iter().position(|x| *x == k) {
                Some(i) => {
                    s.state.selected_keys.remove(i);
                }
                None => s.state.selected_keys.push(k),
            }
        }
    } else if b_p(p, "add").unwrap_or(false) {
        for k in sel {
            if !s.state.selected_keys.contains(&k) {
                s.state.selected_keys.push(k);
            }
        }
    } else {
        s.state.selected_keys = sel;
    }
    // From the Timeline, selected keys select their properties and layers too, as in After
    // Effects, so the Graph Editor shows them (#252).
    if b_p(p, "selectProperties").unwrap_or(false) && !s.state.selected_keys.is_empty() {
        let extend = b_p(p, "add").unwrap_or(false) || b_p(p, "toggle").unwrap_or(false);
        let st = &mut s.state;
        if !extend {
            st.selected_props.clear();
            st.selected_layers.clear();
        }
        for k in &st.selected_keys {
            if !st.selected_props.contains(&(k.layer, k.prop)) {
                st.selected_props.push((k.layer, k.prop));
            }
            if !st.selected_layers.contains(&k.layer) {
                st.selected_layers.push(k.layer);
            }
        }
    }
    if p.get("timeBase").is_none() {
        return Ok(json!(s.state.selected_keys.len()));
    }
    let comp = s.active_comp().ok_or(EngineError::NoComp)?;
    let keys: Vec<Value> = s
        .state
        .selected_keys
        .iter()
        .filter_map(|k| {
            let layer = comp.layer(k.layer)?;
            let mut timing = key_timing(layer, k.time);
            timing["layer"] = json!(k.layer.0);
            timing["prop"] = json!(k.prop);
            Some(timing)
        })
        .collect();
    let warnings: Vec<&str> = keys.iter().filter_map(|k| k.get("warning").and_then(Value::as_str)).collect();
    let mut out = json!({"count": s.state.selected_keys.len(), "keys": keys});
    if !warnings.is_empty() {
        out["warning"] = json!(warnings.join(" "));
    }
    Ok(out)
}

/// Apply `f` to every selected key; keeps the selection pointing at moved keys.
pub(crate) fn edit_keys(s: &mut Session, label: &str, merge: Option<&str>, f: impl Fn(&mut Vec<Keyframe>, usize, &mut Tick) -> bool) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let sel = s.state.selected_keys.clone();
    s.edit(label, merge, |proj, st| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let mut remapped: std::collections::HashMap<KeyRef, KeyRef> = Default::default();
        let mut groups: std::collections::BTreeMap<(LayerId, Uid), Vec<Tick>> = Default::default();
        for k in &sel {
            groups.entry((k.layer, k.prop)).or_default().push(k.time);
        }
        for ((lid, uid), times) in groups {
            let Some(pr) = comp.layer_mut(lid).and_then(|l| l.props.find_mut(uid)) else { continue };
            // Process each stored key once, in an order that avoids collisions when moving.
            let mut idx: Vec<(usize, Tick)> = times.iter().filter_map(|t| key_at(&pr.keys, *t).map(|i| (i, *t))).collect();
            idx.sort_by_key(|x| x.0);
            idx.dedup_by_key(|x| x.0);
            let mut moved: Vec<(Tick, Keyframe)> = vec![];
            // Target time -> surviving source time. A moved key replaces an existing key at
            // its destination, including its selection identity.
            let mut targets: std::collections::HashMap<Tick, Tick> = Default::default();
            for (i, old) in idx.iter().rev() {
                let Some(key) = pr.keys.get(*i) else { continue };
                let mut t = key.time;
                if f(&mut pr.keys, *i, &mut t) {
                    if *i < pr.keys.len() {
                        let mut k = pr.keys.remove(*i);
                        k.time = t;
                        moved.push((*old, k));
                    }
                } else if key_at(&pr.keys, *old).is_some() {
                    targets.insert(*old, *old);
                }
            }
            for (old, k) in moved {
                targets.insert(k.time, old);
                match pr.keys.binary_search_by(|x| x.time.cmp(&k.time)) {
                    Ok(j) => pr.keys[j] = k,
                    Err(j) => pr.keys.insert(j, k),
                }
            }
            // Resolve exact stored indices before roving changes their times. Deleted or
            // stale references cannot select a neighbouring key.
            let tracked: Vec<(Tick, usize)> = targets.into_iter().filter_map(|(at, old)| key_at(&pr.keys, at).map(|i| (old, i))).collect();
            effectcraft_keyframe::retime_roving(&mut pr.keys, pr.spatial);
            for (old, i) in tracked {
                if let Some(key) = pr.keys.get(i) {
                    remapped.insert(KeyRef { layer: lid, prop: uid, time: old }, KeyRef { layer: lid, prop: uid, time: key.time });
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        st.selected_keys = sel.iter().filter_map(|k| remapped.get(k).copied()).filter(|k| seen.insert(*k)).collect();
        Ok(json!(st.selected_keys.len()))
    })
}

/// Move the selected keys by `d` comp seconds, snapped to the comp's frames in comp time (keys
/// of time-stretched layers move with the pointer, not by the layer-time distance). Keys of
/// locked layers stay. A key landing on another key of its property replaces it.
pub(crate) fn shift_keys(s: &mut Session, label: &str, merge: Option<&str>, d: Tick) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    // A drag (steps with one merge key) starts again from the keys before it each step.
    let gesture = merge.is_some_and(|m| s.history.merge_key.as_deref() == Some(m)) && s.key_move.as_ref().map(|g| g.merge.as_str()) == merge;
    let (from, total, base) = match (&s.key_move, s.history.undo.last()) {
        (Some(g), Some((_, base))) if gesture => (g.from.clone(), g.total + d, Some(base.clone())),
        _ => (s.state.selected_keys.clone(), d, None),
    };
    s.key_move = merge.map(|m| crate::KeyMove { merge: m.to_string(), from: from.clone(), total });
    s.edit(label, merge, |proj, st| {
        if let Some(b) = base {
            *proj = (*b).clone();
        }
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        let fr = comp.frame_rate;
        let mut groups: std::collections::BTreeMap<(LayerId, Uid), Vec<Tick>> = Default::default();
        for k in &from {
            groups.entry((k.layer, k.prop)).or_default().push(k.time);
        }
        let mut sel = vec![];
        for ((lid, uid), times) in groups {
            let Some(l) = comp.layer_mut(lid) else { continue };
            let to = |t: Tick| l.layer_time(fr.snap_nearest(l.comp_time(t) + total));
            let moves: Vec<(Tick, Tick)> = times.iter().map(|t| (*t, to(*t))).collect();
            if l.switches.locked {
                sel.extend(times.iter().map(|t| KeyRef { layer: lid, prop: uid, time: *t }));
                continue;
            }
            let Some(pr) = l.props.find_mut(uid) else { continue };
            // Take the moving keys out, then put them back at their new times.
            let mut moved = vec![];
            for (old, new) in &moves {
                if let Some(i) = key_at(&pr.keys, *old) {
                    let mut k = pr.keys.remove(i);
                    k.time = *new;
                    moved.push(k);
                }
            }
            for k in moved {
                sel.push(KeyRef { layer: lid, prop: uid, time: k.time });
                match pr.keys.binary_search_by(|x| x.time.cmp(&k.time)) {
                    Ok(j) => pr.keys[j] = k,
                    Err(j) => pr.keys.insert(j, k),
                }
            }
            // Roving keys follow their neighbours: keep the selection on them.
            let before: Vec<Tick> = pr.keys.iter().map(|k| k.time).collect();
            if effectcraft_keyframe::retime_roving(&mut pr.keys, pr.spatial) {
                for r in sel.iter_mut().filter(|r| r.layer == lid && r.prop == uid) {
                    if let Some(i) = before.iter().position(|t| *t == r.time) {
                        r.time = pr.keys[i].time;
                    }
                }
            }
        }
        st.selected_keys = sel;
        Ok(json!(st.selected_keys.len()))
    })
}

fn move_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Tick::from_seconds_f64(f_p(p, "delta").ok_or_else(|| bad("keys.move", "missing `delta` (seconds)"))?);
    shift_keys(s, "Move Keyframes", merge_p(p), d)
}

fn delete_keys(s: &mut Session, _: &Value) -> Result<Value> {
    let cid = s.active_comp_id().ok_or(EngineError::NoComp)?;
    let sel = s.state.selected_keys.clone();
    s.edit("Delete Keyframes", None, |proj, st| {
        let comp = proj.comp_mut(cid).ok_or(EngineError::NoComp)?;
        for k in &sel {
            if let Some(pr) = comp.layer_mut(k.layer).and_then(|l| l.props.find_mut(k.prop))
                && let Some(i) = key_at(&pr.keys, k.time)
            {
                if pr.keys.len() == 1 {
                    pr.value = pr.keys[0].value.clone();
                }
                pr.keys.remove(i);
            }
        }
        st.selected_keys.clear();
        Ok(json!(sel.len()))
    })
}

fn ease(s: &mut Session, p: &Value) -> Result<Value> {
    let which = str_p(p, "which").unwrap_or("both").to_string();
    edit_keys(s, "Easy Ease", None, move |keys, i, _| {
        easy_ease(&mut keys[i], which != "out", which != "in");
        false
    })
}

fn interpolation(s: &mut Session, p: &Value) -> Result<Value> {
    let parse = |k: &str| match str_p(p, k).map(|v| v.to_ascii_lowercase()) {
        Some(v) if v == "linear" => Some(Interp::Linear),
        Some(v) if v == "bezier" => Some(Interp::Bezier),
        Some(v) if v == "hold" => Some(Interp::Hold),
        _ => None,
    };
    let both = parse("interpolation");
    let inn = parse("in").or(both);
    let out = parse("out").or(both);
    // Temporal interpolation by dialog name: linear|bezier|continuousBezier|autoBezier|hold.
    let temporal = str_p(p, "interpolation").map(|v| v.to_ascii_lowercase());
    let mut auto = b_p(p, "autoBezier");
    let mut continuous = b_p(p, "continuous");
    match temporal.as_deref() {
        Some("autobezier" | "auto") => auto = Some(true),
        Some("continuousbezier" | "continuous") => {
            continuous = Some(true);
            auto = Some(false);
        }
        Some("linear" | "bezier" | "hold") => {
            auto = auto.or(Some(false));
            continuous = continuous.or(Some(false));
        }
        _ => {}
    }
    let inn = if matches!(temporal.as_deref(), Some("autobezier" | "auto" | "continuousbezier" | "continuous")) { inn.or(Some(Interp::Bezier)) } else { inn };
    let out = if matches!(temporal.as_deref(), Some("autobezier" | "auto" | "continuousbezier" | "continuous")) { out.or(Some(Interp::Bezier)) } else { out };
    let spatial = str_p(p, "spatial").map(|v| v.to_ascii_lowercase());
    let roving = b_p(p, "roving");
    edit_keys(s, "Keyframe Interpolation", None, move |keys, i, _| {
        if let Some(sp) = spatial.as_deref() {
            // (Bezier keeps the current handles, with handles where the path left straight.)
            let (tin, tout) = effectcraft_keyframe::spatial_handles(keys, i);
            let k = &mut keys[i];
            match sp {
                "linear" => {
                    k.spatial_auto = false;
                    k.spatial_continuous = false;
                    k.spatial_in = [0.0; 3];
                    k.spatial_out = [0.0; 3];
                }
                "bezier" | "continuous" | "continuousbezier" => {
                    k.spatial_auto = false;
                    k.spatial_in = tin;
                    k.spatial_out = tout;
                    k.spatial_continuous = sp != "bezier";
                }
                "auto" | "autobezier" => {
                    k.spatial_auto = true;
                    k.spatial_continuous = false;
                }
                _ => {}
            }
        }
        if let Some(r) = roving {
            keys[i].roving = r;
        }
        if let Some(c) = continuous {
            keys[i].continuous = c;
        }
        let n = keys[i].value.dims().max(1);
        if let Some(x) = inn {
            keys[i].in_interp = x;
            if x == Interp::Bezier && keys[i].in_ease.is_empty() {
                keys[i].in_ease = vec![Ease::default(); n];
            }
        }
        if let Some(x) = out {
            keys[i].out_interp = x;
            if x == Interp::Bezier && keys[i].out_ease.is_empty() {
                keys[i].out_ease = vec![Ease::default(); n];
            }
        }
        if let Some(a) = auto {
            keys[i].auto_bezier = a;
            if a {
                keys[i].in_interp = Interp::Bezier;
                keys[i].out_interp = Interp::Bezier;
                keys[i].continuous = false;
            }
        }
        if keys[i].continuous && !keys[i].auto_bezier {
            // Continuous Bezier: one speed through the key (average of both sides).
            let k = &mut keys[i];
            k.in_interp = Interp::Bezier;
            k.out_interp = Interp::Bezier;
            if k.in_ease.len() != n {
                k.in_ease = vec![Ease::default(); n];
            }
            if k.out_ease.len() != n {
                k.out_ease = vec![Ease::default(); n];
            }
            for d in 0..n {
                let sp = (k.in_ease[d].speed + k.out_ease[d].speed) / 2.0;
                k.in_ease[d].speed = sp;
                k.out_ease[d].speed = sp;
            }
        }
        false
    })
}

fn toggle_hold(s: &mut Session, _: &Value) -> Result<Value> {
    edit_keys(s, "Toggle Hold Keyframe", None, |keys, i, _| {
        let k = &mut keys[i];
        // Off again: back to Bezier when the key eases (its other side, auto / continuous, or
        // an outgoing ease), else Linear.
        let bezier = k.auto_bezier || k.continuous || k.in_interp == Interp::Bezier || !k.out_ease.is_empty();
        k.out_interp = match k.out_interp {
            Interp::Hold if bezier => Interp::Bezier,
            Interp::Hold => Interp::Linear,
            _ => Interp::Hold,
        };
        false
    })
}

/// A number or a per-dimension array of numbers.
fn nums_p(p: &Value, k: &str, scale: f64) -> Option<Vec<f64>> {
    match p.get(k)? {
        Value::Array(a) => Some(a.iter().filter_map(Value::as_f64).map(|v| v * scale).collect()),
        v => v.as_f64().map(|v| vec![v * scale]),
    }
}

fn velocity(s: &mut Session, p: &Value) -> Result<Value> {
    let mut in_speed = nums_p(p, "inSpeed", 1.0);
    let mut in_inf = nums_p(p, "inInfluence", 0.01);
    let mut out_speed = nums_p(p, "outSpeed", 1.0);
    let mut out_inf = nums_p(p, "outInfluence", 0.01);
    // "Continuous" in the dialog: outgoing velocity follows incoming.
    let continuous = b_p(p, "continuous");
    if continuous == Some(true) {
        if out_speed.is_none() {
            out_speed = in_speed.clone();
        }
        if in_speed.is_none() {
            in_speed = out_speed.clone();
        }
        if out_inf.is_none() {
            out_inf = in_inf.clone();
        }
        if in_inf.is_none() {
            in_inf = out_inf.clone();
        }
    }
    edit_keys(s, "Keyframe Velocity", merge_p(p), move |keys, i, _| {
        let n = keys[i].value.dims().max(1);
        // Current presentation of each side, so unspecified parts keep their values.
        let cur = |keys: &[Keyframe], out: bool| -> Vec<Ease> {
            (0..n).map(|d| effectcraft_keyframe::side_ease(keys, i, d, false, out).unwrap_or(Ease::EASY)).collect()
        };
        let cur_in = cur(keys, false);
        let cur_out = cur(keys, true);
        let k = &mut keys[i];
        let apply = |list: &mut Vec<Ease>, base: Vec<Ease>, sp: &Option<Vec<f64>>, inf: &Option<Vec<f64>>| {
            if list.len() != n {
                *list = base;
            }
            for (d, e) in list.iter_mut().enumerate() {
                if let Some(v) = sp.as_ref().and_then(|v| v.get(d).or(v.first())) {
                    e.speed = *v;
                }
                if let Some(v) = inf.as_ref().and_then(|v| v.get(d).or(v.first())) {
                    e.influence = v.clamp(0.001, 1.0);
                }
            }
        };
        if in_speed.is_some() || in_inf.is_some() {
            k.in_interp = Interp::Bezier;
            apply(&mut k.in_ease, cur_in, &in_speed, &in_inf);
        }
        if out_speed.is_some() || out_inf.is_some() {
            k.out_interp = Interp::Bezier;
            apply(&mut k.out_ease, cur_out, &out_speed, &out_inf);
        }
        if let Some(c) = continuous {
            k.continuous = c;
        }
        k.auto_bezier = false;
        false
    })
}

fn convert_expr_to_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let (cid, lid, uid) = prop_ref(s, p, "prop.convertExpressionToKeyframes")?;
    let comp = s.project.comp(cid).ok_or(EngineError::NoComp)?.clone();
    let layer = comp.layer(lid).ok_or(EngineError::NoComp)?.clone();
    let pr = layer.props.find(uid).ok_or_else(|| bad("prop.convertExpressionToKeyframes", "no property"))?.clone();
    let mut keys = vec![];
    let fd = comp.frame_duration();
    let mut t = layer.in_point;
    while t < layer.out_point {
        let ctx =
            effectcraft_render::EvalCtx { project: &s.project, comp_id: cid, comp: &comp, time: t, expr: s.expr.as_deref(), footage: Some(s.footage.as_ref()) };
        keys.push(Keyframe::new(layer.layer_time(t), ctx.value(&layer, &pr)));
        t += fd;
    }
    // A time-reversed layer maps ascending comp times to descending layer times; keys must stay ordered by layer time.
    keys.sort_by_key(|k| k.time);
    let n = keys.len();
    with_prop(s, "Convert Expression to Keyframes", None, cid, lid, uid, move |pr, _| {
        pr.keys = keys;
        if let Some(e) = &mut pr.expr {
            e.enabled = false;
        }
        Ok(json!(n))
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        query!("prop.get", "Get Property", "{layer?, path|prop, time? (comp s)}", get),
        cmd!(
            "prop.set",
            "Set Property Value",
            [],
            None,
            "{layer?, path|prop, value, time? (layer s by default), timeBase?: layer|comp, merge?}",
            has_layers,
            set
        ),
        cmd!("prop.toggleAnimation", "Toggle Stopwatch", [], None, "{layer?, path|prop, value?, merge?}", has_layers, toggle_anim),
        cmd!("prop.addKey", "Add Keyframe", [], None, "{layer?, path|prop, time? (layer s by default), timeBase?: layer|comp, value?}", has_layers, add_key),
        cmd!("prop.toggleKey", "Add or Remove Keyframe at Current Time", [], None, "{layer?, path|prop}", has_layers, toggle_key),
        cmd!(
            "keys.toggleTransform",
            "Add or Remove Transform Keyframe",
            [],
            None,
            "{layers?, prop: anchor|position|scale|rotation|opacity} (Alt+Shift+A/P/S/R/T)",
            has_layers,
            toggle_transform_key
        ),
        cmd!(
            "prop.setExpression",
            "Add Expression",
            ["Animation"],
            Some("Alt+Shift+="),
            "{layer?, path|prop?, expression?, enabled?} (no path/prop: the selected properties)",
            has_layers,
            set_expr
        ),
        cmd!(
            "prop.reset",
            "Reset Property",
            [],
            None,
            "{layer?, path|prop, default?} → removes keyframes and expression; the value becomes `default`, else the property's own (an effect parameter's, a Transform property's)",
            has_layers,
            reset
        ),
        cmd!("prop.select", "Select Property", [], None, "{layer?, path|prop, add?, selectKeys?}", has_layers, select_prop),
        cmd!("prop.deselect", "Deselect Property", [], None, "{layer?, path|prop}", has_layers, deselect_prop),
        cmd!(
            "prop.convertExpressionToKeyframes",
            "Convert Expression to Keyframes",
            ["Animation", "Keyframe Assistant"],
            None,
            "{layer?, path|prop}",
            has_layers,
            convert_expr_to_keys
        ),
        cmd!(
            "keys.select",
            "Select Keyframes",
            [],
            None,
            "{keys: [{layer, prop: uid | path (or `path`), time (layer s by default)}], timeBase?: layer|comp, add?, toggle?: bool (Shift+click: in or out of the selection), selectProperties?: bool (their properties and layers too, as a Timeline click does)}",
            has_comp,
            select_keys
        ),
        cmd!("keys.move", "Move Keyframes", [], None, "{delta (s), merge?}", has_keys, move_keys),
        cmd!("keys.delete", "Delete Keyframes", [], None, "{}", has_keys, delete_keys),
        cmd!("keys.easyEase", "Easy Ease", ["Animation", "Keyframe Assistant"], Some("F9"), "{which?: both|in|out}", has_keys, ease),
        cmd!("keys.easyEaseIn", "Easy Ease In", ["Animation", "Keyframe Assistant"], Some("Shift+F9"), "{}", has_keys, |s, _| ease(s, &json!({"which": "in"}))),
        cmd!("keys.easyEaseOut", "Easy Ease Out", ["Animation", "Keyframe Assistant"], Some("Cmd+Shift+F9"), "{}", has_keys, |s, _| ease(
            s,
            &json!({"which": "out"})
        )),
        cmd!("keys.toggleHold", "Toggle Hold Keyframe", ["Animation"], Some("Cmd+Alt+H"), "{}", has_keys, toggle_hold),
        cmd!(
            "keys.interpolation",
            "Keyframe Interpolation...",
            ["Animation"],
            Some("Cmd+Alt+K"),
            "{interpolation?: linear|bezier|continuousBezier|autoBezier|hold, in?|out?: linear|bezier|hold, autoBezier?, continuous?, spatial?: linear|bezier|continuousBezier|autoBezier, roving?}",
            has_keys,
            interpolation
        ),
        cmd!(
            "keys.velocity",
            "Keyframe Velocity...",
            ["Animation"],
            Some("Cmd+Shift+K"),
            "{inSpeed?, inInfluence? %, outSpeed?, outInfluence? % (numbers or per-dimension arrays), continuous?}",
            has_keys,
            velocity
        ),
    ]
}
