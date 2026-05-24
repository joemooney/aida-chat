// trace:EPIC-16 | ai:claude
//
// Minimal stdio JSON-RPC 2.0 client for AIDA's MCP server. One
// long-lived subprocess per aida-chat process, lazy-spawned on first
// use. If the subprocess dies, the singleton slot is cleared so the
// next call respawns it; legacy tools use CLI fallback for the
// in-flight failure. The bulk of the implementation lives in
// `client.rs`; this file just re-exports the public surface.

pub mod client;
pub mod protocol;

pub use client::{McpClient, McpError, ResourceMeta};
