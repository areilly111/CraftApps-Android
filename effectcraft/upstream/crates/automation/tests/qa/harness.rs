//! A motion designer's seat: an MCP server over a fully wired headless session, spoken to in
//! JSON-RPC exactly as an MCP client would (`tools/call` lines in, replies out), plus pixel
//! helpers for asserting on what was rendered.

use std::path::{Path, PathBuf};

use effectcraft_automation::{Backend, McpServer, base64};
use serde_json::{Value, json};

pub struct Qa {
    mcp: McpServer,
    next_id: u64,
    pub dir: PathBuf,
}

pub type Img = image::RgbaImage;

impl Qa {
    /// A fresh headless MCP session and an empty scratch folder for its files.
    pub fn new(name: &str) -> Qa {
        let dir = std::env::temp_dir().join(format!("effectcraft-qa-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut qa = Qa { mcp: McpServer::new(Backend::headless(effectcraft_host::session())), next_id: 0, dir };
        let init = qa.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "qa", "version": "0"}}));
        assert_eq!(init["serverInfo"]["name"], "effectcraft");
        qa
    }

    /// A path inside the scratch folder.
    pub fn path(&self, rel: &str) -> String {
        self.dir.join(rel).to_string_lossy().into_owned()
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let line = json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}).to_string();
        let reply: Value = serde_json::from_str(&self.mcp.handle_line(&line).expect("a reply")).expect("JSON reply");
        assert_eq!(reply["id"], self.next_id);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// Call a tool; `Err(message)` when it reports `isError`.
    pub fn try_tool(&mut self, name: &str, args: Value) -> Result<Vec<Value>, String> {
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        let content = r["content"].as_array().cloned().unwrap_or_default();
        if r["isError"].as_bool() == Some(true) {
            return Err(content.first().and_then(|c| c["text"].as_str()).unwrap_or("").to_string());
        }
        Ok(content)
    }

    /// Call a tool that answers JSON; panics with the tool's message if it fails.
    pub fn tool(&mut self, name: &str, args: Value) -> Value {
        match self.try_tool(name, args.clone()) {
            Ok(c) => serde_json::from_str(c[0]["text"].as_str().expect("text content")).expect("JSON text"),
            Err(e) => panic!("{name} {args}: {e}"),
        }
    }

    /// `execute_command`.
    pub fn exec(&mut self, command: &str, params: Value) -> Value {
        self.tool("execute_command", json!({"command": command, "params": params}))
    }

    /// `execute_command` expected to fail; returns the message.
    pub fn exec_err(&mut self, command: &str, params: Value) -> String {
        match self.try_tool("execute_command", json!({"command": command, "params": params})) {
            Ok(c) => panic!("{command} {params} should fail, got {c:?}"),
            Err(e) => e,
        }
    }

    /// `render_frame` at full size, decoded.
    pub fn frame(&mut self, comp: Option<Value>, time: f64) -> Img {
        let mut args = json!({"time": time, "max_side": 0});
        if let Some(c) = comp {
            args["comp"] = c;
        }
        let c = self.try_tool("render_frame", args).unwrap_or_else(|e| panic!("render_frame: {e}"));
        assert_eq!(c[0]["type"], "image");
        let png = base64::decode(c[0]["data"].as_str().unwrap()).expect("base64");
        image::load_from_memory(&png).expect("PNG").to_rgba8()
    }
}

impl Drop for Qa {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Fraction of pixels matching `f`.
pub fn coverage(img: &Img, f: impl Fn(&image::Rgba<u8>) -> bool) -> f64 {
    img.pixels().filter(|p| f(p)).count() as f64 / (img.width() * img.height()).max(1) as f64
}

/// Mean absolute difference per channel (0–255) of two same-size images.
pub fn mean_diff(a: &Img, b: &Img) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions());
    let s: u64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    s as f64 / a.as_raw().len() as f64
}

/// Centroid (x, y) of the pixels matching `f`.
pub fn centroid(img: &Img, f: impl Fn(&image::Rgba<u8>) -> bool) -> Option<(f64, f64)> {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (x, y, p) in img.enumerate_pixels() {
        if f(p) {
            sx += x as f64;
            sy += y as f64;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}

pub fn luma(p: &image::Rgba<u8>) -> f64 {
    0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
}

pub fn load(path: impl AsRef<Path>) -> Img {
    image::open(path.as_ref()).unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().display())).to_rgba8()
}
