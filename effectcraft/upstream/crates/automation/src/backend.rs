//! Where tools run: an in-process session, or the desktop app over its control channel.

use effectcraft_engine::Session;
use effectcraft_engine::time::Tick;
use serde_json::{Value, json};

use crate::{BridgeClient, Error, Result, base64, encode_png, recompress_png};

pub enum Backend {
    /// An in-process session (no window): engine commands, inspection and rendering.
    Headless(Box<Session>),
    /// A running desktop app reached over its control channel: everything, plus the live UI.
    Bridge(BridgeClient),
}

/// A rendered comp frame.
pub struct Frame {
    pub png: Vec<u8>,
    pub comp: Value,
    pub time: f64,
    pub width: u32,
    pub height: u32,
}

impl Backend {
    pub fn headless(session: Session) -> Self {
        Backend::Headless(Box::new(session))
    }

    /// Bridge to the app's control channel at `addr` (`9877` or `127.0.0.1:9877`).
    pub fn bridge(addr: &str) -> Result<Self> {
        Ok(Backend::Bridge(BridgeClient::new(addr)?))
    }

    pub fn is_bridge(&self) -> bool {
        matches!(self, Backend::Bridge(_))
    }

    pub fn session(&mut self) -> Option<&mut Session> {
        match self {
            Backend::Headless(s) => Some(s),
            Backend::Bridge(_) => None,
        }
    }

    /// Run an engine command by id.
    pub fn exec(&mut self, id: &str, params: Value) -> Result<Value> {
        let params = if params.is_null() { json!({}) } else { params };
        match self {
            Backend::Headless(s) => {
                let r = s.execute_checked(id, params);
                s.drain_events();
                Ok(r?)
            }
            Backend::Bridge(b) => b.call("engine.execute", json!({"command": id, "params": params})),
        }
    }

    /// Call a raw control-channel method (bridge only).
    pub fn control(&mut self, method: &str, params: Value) -> Result<Value> {
        match self {
            Backend::Bridge(b) => b.call(method, if params.is_null() { json!({}) } else { params }),
            Backend::Headless(_) => Err(Error::BadArgs(NEED_BRIDGE.into())),
        }
    }

    /// Render a comp frame (comp time `time` seconds, default the CTI) as PNG, longest side at most
    /// `max_side` (0 = full size).
    pub fn render(&mut self, comp: Option<&Value>, time: Option<f64>, max_side: u32) -> Result<Frame> {
        self.render_with(comp, time, max_side, false)
    }

    /// [`Backend::render`]; `transparent` keeps the frame's alpha instead of compositing it over
    /// the comp's background colour.
    pub fn render_with(&mut self, comp: Option<&Value>, time: Option<f64>, max_side: u32, transparent: bool) -> Result<Frame> {
        match self {
            Backend::Headless(s) => {
                let cid = s.resolve_comp(comp).map_err(|_| no_comp(comp))?;
                let t = time.map(Tick::from_seconds_f64).unwrap_or_else(|| s.time());
                let (w, h, rgba) = s.render_rgba8_alpha(cid, t, max_side, transparent)?;
                let png = encode_png(w, h, rgba, max_side)?;
                Ok(Frame { png, comp: json!(cid.0), time: t.seconds(), width: w, height: h })
            }
            Backend::Bridge(b) => {
                let mut p = json!({"max_side": max_side, "base64": true});
                if transparent {
                    p["transparent"] = json!(true);
                }
                if let Some(t) = time {
                    p["time"] = json!(t);
                }
                if let Some(c) = comp.filter(|c| !c.is_null()) {
                    p["comp"] = c.clone();
                }
                let r = b.call("render.frame", p)?;
                let data = r.get("png").and_then(Value::as_str).ok_or_else(|| Error::App("render.frame returned no `png`".into()))?;
                let raw = base64::decode(data).ok_or_else(|| Error::App("render.frame: bad base64".into()))?;
                let (png, width, height) = recompress_png(&raw, max_side)?;
                Ok(Frame { png, comp: r["comp"].clone(), time: r["time"].as_f64().unwrap_or(0.0), width, height })
            }
        }
    }
}

fn no_comp(comp: Option<&Value>) -> Error {
    match comp.filter(|c| !c.is_null()) {
        Some(c) => Error::BadArgs(format!("no composition {c} (see get_project)")),
        None => Error::BadArgs("no active composition: create one (execute_command comp.new) or open a project".into()),
    }
}

pub const NEED_BRIDGE: &str =
    "this tool drives the live app: start it with `effectcraft --control 9877` and run the MCP server as `effectcraft-cli mcp --bridge 9877`";
