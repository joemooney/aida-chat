//! Smoke test for the AIDA MCP client. Spawns the real `aida mcp-serve`
//! subprocess in the current repo and exercises every code path the
//! aida-chat agent loop will use.
//!
//! Run:  `cargo run --example mcp_smoke --features ssr`
//!
//! Exits non-zero on the first failure. Not part of `cargo test` since
//! it requires the `aida` CLI on PATH and a real .aida-store.

use std::path::PathBuf;
use std::process::ExitCode;

use aida_chat::server::mcp::McpClient;
use serde_json::json;

#[tokio::main]
async fn main() -> ExitCode {
    let cwd = std::env::current_dir().expect("cwd");
    let cmd = PathBuf::from("aida");
    let args = vec!["mcp-serve".to_string()];

    println!("→ spawning {} {:?} in {}", cmd.display(), args, cwd.display());
    let client = match McpClient::global(&cmd, &args, &cwd).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("✓ initialize handshake");

    macro_rules! step {
        ($label:expr, $fut:expr) => {{
            print!("→ {} ... ", $label);
            match $fut.await {
                Ok(s) => {
                    let preview: String = s.chars().take(120).collect();
                    println!("ok ({} bytes) — {preview}", s.len());
                    s
                }
                Err(e) => {
                    println!("FAIL: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }};
    }

    step!(
        "tools/call list_requirements",
        client.call_tool("list_requirements", json!({"limit": 3}))
    );
    step!(
        "tools/call show_requirement EPIC-1",
        client.call_tool("show_requirement", json!({"id": "EPIC-1"}))
    );
    step!(
        "tools/call search_requirements",
        client.call_tool("search_requirements", json!({"query": "chat"}))
    );

    print!("→ resources/list ... ");
    let resources = match client.list_resources().await {
        Ok(r) => r,
        Err(e) => {
            println!("FAIL: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("ok ({} resources)", resources.len());
    for r in &resources {
        println!("    - {} ({:?})", r.uri, r.name);
    }

    if let Some(first) = resources.first() {
        step!(
            format!("resources/read {}", first.uri),
            client.read_resource(&first.uri)
        );
    }

    println!("✓ all smoke checks passed");
    ExitCode::SUCCESS
}
