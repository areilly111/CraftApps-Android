//! Live-effect plug-ins: a plug-in of kind `effect` rewrites one object's geometry each time it is
//! drawn, from its appearance stack (an `Effect` record with id `plugin.<plug-in id>`).
//!
//! Drawing evaluates effects often (every frame, hit tests, bounds), so results are cached by
//! plug-in installation, parameters and input geometry: the plug-in runs again only when one of
//! them changes. Failures are cached too (the geometry is then drawn unchanged) and reported by
//! [`last_error`].

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use serde_json::{Value, json};
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::objects::{decode, output_geometry};
use crate::{Error, Kind, Limits, Result, registry};

/// The effect id prefix of live-effect plug-ins: `plugin.<plug-in id>`.
pub const PREFIX: &str = "plugin.";

/// Wall-clock budget of one live-effect run (drawing waits for it), in milliseconds.
pub const WALL_TIME_MS: u64 = 5_000;

/// Cached results kept before the cache starts over.
const CACHE_SIZE: usize = 512;

/// The plug-in id of effect id `id` (`plugin.<id>`).
pub fn plugin_id(effect_id: &str) -> Option<&str> {
    effect_id.strip_prefix(PREFIX)
}

/// The effect id of plug-in `id`.
pub fn effect_id(plugin_id: &str) -> String {
    format!("{PREFIX}{plugin_id}")
}

/// An installed live-effect plug-in by its effect id.
pub fn installed(effect_id: &str) -> Option<std::sync::Arc<crate::Plugin>> {
    registry::get(plugin_id(effect_id)?).filter(|p| p.manifest().kind == Kind::Effect)
}

#[derive(PartialEq)]
struct Key {
    serial: u64,
    params: String,
    bounds: [u64; 4],
    rule: FillRule,
    path: PathData,
}

impl Key {
    fn hash64(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.serial, &self.params, self.bounds, self.rule).hash(&mut h);
        for sp in &self.path.subpaths {
            sp.closed.hash(&mut h);
            for a in &sp.anchors {
                for p in [a.p, a.h_in, a.h_out] {
                    (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
                }
            }
        }
        h.finish()
    }
}

type Cache = HashMap<u64, (Key, Option<PathData>)>;

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static ERRORS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// The last error of live-effect plug-in `id` (cleared when it next succeeds).
pub fn last_error(id: &str) -> Option<String> {
    ERRORS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(id).cloned()
}

fn note(id: &str, r: &Result<PathData>) {
    let mut errors = ERRORS.lock().unwrap_or_else(|e| e.into_inner());
    let errors = errors.get_or_insert_with(HashMap::new);
    match r {
        Ok(_) => errors.remove(id),
        Err(e) => errors.insert(id.to_string(), e.to_string()),
    };
}

/// Runs live-effect plug-in `id` on `path` (whose reference box is `bounds`): the new geometry,
/// or `None` when the plug-in isn't installed, isn't an effect, or fails (the caller then keeps
/// the geometry).
pub fn apply(id: &str, params: &Value, path: &PathData, bounds: Rect, rule: FillRule) -> Option<PathData> {
    let plugin = registry::get(id).filter(|p| p.manifest().kind == Kind::Effect)?;
    let resolved = Value::Object(plugin.manifest().resolve_params(params).ok()?);
    let key = Key {
        serial: plugin.serial(),
        params: resolved.to_string(),
        bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1].map(f64::to_bits),
        rule,
        path: path.clone(),
    };
    let h = key.hash64();
    if let Some((k, out)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(&h))
        && *k == key
    {
        return out.clone();
    }
    let r = run(&plugin, resolved, path, bounds, rule);
    note(id, &r);
    let out = r.ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() >= CACHE_SIZE {
        cache.clear();
    }
    cache.insert(h, (key, out.clone()));
    out
}

fn run(plugin: &crate::Plugin, mut params: Value, path: &PathData, bounds: Rect, rule: FillRule) -> Result<PathData> {
    let b = json!([bounds.x0, bounds.y0, bounds.x1, bounds.y1]);
    params["_context"] = json!({"mode": "effect", "bounds": b});
    let rule = match rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    };
    let input = json!({"objects": [{"type": "path", "path": path, "fillRule": rule, "bounds": b}]});
    let (input, params) = (serde_json::to_vec(&input), serde_json::to_vec(&params));
    let (Ok(input), Ok(params)) = (input, params) else { return Err(Error::Params("can't encode the input".into())) };
    let limits = Limits { wall_time_ms: WALL_TIME_MS, ..plugin.limits().clone() };
    Ok(output_geometry(decode(&plugin.run_limited(&input, &params, &limits)?)?))
}
