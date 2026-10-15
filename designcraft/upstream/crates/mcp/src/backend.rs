//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, …). See `designcraft_ui_egui::control` for the full list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.screenshot`, `ui.click`, dialogs… work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "connected to 127.0.0.1:7979").
    fn describe(&self) -> String;
}

/// A running DesignCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
}

impl Remote {
    /// Connect to `addr` (`127.0.0.1:7979`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1 };
        r.reconnect()?;
        Ok(r)
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(Duration::from_secs(90))).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        if self.conn.is_none() {
            self.reconnect()?;
        }
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected"));
        };
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        let mut reply = String::new();
        if reader.read_line(&mut reply)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed"));
        }
        Ok(reply)
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let reply = match self.roundtrip(&line) {
            Ok(r) => r,
            Err(e) => {
                self.conn = None;
                return Err(format!("DesignCraft app at {}: {e}; operation may have completed; inspect state before retrying", self.addr));
            }
        };
        let v: Value = serde_json::from_str(reply.trim()).map_err(|e| format!("bad reply from app: {e}"))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("connected to the DesignCraft app at {}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn remote_does_not_replay_when_reply_is_lost() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let first = {
                let (stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["method"], "engine.execute");
                assert_eq!(request["params"]["command"], "frame.create");
                request
            };
            let mut applied = 1;
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                let request: Value = serde_json::from_str(&line).unwrap();
                if request == first {
                    applied += 1;
                    writeln!(stream, "{}", json!({"id": request["id"], "ok": true, "result": {"applied": applied}})).unwrap();
                } else {
                    assert_eq!(request, json!({"id": 2, "method": "document.inspect", "params": {}}));
                    writeln!(stream, "{}", json!({"id": request["id"], "ok": true, "result": {"applied": applied}})).unwrap();
                    return applied;
                }
            }
        });
        let mut remote = Remote::connect(&addr).unwrap();

        let result = remote.call("engine.execute", json!({"command": "frame.create", "params": {"rect": [0, 0, 100, 100]}}));
        let next = remote.call("document.inspect", json!({}));
        let applied = server.join().unwrap();

        assert_eq!(applied, 1, "lost reply must not replay the applied edit; client returned {result:?}");
        assert!(result.is_err(), "lost reply must report an uncertain outcome: {result:?}");
        assert_eq!(next, Ok(json!({"applied": 1})), "the next explicit request must reconnect successfully");
    }
}
