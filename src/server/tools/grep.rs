// trace:STORY-4 | ai:claude
//
// grep_repo: scoped ripgrep wrapper. We invoke `rg` directly via
// Command (no shell), pass the pattern as an explicit arg, and the
// optional path_glob via `--glob` (not concatenated into a path).

use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;

pub fn grep_repo_spec() -> Tool {
    Tool {
        name: "grep_repo",
        description: "Search this repository for a pattern using ripgrep. Returns lines of the \
            form 'path:line:matched-text'. Useful for locating where a symbol, string, or \
            concept lives before reading the file. Honors .gitignore by default.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "ripgrep regex pattern. Plain strings work too."
                },
                "path_glob": {
                    "type": "string",
                    "description": "Optional glob to scope the search, e.g. '*.rs' or 'docs/**'."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Cap on returned matches (default 200)."
                }
            },
            "required": ["pattern"]
        }),
    }
}

pub async fn grep_repo(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let pattern = input
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'pattern'".into()))?;
    if pattern.is_empty() {
        return Err(ToolError::BadInput("empty pattern".into()));
    }
    let max_results = input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .clamp(1, 2000) as usize;
    let path_glob = input.get("path_glob").and_then(|v| v.as_str());

    let mut cmd = Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count")
        .arg(max_results.to_string())
        // Skip the orphan-store data and .git outright — aida_* tools
        // are the supported way to read requirements.
        .arg("--glob")
        .arg("!.git")
        .arg("--glob")
        .arg("!.aida-store");
    if let Some(g) = path_glob {
        if g.starts_with('-') {
            return Err(ToolError::BadInput(
                "path_glob may not start with '-'".into(),
            ));
        }
        cmd.arg("--glob").arg(g);
    }
    cmd.arg("--").arg(pattern).arg(".");
    cmd.current_dir(&cfg.repo_root);

    let out = cmd
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("spawn rg: {e}. Is ripgrep installed?")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // ripgrep exits with code 1 when there are no matches; that's not an error.
    match out.status.code() {
        Some(0) => Ok(stdout),
        Some(1) => Ok("(no matches)".into()),
        Some(c) => Err(ToolError::Execution(format!(
            "rg exited {c}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        None => Err(ToolError::Execution("rg killed by signal".into())),
    }
}
