//! What the user did recently, read from the command log: the shortcut menus' Recent Commands.

use crate::{Session, find_command};

/// How a started command appears in the log ([`Session::start`]).
const STARTED: &str = "Command: ";

impl Session {
    /// The commands started most recently (typed, from a menu or repeated), newest first, each
    /// once, at most `max`. Ids are registered command ids.
    pub fn recent_commands(&self, max: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in self.log.iter().rev() {
            if out.len() >= max {
                break;
            }
            let Some(name) = line.strip_prefix(STARTED) else { continue };
            let Some(spec) = find_command(name.trim()) else { continue };
            if !out.iter().any(|c| c == spec.id) {
                out.push(spec.id.to_string());
            }
        }
        out
    }
}
