// trace:STORY-4 EPIC-16 | ai:claude
//
// AIDA query tools. Two transports:
//
//   1. **MCP** (preferred): one long-lived `aida mcp-serve` subprocess,
//      managed by `server::mcp::McpClient`. Used for `aida_list`,
//      `aida_show`, `aida_search`, and the new `aida_resource`.
//   2. **CLI fallback**: `Command::new("aida").args([...])` with an
//      explicit subcommand allowlist. Used when MCP returns
//      `Unavailable`, `Closed`, or `Timeout` — and always for
//      `aida_history`, since AIDA's MCP server does not expose a
//      `history` tool.
//
// Either way the model can never reach a shell: arguments are passed as
// explicit `args` (no shell expansion) and the subcommand is fixed.

use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;
use crate::server::mcp::{McpClient, McpError};

// --------------------------------------------------------------------------
// Tool specs (stable contract for the model)
// --------------------------------------------------------------------------

pub fn aida_list_spec() -> Tool {
    Tool {
        name: "aida_list",
        description: "List requirements tracked in this project's AIDA store. Optional filters: \
            status (draft|approved|in-progress|completed|rejected) and type (epic|story|task|bug|spike|...). \
            Returns a compact one-line-per-requirement summary.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "status": {"type": "string"},
                "type":   {"type": "string"}
            }
        }),
    }
}

pub fn aida_show_spec() -> Tool {
    Tool {
        name: "aida_show",
        description: "Show full details of a single AIDA requirement by its SPEC-ID \
            (e.g. EPIC-1, STORY-2, BUG-17). Returns the title, status, description body, \
            comments, and links.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "SPEC-ID such as EPIC-1, STORY-2, BUG-17"
                }
            },
            "required": ["id"]
        }),
    }
}

pub fn aida_search_spec() -> Tool {
    Tool {
        name: "aida_search",
        description: "Full-text search across AIDA requirement titles and descriptions. \
            Case-insensitive. Returns the matching SPEC-IDs and one-line summaries.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        }),
    }
}

pub fn aida_history_spec() -> Tool {
    Tool {
        name: "aida_history",
        description: "Recent activity on this project's requirements — what was touched and how \
            it stands now. Use this when the user asks 'what changed lately' or 'what was I \
            working on'. Returns a digest sorted by last-touch time.",
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

pub fn aida_resource_spec() -> Tool {
    Tool {
        name: "aida_resource",
        description: "Read substrate artefacts that the structured aida_* tools don't cover — \
            plan archives, project summary, requirements tree, and other resources exposed by \
            the AIDA MCP server. Use action='list' first to discover available URIs, then \
            action='read' with a specific URI. Only available when the AIDA MCP server is \
            reachable; errors cleanly otherwise.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "read"]
                },
                "uri": {
                    "type": "string",
                    "description": "Required when action='read'. A URI returned by action='list'."
                }
            },
            "required": ["action"]
        }),
    }
}

// --------------------------------------------------------------------------
// Executors
// --------------------------------------------------------------------------

pub async fn aida_list(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let status = input.get("status").and_then(|v| v.as_str());
    let ty = input.get("type").and_then(|v| v.as_str());
    for arg in [status, ty].into_iter().flatten() {
        if !is_simple_token(arg) {
            return Err(ToolError::BadInput(format!(
                "invalid token in arg: {arg:?}"
            )));
        }
    }

    let mut mcp_args = serde_json::Map::new();
    if let Some(s) = status {
        mcp_args.insert("status".into(), Value::String(s.into()));
    }
    if let Some(t) = ty {
        mcp_args.insert("type".into(), Value::String(t.into()));
    }
    match try_mcp(cfg, "list_requirements", Value::Object(mcp_args)).await {
        Ok(text) => Ok(text),
        Err(ToolError::Execution(e)) if is_unavailable(&e) => {
            let mut args = vec!["list".to_string()];
            if let Some(s) = status {
                args.push("--status".into());
                args.push(s.into());
            }
            if let Some(t) = ty {
                args.push("--type".into());
                args.push(t.into());
            }
            run_aida(cfg, &args).await
        }
        Err(e) => Err(e),
    }
}

pub async fn aida_show(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let id = input
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'id'".into()))?;
    if !is_spec_id(id) {
        return Err(ToolError::BadInput(format!(
            "id does not look like a SPEC-ID: {id}"
        )));
    }
    match try_mcp(cfg, "show_requirement", json!({ "id": id })).await {
        Ok(text) => Ok(text),
        Err(ToolError::Execution(e)) if is_unavailable(&e) => {
            run_aida(cfg, &["show".into(), id.to_string()]).await
        }
        Err(e) => Err(e),
    }
}

pub async fn aida_search(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let q = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'query'".into()))?;
    if q.starts_with('-') {
        return Err(ToolError::BadInput(
            "query may not start with '-' (would be interpreted as a flag)".into(),
        ));
    }
    match try_mcp(cfg, "search_requirements", json!({ "query": q })).await {
        Ok(text) => Ok(text),
        Err(ToolError::Execution(e)) if is_unavailable(&e) => {
            run_aida(cfg, &["search".into(), q.to_string()]).await
        }
        Err(e) => Err(e),
    }
}

/// `aida_history` is CLI-only — AIDA's MCP server does not expose a
/// `history` tool. We still keep the same tool name so the model's
/// contract is stable.
pub async fn aida_history(cfg: &ServerConfig, _input: &Value) -> Result<String, ToolError> {
    run_aida(cfg, &["history".into()]).await
}

pub async fn aida_resource(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let action = input
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'action'".into()))?;
    let client = McpClient::global(&cfg.mcp_command, &cfg.mcp_args, &cfg.repo_root)
        .await
        .map_err(|e| {
            ToolError::Execution(format!(
                "aida_resource needs the AIDA MCP server, which is not available ({e}). \
             There is no CLI equivalent for this tool."
            ))
        })?;
    match action {
        "list" => {
            let resources = client
                .list_resources()
                .await
                .map_err(|e| ToolError::Execution(format!("resources/list: {e}")))?;
            if resources.is_empty() {
                return Ok("(no resources)".into());
            }
            let mut out = String::new();
            for r in resources {
                out.push_str(&format!("- {}", r.uri));
                if let Some(n) = &r.name {
                    out.push_str(&format!(" ({n})"));
                }
                if let Some(d) = &r.description {
                    out.push_str(&format!(" — {d}"));
                }
                out.push('\n');
            }
            Ok(out)
        }
        "read" => {
            let uri = input
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::BadInput("missing 'uri' for action='read'".into()))?;
            client
                .read_resource(uri)
                .await
                .map_err(|e| ToolError::Execution(format!("resources/read {uri}: {e}")))
        }
        other => Err(ToolError::BadInput(format!(
            "unknown action {other:?} (expected 'list' or 'read')"
        ))),
    }
}

// --------------------------------------------------------------------------
// Transport helpers
// --------------------------------------------------------------------------

/// Attempt the call over MCP. Maps transport-liveness failures to a
/// recognisable `ToolError::Execution(...)` so the caller can detect
/// them and fall back to the CLI. Other errors propagate verbatim.
async fn try_mcp(cfg: &ServerConfig, tool: &str, args: Value) -> Result<String, ToolError> {
    let client = McpClient::global(&cfg.mcp_command, &cfg.mcp_args, &cfg.repo_root)
        .await
        .map_err(|e| match e {
            McpError::Unavailable(s) => ToolError::Execution(unavailable_marker(&s)),
            McpError::Closed | McpError::Timeout => {
                ToolError::Execution(unavailable_marker(&e.to_string()))
            }
            other => ToolError::Execution(format!("mcp: {other}")),
        })?;
    client.call_tool(tool, args).await.map_err(|e| match e {
        McpError::Unavailable(s) => ToolError::Execution(unavailable_marker(&s)),
        McpError::Closed | McpError::Timeout => {
            ToolError::Execution(unavailable_marker(&format!("mcp tool {tool}: {e}")))
        }
        other => ToolError::Execution(format!("mcp tool {tool}: {other}")),
    })
}

const UNAVAILABLE_PREFIX: &str = "mcp-unavailable:";

fn unavailable_marker(reason: &str) -> String {
    format!("{UNAVAILABLE_PREFIX} {reason}")
}

fn is_unavailable(error_msg: &str) -> bool {
    error_msg.starts_with(UNAVAILABLE_PREFIX)
}

async fn run_aida(cfg: &ServerConfig, args: &[String]) -> Result<String, ToolError> {
    let out = Command::new("aida")
        .args(args)
        .current_dir(&cfg.repo_root)
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("spawn aida: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(ToolError::Execution(format!(
            "aida exited with {}: {}",
            out.status,
            stderr.trim()
        )));
    }
    let mut text = stdout;
    if !stderr.trim().is_empty() {
        text.push_str("\n\n[stderr]\n");
        text.push_str(&stderr);
    }
    Ok(text)
}

fn is_simple_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_spec_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && s.contains('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_status_token() {
        assert!(!is_simple_token("approved; rm -rf"));
        assert!(!is_simple_token("--status"));
        assert!(is_simple_token("approved"));
        assert!(is_simple_token("in-progress"));
    }

    #[test]
    fn spec_id_validator() {
        assert!(is_spec_id("EPIC-1"));
        assert!(is_spec_id("BUG-1-017"));
        assert!(!is_spec_id("epic1")); // no hyphen
        assert!(!is_spec_id("EPIC 1")); // space
        assert!(!is_spec_id("EPIC-1; ls"));
    }

    #[test]
    fn unavailable_marker_roundtrip() {
        let m = unavailable_marker("spawn failed");
        assert!(is_unavailable(&m));
        assert!(!is_unavailable("mcp tool list_requirements: timed out"));
    }
}
