// trace:EPIC-29 | ai:claude
//
// Chart tools for the agent surface. Three V1 tools, each reading the
// AIDA substrate at `cfg.repo_root` and producing a complete SVG.
//
//   * `chart_status`  — status distribution donut (always renderable).
//   * `chart_sprint`  — burn-down + burn-up + velocity for one sprint.
//                       Defaults to the active sprint, or the most
//                       recent sprint with dates.
//   * `chart_feature` — feature-progress horizontal bars.
//
// Each tool's TEXT output to the model is a short text summary
// (so the LLM has a cheap, structured handle on what it just produced).
// The full SVG flows out-of-band via the `chart` SSE artifact event
// (see backends/anthropic.rs), so it lands in the chat UI without
// being copy-pasted through the model.

use std::path::Path;

use serde_json::{json, Value};

use super::{Tool, ToolError};
use crate::server::charts::{
    data::{
        compute_burndown, compute_burnup, compute_feature_progress, compute_status_counts,
        compute_velocity, SprintState,
    },
    render_burndown_svg, render_burnup_svg, render_feature_progress_svg, render_status_svg,
    render_velocity_svg, AidaStore, Sprint,
};
use crate::server::config::ServerConfig;

/// One rendered chart, packaged for the frontend.
///
/// `kind` is one of `status` / `burndown` / `burnup` / `velocity` /
/// `feature_progress`; the frontend uses it as the CSS hook and for
/// stable rendering identity.
#[derive(Debug, Clone)]
pub struct ChartArtifact {
    pub kind: &'static str,
    pub svg: String,
    pub caption: Option<String>,
}

/// Multi-artifact result from a single chart-tool invocation. Each
/// artifact renders as its own panel in the UI. `summary` is the
/// text returned to the model so it doesn't see the raw SVG bytes.
#[derive(Debug, Clone)]
pub struct ChartToolResult {
    pub artifacts: Vec<ChartArtifact>,
    pub summary: String,
}

// =========================================================================
// Tool specs
// =========================================================================

pub fn chart_status_spec() -> Tool {
    Tool {
        name: "chart_status",
        description: "Render a status-distribution donut chart over every requirement in this \
            project's AIDA substrate. Always renderable. Use this when the user asks 'where are \
            we', 'what's the status breakdown', or any high-level project-health question.",
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

pub fn chart_sprint_spec() -> Tool {
    Tool {
        name: "chart_sprint",
        description: "Render burn-down + burn-up + velocity charts for a sprint. With no \
            arguments, picks the active sprint (else the most-recently-numbered sprint with \
            dates). Pass `sprint_id` to target a specific sprint (e.g. `SPRINT-3`). Returns three \
            chart artifacts.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "sprint_id": {
                    "type": "string",
                    "description": "Optional SPEC-ID of the sprint, e.g. SPRINT-3."
                }
            }
        }),
    }
}

pub fn chart_feature_spec() -> Tool {
    Tool {
        name: "chart_feature",
        description: "Render a feature-progress chart — horizontal bar per `feature` field, \
            showing completed vs total. Use this when the user asks 'how is feature X', or \
            'what feature areas are lagging'.",
        input_schema: json!({
            "type": "object",
            "properties": {}
        }),
    }
}

// =========================================================================
// Executors
// =========================================================================

pub async fn chart_status(
    cfg: &ServerConfig,
    _input: &Value,
) -> Result<ChartToolResult, ToolError> {
    let store = load_store(&cfg.repo_root)?;
    let items: Vec<_> = store.items.iter().collect();
    let counts = compute_status_counts(&items);
    let svg = render_status_svg(&counts)
        .map_err(|e| ToolError::Execution(format!("render status svg: {e}")))?;
    let breakdown = counts
        .buckets
        .iter()
        .map(|(s, n)| format!("{s}={n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let summary = format!(
        "Rendered status distribution chart: {} requirements across {} status buckets ({breakdown}).",
        counts.total,
        counts.buckets.len()
    );
    Ok(ChartToolResult {
        artifacts: vec![ChartArtifact {
            kind: "status",
            svg,
            caption: Some(format!("{} requirements total", counts.total)),
        }],
        summary,
    })
}

pub async fn chart_sprint(
    cfg: &ServerConfig,
    input: &Value,
) -> Result<ChartToolResult, ToolError> {
    let store = load_store(&cfg.repo_root)?;
    let sprints = store.sprints();
    if sprints.is_empty() {
        // Render empty-state charts so the operator sees what's missing.
        let empty_bd = render_burndown_svg(&[]).unwrap_or_default();
        let empty_bu = render_burnup_svg(&[]).unwrap_or_default();
        let empty_v = render_velocity_svg(&[]).unwrap_or_default();
        return Ok(ChartToolResult {
            artifacts: vec![
                ChartArtifact { kind: "burndown", svg: empty_bd, caption: None },
                ChartArtifact { kind: "burnup", svg: empty_bu, caption: None },
                ChartArtifact { kind: "velocity", svg: empty_v, caption: None },
            ],
            summary: "No sprints in this project's substrate. Sprint charts cannot be \
                rendered until at least one requirement of type=Sprint with start_date + \
                end_date custom fields exists."
                .into(),
        });
    }

    // Sprint selection.
    let chosen: &Sprint<'_> = if let Some(id) = input.get("sprint_id").and_then(|v| v.as_str()) {
        match sprints.iter().find(|s| s.req.spec_id == id) {
            Some(s) => s,
            None => {
                return Err(ToolError::BadInput(format!(
                    "no sprint with spec_id {id}. Available: {}",
                    sprints
                        .iter()
                        .map(|s| s.req.spec_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    } else {
        let today = today_yyyy_mm_dd();
        sprints
            .iter()
            .find(|s| s.state(&today) == SprintState::Active)
            .or_else(|| {
                sprints
                    .iter()
                    .filter(|s| s.start_date.is_some() && s.end_date.is_some())
                    .max_by_key(|s| s.sprint_number)
            })
            .unwrap_or_else(|| &sprints[0])
    };

    let sprint_items = store.sprint_items(chosen);
    let (start, end) = (
        chosen.start_date.as_deref().unwrap_or(""),
        chosen.end_date.as_deref().unwrap_or(""),
    );

    let bd_points = compute_burndown(&sprint_items, start, end);
    let bu_points = compute_burnup(&sprint_items, start, end);
    let bd_svg = render_burndown_svg(&bd_points)
        .map_err(|e| ToolError::Execution(format!("render burndown: {e}")))?;
    let bu_svg = render_burnup_svg(&bu_points)
        .map_err(|e| ToolError::Execution(format!("render burnup: {e}")))?;

    // Velocity always reflects all sprints (gives context).
    let velocity_points = compute_velocity(&sprints, |s| store.sprint_items(s));
    let v_svg = render_velocity_svg(&velocity_points)
        .map_err(|e| ToolError::Execution(format!("render velocity: {e}")))?;

    let completed = sprint_items
        .iter()
        .filter(|i| i.status == "Completed")
        .count();
    let total = sprint_items.len();
    let caption_bd = Some(format!(
        "{} of {} items completed",
        completed, total
    ));
    let summary = format!(
        "Rendered burn-down + burn-up + velocity for {} ({}–{}): {}/{} items completed, {} sprint(s) total.",
        chosen.req.spec_id,
        start,
        end,
        completed,
        total,
        sprints.len()
    );
    Ok(ChartToolResult {
        artifacts: vec![
            ChartArtifact {
                kind: "burndown",
                svg: bd_svg,
                caption: caption_bd,
            },
            ChartArtifact {
                kind: "burnup",
                svg: bu_svg,
                caption: None,
            },
            ChartArtifact {
                kind: "velocity",
                svg: v_svg,
                caption: Some(format!("across {} sprint(s)", sprints.len())),
            },
        ],
        summary,
    })
}

pub async fn chart_feature(
    cfg: &ServerConfig,
    _input: &Value,
) -> Result<ChartToolResult, ToolError> {
    let store = load_store(&cfg.repo_root)?;
    let items: Vec<_> = store.items.iter().collect();
    let rows = compute_feature_progress(&items);
    let svg = render_feature_progress_svg(&rows)
        .map_err(|e| ToolError::Execution(format!("render feature progress: {e}")))?;
    let head: Vec<String> = rows
        .iter()
        .take(3)
        .map(|r| format!("{} {}/{}", r.feature, r.completed, r.total))
        .collect();
    let summary = format!(
        "Rendered feature-progress chart with {} feature group(s). Top: {}.",
        rows.len(),
        head.join("; ")
    );
    Ok(ChartToolResult {
        artifacts: vec![ChartArtifact {
            kind: "feature_progress",
            svg,
            caption: Some(format!("{} feature group(s)", rows.len())),
        }],
        summary,
    })
}

// =========================================================================
// Helpers
// =========================================================================

fn load_store(repo_root: &Path) -> Result<AidaStore, ToolError> {
    if !AidaStore::has_store(repo_root) {
        return Err(ToolError::Execution(format!(
            "no `.aida-store/` directory under {}. Charts read AIDA's distributed \
             store directly; point AIDA_CHAT_REPO_ROOT at an AIDA-initialized project.",
            repo_root.display()
        )));
    }
    AidaStore::load(repo_root)
        .map_err(|e| ToolError::Execution(format!("load aida store: {e}")))
}

fn today_yyyy_mm_dd() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chart_status_against_aida_core() {
        // Skip the test gracefully if the AIDA core checkout isn't
        // available — local-dev pleasantness, doesn't hide failures
        // on the dev box (which always has /home/joe/ai/aida).
        let aida = Path::new("/home/joe/ai/aida");
        if !AidaStore::has_store(aida) {
            eprintln!("skipping: no /home/joe/ai/aida/.aida-store on this machine");
            return;
        }
        let cfg = crate::server::config::ServerConfig {
            backend: crate::server::config::Backend::Anthropic,
            anthropic_api_key: Some("x".into()),
            model: "x".into(),
            repo_root: aida.to_path_buf(),
            max_tool_iterations: 1,
            max_output_tokens: 1,
            max_read_bytes: 1,
            session_ttl: std::time::Duration::from_secs(1),
            mcp_command: std::path::PathBuf::from("aida"),
            mcp_args: vec!["mcp-serve".to_string()],
        };
        let out = chart_status(&cfg, &json!({})).await.unwrap();
        assert_eq!(out.artifacts.len(), 1);
        assert_eq!(out.artifacts[0].kind, "status");
        assert!(out.artifacts[0].svg.starts_with("<svg"));
        assert!(out.summary.contains("Rendered status distribution"));
    }
}
