// trace:STORY-4 | ai:claude
//
// The tool surface the agent is allowed to invoke. Every tool here is
// confined: file/grep tools are confined to `repo_root`, `aida_*`
// tools shell out only to a fixed allowlist of subcommands.

pub mod aida;
pub mod fs;
pub mod grep;
pub mod traces;

use serde_json::Value;
use thiserror::Error;

use crate::server::config::ServerConfig;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("bad input: {0}")]
    BadInput(String),
    #[error("not allowed: {0}")]
    NotAllowed(String),
    #[error("io: {0}")]
    Io(String),
    #[error("execution: {0}")]
    Execution(String),
}

/// One tool, ready to publish to Anthropic and dispatch on.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub fn all_tool_specs() -> Vec<Tool> {
    vec![
        fs::read_file_spec(),
        fs::list_directory_spec(),
        grep::grep_repo_spec(),
        traces::find_traces_spec(),
        aida::aida_list_spec(),
        aida::aida_show_spec(),
        aida::aida_search_spec(),
        aida::aida_history_spec(),
        aida::aida_resource_spec(),
        aida::aida_comment_add_spec(),
        aida::aida_add_spec(),
        // trace:STORY-24 | ai:agy
        aida::aida_ultraplan_spec(),
    ]
}

/// Dispatch a tool_use block by name + raw JSON input. Returns the
/// text the agent will see as `tool_result.content`.
pub async fn dispatch(cfg: &ServerConfig, name: &str, input: &Value) -> Result<String, ToolError> {
    match name {
        "read_file" => fs::read_file(cfg, input).await,
        "list_directory" => fs::list_directory(cfg, input).await,
        "grep_repo" => grep::grep_repo(cfg, input).await,
        "find_traces" => traces::find_traces(cfg, input).await,
        "aida_list" => aida::aida_list(cfg, input).await,
        "aida_show" => aida::aida_show(cfg, input).await,
        "aida_search" => aida::aida_search(cfg, input).await,
        "aida_history" => aida::aida_history(cfg, input).await,
        "aida_resource" => aida::aida_resource(cfg, input).await,
        "aida_comment_add" => aida::aida_comment_add(cfg, input).await,
        "aida_add" => aida::aida_add(cfg, input).await,
        // trace:STORY-24 | ai:agy
        "aida_ultraplan" => aida::aida_ultraplan(cfg, input).await,
        other => Err(ToolError::NotAllowed(format!("unknown tool {other}"))),
    }
}

/// A short, human-friendly preview of a tool input — used in the UI
/// badge so the user can see what the agent looked at.
pub fn preview_input(input: &Value) -> String {
    let raw = input.to_string();
    if raw.len() <= 120 {
        raw
    } else {
        format!("{}…", &raw[..120])
    }
}
