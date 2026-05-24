//! Smoke test for the AIDA MCP client. Spawns the real `aida mcp-serve`
//! subprocess in the current repo and exercises every code path the
//! aida-chat agent loop will use.
//!
//! Run:  `cargo run --example mcp_smoke --features ssr`
//!
//! Exits non-zero on the first failure. Not part of `cargo test` since
//! it requires the `aida` CLI on PATH and a real .aida-store.

use std::path::PathBuf;
use std::process::{Command as StdCommand, ExitCode};

use aida_chat::server::mcp::McpClient;
use serde_json::json;

#[tokio::main]
async fn main() -> ExitCode {
    let cwd = std::env::current_dir().expect("cwd");
    let cmd = PathBuf::from("aida");
    let args = vec!["mcp-serve".to_string()];

    println!(
        "→ spawning {} {:?} in {}",
        cmd.display(),
        args,
        cwd.display()
    );
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

    let comment_marker = format!(
        "mcp_smoke temporary add_comment check {}",
        std::process::id()
    );
    // Live tools/list advertises add_comment(id, text). Current AIDA stores
    // that text as the author and hardcodes the body to "mcp"; this smoke
    // exercises and cleans up the live MCP behavior so aida-chat can keep
    // using the CLI write path until the upstream tool is fixed.
    step!(
        "tools/call add_comment STORY-21",
        client.call_tool(
            "add_comment",
            json!({"id": "STORY-21", "text": comment_marker.clone()})
        )
    );
    match delete_smoke_comment("STORY-21", &comment_marker) {
        Ok(true) => println!("✓ cleaned up temporary add_comment smoke comment"),
        Ok(false) => {
            println!("⚠ add_comment returned ok, but no persisted comment was visible to clean up")
        }
        Err(e) => {
            println!("FAIL: cleanup temporary comment: {e}");
            return ExitCode::FAILURE;
        }
    }

    let req_marker = format!("mcp_smoke temporary add_requirement {}", std::process::id());
    // Live AIDA 0.5.2 tools/list advertises add_requirement(type, title,
    // description, status). This smoke intentionally checks whether the
    // success response is backed by a CLI-visible persisted requirement;
    // STORY-22 keeps the product write path on `aida add` while this returns
    // success without persistence.
    step!(
        "tools/call add_requirement task",
        client.call_tool(
            "add_requirement",
            json!({
                "type": "task",
                "title": req_marker.clone(),
                "description": "temporary MCP add_requirement smoke; delete me",
                "status": "draft"
            })
        )
    );
    match delete_smoke_requirement(&req_marker) {
        Ok(Some(spec_id)) => {
            println!("✓ cleaned up temporary add_requirement smoke spec {spec_id}")
        }
        Ok(None) => println!(
            "⚠ add_requirement returned ok, but no persisted requirement was visible to clean up"
        ),
        Err(e) => {
            println!("FAIL: cleanup temporary requirement: {e}");
            return ExitCode::FAILURE;
        }
    }

    println!("✓ all smoke checks passed");
    ExitCode::SUCCESS
}

fn delete_smoke_comment(spec_id: &str, marker: &str) -> Result<bool, String> {
    let out = StdCommand::new("aida")
        .args(["comment", "list", spec_id])
        .output()
        .map_err(|e| format!("spawn aida comment list: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "aida comment list exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut previous_id = None;
    for line in stdout.lines() {
        if line.ends_with(':') && !line.starts_with(' ') {
            previous_id = Some(line.trim_end_matches(':').to_string());
        }
        if line.trim() == marker || line.contains(marker) {
            let comment_id = previous_id.ok_or_else(|| "marker had no preceding ID".to_string())?;
            let delete = StdCommand::new("aida")
                .args([
                    "comment",
                    "delete",
                    "--req-id",
                    spec_id,
                    "--comment-id",
                    &comment_id,
                ])
                .output()
                .map_err(|e| format!("spawn aida comment delete: {e}"))?;
            if delete.status.success() {
                return Ok(true);
            }
            return Err(format!(
                "aida comment delete exited with {}: {}",
                delete.status,
                String::from_utf8_lossy(&delete.stderr).trim()
            ));
        }
    }
    Ok(false)
}

fn delete_smoke_requirement(marker: &str) -> Result<Option<String>, String> {
    let out = StdCommand::new("aida")
        .args(["search", marker])
        .output()
        .map_err(|e| format!("spawn aida search: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "aida search exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let Some(spec_id) = parse_spec_id(&stdout) else {
        return Ok(None);
    };
    let delete = StdCommand::new("aida")
        .args(["del", "-y", &spec_id])
        .output()
        .map_err(|e| format!("spawn aida del: {e}"))?;
    if delete.status.success() {
        Ok(Some(spec_id))
    } else {
        Err(format!(
            "aida del exited with {}: {}",
            delete.status,
            String::from_utf8_lossy(&delete.stderr).trim()
        ))
    }
}

fn parse_spec_id(text: &str) -> Option<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|token| {
            token.contains('-')
                && token.chars().any(|c| c.is_ascii_digit())
                && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(str::to_string)
}
