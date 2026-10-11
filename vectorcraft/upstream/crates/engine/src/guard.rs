//! Last line of defence against bugs: frontends never crash on a panic.
//!
//! Code must not panic (see `AGENTS.md` › Robustness); fallible work returns `Result`. Should a bug
//! panic anyway, the entry points ([`crate::Session::execute`], tool events, the MCP server, the UI
//! frame and the control channel) catch it with [`catch_panic`], roll back what they can and report
//! an error instead of taking the app (and the user's unsaved work) down.
//!
//! On wasm the panic strategy is `abort`, so this only helps native builds; the rule against
//! panicking code is what protects the web app.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`, turning a panic into `Err(message)`. The default panic hook still prints the message
/// and location to stderr. Callers must leave their state consistent after an `Err` (roll back, or
/// rebuild the part `f` was working on).
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        let msg = panic_message(payload.as_ref());
        log::error!("recovered from a panic: {msg}");
        msg
    })
}

/// The message of a panic payload (`panic!("…")` carries a `&str` or a `String`).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_panic_returns_value_or_message() {
        assert_eq!(catch_panic(|| 7), Ok(7));
        assert_eq!(catch_panic(|| -> i32 { panic!("boom {}", 1) }), Err("boom 1".to_string()));
        assert_eq!(catch_panic(|| -> i32 { std::panic::panic_any(5_u8) }), Err("unknown panic".to_string()));
    }
}
