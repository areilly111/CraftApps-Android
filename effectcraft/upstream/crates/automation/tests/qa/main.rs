//! M13.15 end-to-end QA: realistic projects built only through the agent interfaces (MCP tools
//! over JSON-RPC, the same commands the CLI and control channel run), asserting on rendered
//! pixels and written files rather than on command success alone.

mod affordances;
mod compositing;
mod harness;
mod kinetic;
mod lifecycle;
mod shapes;
mod tracking;
