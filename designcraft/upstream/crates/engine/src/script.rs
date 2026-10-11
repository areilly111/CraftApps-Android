//! Command scripts for agents and shells: a list of `{command, params}` steps where later steps
//! can use earlier results.
//!
//! **References.** A string parameter that is exactly `"$N.path"` (or `"$last.path"`, `"$N"`)
//! becomes the JSON value at `path` in the result of step `N` (0-based): after
//! `frame.create → {"id": 8, "story": 9}`, `{"story": "$0.story"}` passes `9`. Paths are dotted
//! keys and array indices (`$2.ids.0`). Inside longer strings `${N.path}` is replaced by the
//! value's text (`"Frame ${0.id}"`).
//!
//! **Script text** ([`parse`]): a JSON array of steps, JSON lines (`{"command": …, "params": …}`),
//! or compact lines `command.id {json}` / `command.id={json}` / `command.id`; blank lines and
//! lines starting with `#` or `//` are ignored.

use serde_json::{Value, json};

/// One step of a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub command: String,
    pub params: Value,
}

/// Parse script text (see the module docs).
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let t = text.trim();
    if t.starts_with('[') {
        let v: Value = serde_json::from_str(t).map_err(|e| format!("script: {e}"))?;
        return v
            .as_array()
            .ok_or("script: expected an array")?
            .iter()
            .enumerate()
            .map(|(i, s)| step_of(s).map_err(|e| format!("step {i}: {e}")))
            .collect();
    }
    let mut out = Vec::new();
    for (n, line) in t.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") {
            continue;
        }
        let step = if l.starts_with('{') {
            serde_json::from_str::<Value>(l).map_err(|e| e.to_string()).and_then(|v| step_of(&v))
        } else {
            // `id {json}`, `id={json}` or `id`.
            let split = l.find([' ', '=', '\t']).unwrap_or(l.len());
            let (id, rest) = l.split_at(split);
            let rest = rest.trim_start_matches(['=', ' ', '\t']).trim();
            let params = if rest.is_empty() { Ok(json!({})) } else { serde_json::from_str(rest).map_err(|e| e.to_string()) };
            params.map(|params| Step { command: id.to_string(), params })
        };
        out.push(step.map_err(|e| format!("line {}: {e}", n + 1))?);
    }
    Ok(out)
}

fn step_of(v: &Value) -> Result<Step, String> {
    let command = v.get("command").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or("missing `command`")?.to_string();
    let params = v.get("params").cloned().filter(|p| !p.is_null()).unwrap_or(json!({}));
    if !params.is_object() {
        return Err("`params` must be an object".into());
    }
    Ok(Step { command, params })
}

/// The value a reference (`N.path`, `last.path`) points at.
fn lookup(r: &str, results: &[Value]) -> Result<Value, String> {
    let mut parts = r.split('.');
    let head = parts.next().unwrap_or("");
    let idx = if head == "last" {
        results.len().checked_sub(1).ok_or("`$last` before any step")?
    } else {
        head.parse::<usize>().map_err(|_| format!("bad reference `${r}`"))?
    };
    let mut v = results.get(idx).ok_or_else(|| format!("`${r}`: step {idx} hasn't run (only {} have)", results.len()))?.clone();
    for p in parts {
        v = match (&v, p.parse::<usize>()) {
            (Value::Array(a), Ok(i)) => a.get(i).cloned(),
            (Value::Object(o), _) => o.get(p).cloned(),
            _ => None,
        }
        .ok_or_else(|| format!("`${r}`: no `{p}` in {}", short(&v)))?;
    }
    Ok(v)
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 120 { format!("{}…", &s[..s.floor_char_boundary(120)]) } else { s }
}

/// Replace references in `params` with values from earlier `results`.
pub fn resolve(params: &Value, results: &[Value]) -> Result<Value, String> {
    Ok(match params {
        Value::String(s) => {
            if let Some(r) = s.strip_prefix('$').filter(|r| !r.is_empty() && !r.contains(['{', ' '])) {
                return lookup(r, results);
            }
            if !s.contains("${") {
                return Ok(params.clone());
            }
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some(i) = rest.find("${") {
                out.push_str(&rest[..i]);
                let end = rest[i..].find('}').ok_or_else(|| format!("unclosed `${{` in {s:?}"))? + i;
                let v = lookup(&rest[i + 2..end], results)?;
                match v {
                    Value::String(t) => out.push_str(&t),
                    other => out.push_str(&other.to_string()),
                }
                rest = &rest[end + 1..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve(v, results)).collect::<Result<_, _>>()?),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| Ok((k.clone(), resolve(v, results)?))).collect::<Result<_, String>>()?),
        v => v.clone(),
    })
}

/// Outcome of running a script.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub results: Vec<Value>,
    /// The failing step (index, command, message); later steps didn't run.
    pub failed: Option<(usize, String, String)>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        let mut v = json!({"completed": self.results.len(), "results": self.results});
        if let Some((i, c, e)) = &self.failed {
            v["failedIndex"] = json!(i);
            v["failedCommand"] = json!(c);
            v["error"] = json!(e);
        }
        v
    }
}

/// Run `steps` through `exec`, resolving references; stops at the first error.
pub fn run(steps: &[Step], mut exec: impl FnMut(&str, Value) -> Result<Value, String>) -> Report {
    let mut results = Vec::with_capacity(steps.len());
    for (i, s) in steps.iter().enumerate() {
        let r = resolve(&s.params, &results).and_then(|p| exec(&s.command, p));
        match r {
            Ok(v) => results.push(v),
            Err(e) => return Report { results, failed: Some((i, s.command.clone(), e)) },
        }
    }
    Report { results, failed: None }
}

impl crate::Session {
    /// Run a script on this session (see [`crate::script`]).
    pub fn run_script(&mut self, steps: &[Step]) -> Report {
        run(steps, |c, p| self.execute(c, &p).map_err(|e| e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_forms() {
        let a = parse(r#"[{"command": "file.new"}, {"command": "frame.create", "params": {"rect": [0, 0, 10, 10]}}]"#).unwrap();
        let b = parse("# comment\nfile.new\n\nframe.create {\"rect\": [0, 0, 10, 10]}\n").unwrap();
        let c = parse("{\"command\": \"file.new\"}\nframe.create={\"rect\": [0, 0, 10, 10]}").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert!(parse("frame.create {bad").unwrap_err().contains("line 1"));
    }

    #[test]
    fn resolves_references() {
        let results = vec![json!({"id": 8, "story": 9, "ids": [4, 5]}), json!({"name": "A"})];
        let p = json!({"story": "$0.story", "id": "$0.ids.1", "label": "Frame ${0.id} of ${last.name}", "keep": "$", "list": ["$1.name"]});
        let r = resolve(&p, &results).unwrap();
        assert_eq!(r, json!({"story": 9, "id": 5, "label": "Frame 8 of A", "keep": "$", "list": ["A"]}));
        assert!(resolve(&json!({"x": "$5.id"}), &results).unwrap_err().contains("hasn't run"));
        assert!(resolve(&json!({"x": "$0.nope"}), &results).unwrap_err().contains("no `nope`"));
    }

    #[test]
    fn runs_on_a_session_with_references() {
        let mut s = crate::Session::new();
        let steps = parse(
            "file.new {\"pages\": 2}\n\
             frame.create {\"rect\": [72, 72, 300, 200], \"content\": \"text\", \"text\": \"Hello\"}\n\
             text.select {\"story\": \"$1.story\", \"anchor\": 5, \"focus\": 5}\n\
             text.insert {\"text\": \" world\"}\n\
             story.get {\"story\": \"$1.story\"}",
        )
        .unwrap();
        let r = s.run_script(&steps);
        assert!(r.failed.is_none(), "{r:?}");
        assert_eq!(r.results[4]["text"], "Hello world");
        // Failure reports the step and keeps earlier results.
        let r = s.run_script(&parse("document.inspect\nno.such.command\ndocument.inspect").unwrap());
        assert_eq!(r.results.len(), 1);
        assert_eq!(r.failed.as_ref().map(|f| (f.0, f.1.as_str())), Some((1, "no.such.command")));
        assert_eq!(r.to_json()["failedIndex"], 1);
    }
}
