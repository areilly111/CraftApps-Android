//! Long renders over MCP (docs/mcp.md "Progress and cancellation"). While [`McpServer::serve`]
//! runs, a blocking `renderQueue.render` (`command_run`, headless) renders on the session's
//! background render job instead of inline: the server keeps reading stdin, answers every other
//! request in between (the job renders a snapshot of the project), reports progress as
//! `notifications/progress` when the request carried `_meta.progressToken`, and stops the render
//! on `notifications/cancelled` (deleting the partial output, sending no response).

use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use serde_json::{Value, json};

use crate::server::{McpServer, call_result};
use crate::tools::{self, Reply};

/// A `tools/call` result for `call`, with `resultType` for modern clients.
fn result_for(call: &LongCall, r: Result<Reply, crate::Error>) -> Value {
    let mut result = call_result(r);
    if call.modern {
        result["resultType"] = json!("complete");
    }
    result
}

/// How often the job is polled (and at most how often progress is reported).
const POLL: Duration = Duration::from_millis(100);

/// A `tools/call` that runs as a long job.
pub struct LongCall {
    pub id: Value,
    pub params: Value,
    pub token: Option<Value>,
    /// The request declared MCP 2026-07-28+ (its result then carries `resultType`).
    pub modern: bool,
}

/// Whether `msg` is a blocking `renderQueue.render` call (valid arguments, headless backend).
pub fn long_call(server: &mut McpServer, msg: &Value) -> Option<LongCall> {
    let id = msg.get("id").filter(|i| i.is_string() || i.is_number())?;
    if msg.get("method").and_then(Value::as_str) != Some("tools/call") || server.backend().is_bridge() {
        return None;
    }
    let p = msg.get("params")?;
    let name = p.get("name")?.as_str()?;
    let args = p.get("arguments").cloned().unwrap_or(json!({}));
    if !matches!(name, "command_run" | "execute_command")
        || args.get("id").or_else(|| args.get("command")).and_then(Value::as_str) != Some("renderQueue.render")
    {
        return None;
    }
    if tools::find(name).and_then(|t| t.unknown_arg(&args)).is_some() {
        return None; // the normal path reports it
    }
    let params = match args.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(v) if v.is_object() => v.clone(),
        _ => return None,
    };
    if params.get("wait").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let token = p.get("_meta").and_then(|m| m.get("progressToken")).filter(|t| t.is_string() || t.is_number()).cloned();
    Some(LongCall { id: id.clone(), params, token, modern: crate::server::is_modern(p) })
}

fn send(out: &mut impl Write, v: &Value) -> std::io::Result<()> {
    out.write_all(v.to_string().as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// The `requestId` of a `notifications/cancelled` line.
fn cancelled_id(line: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(line).ok()?;
    (v.get("method")?.as_str()? == "notifications/cancelled").then(|| v.get("params")?.get("requestId").cloned())?
}

impl McpServer {
    /// Run `call` to completion or cancellation, handling the other messages from `inbox`
    /// (`None` = stdin closed) meanwhile. Returns false when stdin closed.
    pub(crate) fn run_long(&mut self, call: LongCall, inbox: &Receiver<Option<String>>, out: &mut impl Write) -> std::io::Result<bool> {
        let mut params = call.params.clone();
        params["wait"] = json!(false);
        let start = self.guarded("command_run", &json!({"id": "renderQueue.render", "params": params}));
        let items = match &start {
            Ok(Reply::Json(v)) => v["items"].as_array().cloned().unwrap_or_default(),
            _ => {
                send(out, &json!({"jsonrpc": "2.0", "id": call.id, "result": result_for(&call, start)}))?;
                return Ok(true);
            }
        };
        let ids: Vec<u64> = items.iter().filter_map(|i| i["id"].as_u64()).collect();
        let shared = self.backend().session().and_then(|s| s.render_job.as_ref().map(|job| job.shared.clone()));
        let mut last = -1.0f64;
        let mut last_report: Option<std::time::Instant> = None;
        let mut open = true;
        let mut cancelled = false;
        loop {
            let state = shared.as_ref().map(|job| job.snapshot());
            let rendering = state.as_ref().is_some_and(|state| !state.finished);
            if let (Some(token), Some(st)) = (&call.token, &state)
                && st.items_total > 0
                && !cancelled
            {
                let frac = if st.total > 0 { st.done as f64 / st.total as f64 } else { 0.0 };
                // the current item's frames count until it is done
                let progress = (st.items_done as f64 + if st.items_done < st.items_total { frac.min(1.0) } else { 0.0 }).min(st.items_total as f64);
                if progress > last && (!rendering || last_report.is_none_or(|at| at.elapsed() >= POLL)) {
                    last = progress;
                    last_report = Some(std::time::Instant::now());
                    let message = format!(
                        "Rendering item {} of {}: frame {} of {}",
                        st.items_done.saturating_add(1).min(st.items_total),
                        st.items_total,
                        st.done,
                        st.total
                    );
                    send(
                        out,
                        &json!({"jsonrpc": "2.0", "method": "notifications/progress",
                        "params": {"progressToken": token, "progress": progress, "total": st.items_total, "message": message}}),
                    )?;
                }
            }
            if !rendering {
                break;
            }
            if !open {
                // stdin closed: finish the render (a piped script expects its reply), read nothing more
                std::thread::sleep(POLL);
                continue;
            }
            match inbox.recv_timeout(POLL) {
                Ok(Some(line)) => {
                    if cancelled_id(&line).as_ref() == Some(&call.id) {
                        cancelled = true;
                        if let Some(job) = &shared {
                            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        continue;
                    }
                    if let Some(other) = serde_json::from_str::<Value>(&line).ok().and_then(|msg| long_call(self, &msg)) {
                        let result = result_for(&other, Err(crate::Error::Other("a render is already in progress".into())));
                        send(out, &json!({"jsonrpc":"2.0", "id":other.id, "result":result}))?;
                        continue;
                    }
                    if let Some(reply) = self.handle_line(&line) {
                        out.write_all(reply.as_bytes())?;
                        out.write_all(b"\n")?;
                        out.flush()?;
                    }
                }
                Ok(None) | Err(RecvTimeoutError::Disconnected) => open = false,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        let Some(s) = self.backend().session() else { return Ok(open) };
        s.poll_render();
        s.drain_events();
        let list = self.guarded("command_run", &json!({"id": "renderQueue.list"}));
        let items: Vec<Value> = match &list {
            Ok(Reply::Json(v)) => {
                v["items"].as_array().cloned().unwrap_or_default().into_iter().filter(|i| i["id"].as_u64().is_some_and(|id| ids.contains(&id))).collect()
            }
            _ => Vec::new(),
        };
        if cancelled {
            // Exporters remove only files they actually opened for this interrupted output.
            return Ok(open); // MCP: no response for a cancelled request
        }
        let failed = ids.len().saturating_sub(items.len()).saturating_add(items.iter().filter(|item| item["statusLabel"].as_str() != Some("Done")).count());
        let result = result_for(&call, Ok(Reply::Json(json!({"rendering": false, "items": items, "failed": failed}))));
        send(out, &json!({"jsonrpc": "2.0", "id": call.id, "result": result}))?;
        Ok(open)
    }
}
