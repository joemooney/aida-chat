// trace:STORY-4 | ai:claude
//
// AIDA query tools. We never invoke a shell — every call is
// Command::new("aida").args(&[...]) with explicit args. The subcommand
// is a fixed allowlist (`list`, `show`, `search`, `history`) and the
// arguments are passed without shell expansion, so the model can't
// inject extra flags or pipe to another binary.

use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;

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

pub async fn aida_list(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let mut args = vec!["list".to_string()];
    if let Some(s) = input.get("status").and_then(|v| v.as_str()) {
        if !is_simple_token(s) {
            return Err(ToolError::BadInput("invalid status".into()));
        }
        args.push("--status".into());
        args.push(s.into());
    }
    if let Some(t) = input.get("type").and_then(|v| v.as_str()) {
        if !is_simple_token(t) {
            return Err(ToolError::BadInput("invalid type".into()));
        }
        args.push("--type".into());
        args.push(t.into());
    }
    run_aida(cfg, &args).await
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
    run_aida(cfg, &["show".into(), id.to_string()]).await
}

pub async fn aida_search(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let q = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'query'".into()))?;
    // Pass the query as a positional arg; aida treats it as a string.
    // Reject anything that looks like a flag, just to be safe.
    if q.starts_with('-') {
        return Err(ToolError::BadInput(
            "query may not start with '-' (would be interpreted as a flag)".into(),
        ));
    }
    run_aida(cfg, &["search".into(), q.to_string()]).await
}

pub async fn aida_history(cfg: &ServerConfig, _input: &Value) -> Result<String, ToolError> {
    run_aida(cfg, &["history".into()]).await
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
}
