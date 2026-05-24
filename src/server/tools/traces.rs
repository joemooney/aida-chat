// trace:EPIC-16 | ai:claude
//
// `find_traces`: locate every `// trace:SPEC-ID ...` comment that
// references a given SPEC-ID. The differentiation surface — turns the
// spec graph from "list of names" into "actual file:line code".
//
// A trace comment can list multiple IDs separated by spaces, e.g.
//   // trace:STORY-3 STORY-15 | ai:claude
// so we match SPEC-ID at any position in the ID list, using a regex
// of the form `\btrace:(\S+\s+)*<ESCAPED-ID>\b`.

use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;

pub fn find_traces_spec() -> Tool {
    Tool {
        name: "find_traces",
        description: "Find every `// trace:SPEC-ID …` comment in the repository that references \
            a given SPEC-ID. Returns 'path:line:comment-text' lines. This is the fastest way to \
            answer 'where is EPIC-12 implemented?' — preferred over grep_repo for that question.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "spec_id": {
                    "type": "string",
                    "description": "SPEC-ID such as EPIC-1, STORY-2, BUG-17"
                },
                "path_glob": {
                    "type": "string",
                    "description": "Optional glob to scope the search, e.g. 'src/**' or '*.rs'."
                }
            },
            "required": ["spec_id"]
        }),
    }
}

pub async fn find_traces(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let spec_id = input
        .get("spec_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'spec_id'".into()))?;
    if !is_spec_id(spec_id) {
        return Err(ToolError::BadInput(format!(
            "spec_id does not look like a SPEC-ID: {spec_id}"
        )));
    }

    let pattern = format!(r"\btrace:(\S+\s+)*{}\b", regex_escape(spec_id));

    let mut cmd = Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--glob")
        .arg("!.git")
        .arg("--glob")
        .arg("!.aida-store");
    if let Some(g) = input.get("path_glob").and_then(|v| v.as_str()) {
        if g.starts_with('-') {
            return Err(ToolError::BadInput(
                "path_glob may not start with '-'".into(),
            ));
        }
        cmd.arg("--glob").arg(g);
    }
    cmd.arg("--").arg(&pattern).arg(".");
    cmd.current_dir(&cfg.repo_root);

    let out = cmd
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("spawn rg: {e}. Is ripgrep installed?")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    match out.status.code() {
        Some(0) => {
            // ripgrep already prints `path:line:text`; sort for stability.
            let mut lines: Vec<&str> = stdout.lines().collect();
            lines.sort();
            Ok(lines.join("\n"))
        }
        Some(1) => Ok(format!("(no trace comments reference {spec_id})")),
        Some(c) => Err(ToolError::Execution(format!(
            "rg exited {c}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        None => Err(ToolError::Execution("rg killed by signal".into())),
    }
}

fn is_spec_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && s.contains('-')
}

/// Escape regex metachars in a SPEC-ID. After `is_spec_id` the only
/// metachar that can appear is `-`, which is not a metachar outside a
/// character class — but escape it defensively anyway in case the
/// validator is ever relaxed.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::server::config::{Backend, ServerConfig};

    fn fixture_cfg(root: PathBuf) -> ServerConfig {
        ServerConfig {
            backend: Backend::Anthropic,
            anthropic_api_key: Some("x".into()),
            model: "claude-sonnet-4-6".into(),
            repo_root: root,
            max_tool_iterations: 1,
            max_output_tokens: 1,
            max_read_bytes: 1,
            session_ttl: Duration::from_secs(1),
            mcp_command: PathBuf::from("aida"),
            mcp_args: vec!["mcp-serve".to_string()],
        }
    }

    #[tokio::test]
    async fn finds_single_id_and_multi_id_traces() {
        let tmp = tempdir_for_test("traces-multi");
        std::fs::write(
            tmp.join("a.rs"),
            "// trace:EPIC-1 | ai:claude\nfn a() {}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("b.rs"),
            "// trace:STORY-3 EPIC-1 | ai:claude\nfn b() {}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("c.rs"),
            "// trace:STORY-3 | ai:claude\nfn c() {}\n",
        )
        .unwrap();

        let cfg = fixture_cfg(tmp.clone());
        let out =
            find_traces(&cfg, &json!({"spec_id": "EPIC-1"}))
                .await
                .unwrap();
        assert!(out.contains("a.rs:1:"), "missing a.rs match in {out:?}");
        assert!(out.contains("b.rs:1:"), "missing b.rs match in {out:?}");
        assert!(!out.contains("c.rs"), "c.rs should not match: {out:?}");
    }

    #[tokio::test]
    async fn empty_result_message_when_no_matches() {
        let tmp = tempdir_for_test("traces-empty");
        std::fs::write(tmp.join("x.rs"), "fn x() {}\n").unwrap();
        let cfg = fixture_cfg(tmp);
        let out =
            find_traces(&cfg, &json!({"spec_id": "EPIC-99"}))
                .await
                .unwrap();
        assert!(out.starts_with("(no trace comments"), "{out:?}");
    }

    #[tokio::test]
    async fn rejects_bad_spec_id() {
        let cfg = fixture_cfg(std::env::temp_dir());
        let err = find_traces(&cfg, &json!({"spec_id": "EPIC 1"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::BadInput(_)));
    }

    fn tempdir_for_test(label: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("aida-chat-{label}-{pid}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
