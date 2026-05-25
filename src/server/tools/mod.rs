// trace:STORY-4 | ai:claude
//
// The tool surface the agent is allowed to invoke. Every tool here is
// confined: file/grep tools are confined to `repo_root`, `aida_*`
// tools shell out only to a fixed allowlist of subcommands.

pub mod aida;
pub mod charts;
pub mod drift;
pub mod fs;
pub mod grep;
pub mod memory;
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
        memory::write_memory_spec(),
        aida::aida_list_spec(),
        aida::aida_show_spec(),
        aida::aida_search_spec(),
        aida::aida_history_spec(),
        aida::aida_resource_spec(),
        aida::aida_comment_add_spec(),
        aida::aida_add_spec(),
        // trace:EPIC-29 | ai:claude — V1 chart tools
        charts::chart_status_spec(),
        charts::chart_sprint_spec(),
        charts::chart_feature_spec(),
        drift::verify_trace_drift_spec(),
        // trace:STORY-24 | ai:agy — (verify when STORY-24 lands)
        // trace:EPIC-29 | ai:claude — V2 chart tools
        charts::chart_cfd_spec(),
        charts::chart_dep_graph_spec(),
        charts::chart_cycle_time_spec(),
    ]
}

/// True iff `name` is one of the chart-rendering tools that emits
/// SVG artifacts out-of-band. The anthropic backend dispatches these
/// via `dispatch_chart` instead of the regular `dispatch` path so the
/// artifacts can flow through SSE without round-tripping the SVG
/// through the model.
pub fn is_chart_tool(name: &str) -> bool {
    matches!(
        name,
        "chart_status"
            | "chart_sprint"
            | "chart_feature"
            | "chart_cfd"
            | "chart_dep_graph"
            | "chart_cycle_time"
    )
}

/// Chart-specific dispatch — returns a `ChartToolResult` carrying both
/// a model-visible summary and a vec of `ChartArtifact`s for the UI.
pub async fn dispatch_chart(
    cfg: &ServerConfig,
    name: &str,
    input: &Value,
) -> Result<charts::ChartToolResult, ToolError> {
    match name {
        "chart_status" => charts::chart_status(cfg, input).await,
        "chart_sprint" => charts::chart_sprint(cfg, input).await,
        "chart_feature" => charts::chart_feature(cfg, input).await,
        // trace:EPIC-29 | ai:claude — V2
        "chart_cfd" => charts::chart_cfd(cfg, input).await,
        "chart_dep_graph" => charts::chart_dep_graph(cfg, input).await,
        "chart_cycle_time" => charts::chart_cycle_time(cfg, input).await,
        other => Err(ToolError::NotAllowed(format!("unknown chart tool {other}"))),
    }
}

/// Dispatch a tool_use block by name + raw JSON input. Returns the
/// text the agent will see as `tool_result.content`.
pub async fn dispatch(cfg: &ServerConfig, name: &str, input: &Value) -> Result<String, ToolError> {
    match name {
        "read_file" => fs::read_file(cfg, input).await,
        "list_directory" => fs::list_directory(cfg, input).await,
        "grep_repo" => grep::grep_repo(cfg, input).await,
        "find_traces" => traces::find_traces(cfg, input).await,
        "write_memory" => memory::write_memory(cfg, input).await,
        "aida_list" => aida::aida_list(cfg, input).await,
        "aida_show" => aida::aida_show(cfg, input).await,
        "aida_search" => aida::aida_search(cfg, input).await,
        "aida_history" => aida::aida_history(cfg, input).await,
        "aida_resource" => aida::aida_resource(cfg, input).await,
        "aida_comment_add" => aida::aida_comment_add(cfg, input).await,
        "aida_add" => aida::aida_add(cfg, input).await,
        "verify_trace_drift" => drift::verify_trace_drift(cfg, input).await,
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
