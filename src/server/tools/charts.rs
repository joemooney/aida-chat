// trace:EPIC-29 | ai:agy
//
// Agile chart tools for aida-chat. The data reducers and SVG shapes are
// ports of ~/ai/aida/aida-web-react/src/lib/sprint-utils.ts and the
// hand-authored chart components under src/components/**/charts.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;

use chrono::{NaiveDate, Utc};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::messages::ChartArtifact;
use crate::server::config::ServerConfig;
use crate::server::tools::{Tool, ToolError};

const CHART_MARKER: &str = "AIDA_CHART_ARTIFACT";
const W: f64 = 920.0;
const PAD: PlotPad = PlotPad {
    top: 26.0,
    right: 22.0,
    bottom: 34.0,
    left: 42.0,
};

#[derive(Clone, Copy)]
struct PlotPad {
    top: f64,
    right: f64,
    bottom: f64,
    left: f64,
}

#[derive(Debug, Clone)]
struct Requirement {
    id: String,
    spec_id: String,
    title: String,
    status: String,
    req_type: String,
    feature: String,
    created_at: String,
    modified_at: String,
    yaml_path: String,
    weight: i64,
    custom_fields: HashMap<String, String>,
    relationships: Vec<Relationship>,
}

#[derive(Debug, Clone)]
struct Relationship {
    rel_type: String,
    target_id: String,
}

#[derive(Debug, Clone, PartialEq)]
struct BurndownPoint {
    date: String,
    remaining: f64,
    ideal: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct BurnupPoint {
    date: String,
    completed: f64,
    scope: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct VelocityPoint {
    sprint_label: String,
    points: i64,
}

pub fn chart_status_spec() -> Tool {
    Tool {
        name: "chart_status",
        description: "Render an inline status-distribution chart for the current AIDA project. Use when the user asks for status mix, portfolio overview, blocked/open work, or agile metrics.",
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

pub fn chart_sprint_spec() -> Tool {
    Tool {
        name: "chart_sprint",
        description: "Render inline sprint burndown, burn-up, and velocity charts from AIDA sprint requirements. Optional sprint_id selects a sprint; otherwise the active or most recent sprint is used.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "sprint_id": {"type": "string", "description": "Optional sprint SPEC-ID such as SPRINT-3."}
            },
            "additionalProperties": false
        }),
    }
}

pub fn chart_feature_spec() -> Tool {
    Tool {
        name: "chart_feature",
        description: "Render inline feature or epic progress bars from AIDA requirements. Optional epic_id filters work tagged or parented to an epic.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "epic_id": {"type": "string", "description": "Optional epic SPEC-ID such as EPIC-29."}
            },
            "additionalProperties": false
        }),
    }
}

pub async fn chart_status(cfg: &ServerConfig, _input: &Value) -> Result<String, ToolError> {
    let data = load_dataset(&cfg.repo_root, false)?;
    let artifact = status_artifact(&data.requirements);
    Ok(encode_artifact(&artifact))
}

pub async fn chart_sprint(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let sprint_id = input
        .get("sprint_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let data = load_dataset(&cfg.repo_root, true)?;
    let artifact = sprint_artifact(&data.requirements, sprint_id);
    Ok(encode_artifact(&artifact))
}

pub async fn chart_feature(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let epic_id = input
        .get("epic_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let data = load_dataset(&cfg.repo_root, false)?;
    let artifact = feature_artifact(&data.requirements, epic_id);
    Ok(encode_artifact(&artifact))
}

pub fn extract_chart_artifact(output: &str) -> Option<ChartArtifact> {
    let json = output.strip_prefix(CHART_MARKER)?.trim();
    serde_json::from_str(json).ok()
}

fn encode_artifact(artifact: &ChartArtifact) -> String {
    format!(
        "{CHART_MARKER}\n{}",
        serde_json::to_string(artifact).unwrap_or_else(|_| "{}".into())
    )
}

struct Dataset {
    requirements: Vec<Requirement>,
}

fn load_dataset(repo_root: &Path, with_yaml: bool) -> Result<Dataset, ToolError> {
    let db = repo_root.join(".aida/cache.db");
    if !db.is_file() {
        return Err(ToolError::Execution(format!(
            "AIDA cache not found at {}",
            db.display()
        )));
    }
    let conn =
        Connection::open(&db).map_err(|e| ToolError::Execution(format!("open cache: {e}")))?;
    let mut stmt = conn
        .prepare(
            "select id, coalesce(spec_id, agreed_id, id), title, status, req_type, feature, \
             created_at, modified_at, yaml_path from requirements_cache where archived = 0",
        )
        .map_err(|e| ToolError::Execution(format!("query cache: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Requirement {
                id: row.get(0)?,
                spec_id: row.get(1)?,
                title: row.get(2)?,
                status: row.get(3)?,
                req_type: row.get(4)?,
                feature: row.get(5)?,
                created_at: row.get(6)?,
                modified_at: row.get(7)?,
                yaml_path: row.get(8)?,
                weight: 1,
                custom_fields: HashMap::new(),
                relationships: vec![],
            })
        })
        .map_err(|e| ToolError::Execution(format!("read cache rows: {e}")))?;
    let mut requirements = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ToolError::Execution(format!("read cache row: {e}")))?;
    if with_yaml {
        for req in &mut requirements {
            if let Ok(yaml) = read_store_yaml(repo_root, &req.yaml_path) {
                enrich_from_yaml(req, &yaml);
            }
        }
    }
    Ok(Dataset { requirements })
}

fn read_store_yaml(repo_root: &Path, yaml_path: &str) -> Result<String, ToolError> {
    let store = repo_root.join(".aida-store");
    let git_dir = store.join(".git");
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(&git_dir)
        .arg("--work-tree")
        .arg(&store)
        .arg("show")
        .arg(format!("HEAD:{yaml_path}"))
        .output()
        .map_err(|e| ToolError::Execution(format!("git show {yaml_path}: {e}")))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|e| ToolError::Execution(format!("utf8 {yaml_path}: {e}")));
    }
    std::fs::read_to_string(store.join(yaml_path))
        .map_err(|e| ToolError::Execution(format!("read {yaml_path}: {e}")))
}

fn enrich_from_yaml(req: &mut Requirement, yaml: &str) {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return;
    };
    if let Some(weight) = get_i64(&value, "weight") {
        req.weight = weight.max(0);
    }
    if let Some(fields) = get_map(&value, "custom_fields") {
        for (k, v) in fields {
            if let Some(key) = k.as_str() {
                req.custom_fields
                    .insert(key.to_string(), scalar_to_string(v));
            }
        }
    }
    if let Some(rels) = get_seq(&value, "relationships") {
        req.relationships = rels
            .iter()
            .filter_map(|rel| {
                Some(Relationship {
                    rel_type: relationship_type(rel)?,
                    target_id: get_str(rel, "target_id")?.to_string(),
                })
            })
            .collect();
    }
}

fn get_map<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a serde_yaml::Mapping> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_mapping()
}

fn get_seq<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a Vec<serde_yaml::Value>> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_sequence()
}

fn get_str<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_str()
}

fn get_i64(value: &serde_yaml::Value, key: &str) -> Option<i64> {
    value
        .as_mapping()?
        .get(serde_yaml::Value::String(key.into()))?
        .as_i64()
}

fn scalar_to_string(v: &serde_yaml::Value) -> String {
    if let Some(s) = v.as_str() {
        s.to_string()
    } else if let Some(i) = v.as_i64() {
        i.to_string()
    } else if let Some(b) = v.as_bool() {
        b.to_string()
    } else {
        String::new()
    }
}

fn relationship_type(rel: &serde_yaml::Value) -> Option<String> {
    let raw = rel
        .as_mapping()?
        .get(serde_yaml::Value::String("rel_type".into()))?;
    match raw {
        serde_yaml::Value::Tagged(tagged) => Some(
            tagged.tag.to_string().trim_start_matches('!').to_string()
                + " "
                + &scalar_to_string(&tagged.value),
        ),
        serde_yaml::Value::String(s) => Some(s.clone()),
        other => Some(format!("{other:?}")),
    }
}

fn status_artifact(requirements: &[Requirement]) -> ChartArtifact {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for req in requirements.iter().filter(|r| !is_stateless(&r.req_type)) {
        *counts.entry(req.status.clone()).or_default() += 1;
    }
    let total: usize = counts.values().sum();
    let svg = if total == 0 {
        empty_svg(
            "Status distribution",
            "No requirements found in this AIDA project.",
        )
    } else {
        status_svg(&counts, total)
    };
    ChartArtifact {
        title: "Status distribution".into(),
        summary: format!("{total} tracked requirements by status."),
        svg,
    }
}

fn sprint_artifact(requirements: &[Requirement], sprint_id: Option<&str>) -> ChartArtifact {
    let sprints = requirements
        .iter()
        .filter(|r| r.req_type.eq_ignore_ascii_case("sprint"))
        .collect::<Vec<_>>();
    let Some(sprint) = select_sprint(&sprints, sprint_id) else {
        return ChartArtifact {
            title: "Sprint charts".into(),
            summary: "No sprint requirements found.".into(),
            svg: empty_svg(
                "Sprint charts",
                "No sprint requirements found in this project.",
            ),
        };
    };
    let items = sprint_items(sprint, requirements);
    let start = sprint.custom_fields.get("start_date").cloned();
    let end = sprint.custom_fields.get("end_date").cloned();
    let velocity = compute_velocity_data(&sprints, requirements);
    let svg = if items.is_empty() {
        empty_svg(
            &format!("{} sprint charts", sprint.spec_id),
            "This sprint has no assigned work items.",
        )
    } else if let (Some(start), Some(end)) = (start.as_deref(), end.as_deref()) {
        let burndown = compute_burndown_data(&items, start, end);
        let burnup = compute_burnup_data(&items, start, end);
        sprint_svg(sprint, &items, &burndown, &burnup, &velocity)
    } else {
        empty_svg(
            &format!("{} sprint charts", sprint.spec_id),
            "Sprint dates are missing, so burn-down and burn-up cannot be plotted.",
        )
    };
    ChartArtifact {
        title: format!("{} sprint charts", sprint.spec_id),
        summary: format!("{} items assigned to {}.", items.len(), sprint.spec_id),
        svg,
    }
}

fn feature_artifact(requirements: &[Requirement], epic_id: Option<&str>) -> ChartArtifact {
    let mut groups: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for req in requirements.iter().filter(|r| !is_stateless(&r.req_type)) {
        if let Some(epic) = epic_id {
            let needle = epic.to_ascii_lowercase();
            let hay = format!("{} {}", req.title, req.feature).to_ascii_lowercase();
            if !hay.contains(&needle) && req.spec_id != epic {
                continue;
            }
        }
        let key = if req.feature.trim().is_empty() {
            "Uncategorized"
        } else {
            req.feature.as_str()
        };
        let entry = groups.entry(key.to_string()).or_default();
        entry.0 += 1;
        if is_done(&req.status) {
            entry.1 += 1;
        }
    }
    let total: usize = groups.values().map(|(t, _)| *t).sum();
    let title = epic_id
        .map(|id| format!("{id} feature progress"))
        .unwrap_or_else(|| "Feature progress".into());
    let svg = if total == 0 {
        empty_svg(&title, "No feature-progress data found for this selection.")
    } else {
        feature_svg(&title, &groups)
    };
    ChartArtifact {
        title,
        summary: format!("{total} requirements grouped by feature."),
        svg,
    }
}

fn select_sprint<'a>(
    sprints: &'a [&Requirement],
    sprint_id: Option<&str>,
) -> Option<&'a Requirement> {
    if let Some(id) = sprint_id {
        if let Some(s) = sprints.iter().copied().find(|s| s.spec_id == id) {
            return Some(s);
        }
    }
    let today = Utc::now().date_naive();
    sprints
        .iter()
        .copied()
        .find(|s| {
            let Some(start) = s
                .custom_fields
                .get("start_date")
                .and_then(|d| parse_date(d))
            else {
                return false;
            };
            let Some(end) = s.custom_fields.get("end_date").and_then(|d| parse_date(d)) else {
                return false;
            };
            start <= today && today <= end && !s.status.eq_ignore_ascii_case("Rejected")
        })
        .or_else(|| {
            sprints.iter().copied().max_by_key(|s| {
                s.custom_fields
                        .get("end_date")
                        .and_then(|d| parse_date(d))
                        .or_else(|| parse_date(&s.modified_at[..10.min(s.modified_at.len())]))
                        .or_else(|| parse_date(&s.created_at[..10.min(s.created_at.len())]))
                })
        })
}

fn sprint_items<'a>(sprint: &Requirement, requirements: &'a [Requirement]) -> Vec<&'a Requirement> {
    let by_uuid: HashMap<&str, &Requirement> =
        requirements.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut out = vec![];
    for rel in &sprint.relationships {
        if rel.rel_type.contains("sprint_contains") || rel.rel_type.contains("sprint_assignment") {
            if let Some(req) = by_uuid.get(rel.target_id.as_str()) {
                if !is_stateless(&req.req_type) {
                    out.push(*req);
                }
            }
        }
    }
    for req in requirements {
        if req.relationships.iter().any(|rel| {
            (rel.rel_type.contains("sprint_assignment") || rel.rel_type.contains("sprint_contains"))
                && (rel.target_id == sprint.id || rel.target_id == sprint.spec_id)
        }) && !out.iter().any(|existing| existing.id == req.id)
            && !is_stateless(&req.req_type)
        {
            out.push(req);
        }
    }
    out
}

fn compute_burndown_data(
    items: &[&Requirement],
    start_date: &str,
    end_date: &str,
) -> Vec<BurndownPoint> {
    let Some(start) = parse_date(start_date) else {
        return vec![];
    };
    let Some(end) = parse_date(end_date) else {
        return vec![];
    };
    let total = items.len() as f64;
    if total == 0.0 || start >= end {
        return vec![];
    }
    let days = (end - start).num_days().max(0);
    let mut completions: HashMap<String, usize> = HashMap::new();
    for item in items.iter().filter(|i| is_done(&i.status)) {
        let d = item
            .modified_at
            .get(0..10)
            .unwrap_or(start_date)
            .to_string();
        *completions.entry(d).or_default() += 1;
    }
    let mut remaining = total;
    (0..=days)
        .map(|i| {
            let date = start + chrono::Duration::days(i);
            let date_s = date.to_string();
            remaining -= *completions.get(&date_s).unwrap_or(&0) as f64;
            let ideal = total - (total * i as f64) / days.max(1) as f64;
            BurndownPoint {
                date: date_s,
                remaining: remaining.max(0.0),
                ideal: (ideal * 10.0).round() / 10.0,
            }
        })
        .collect()
}

fn compute_burnup_data(
    items: &[&Requirement],
    start_date: &str,
    end_date: &str,
) -> Vec<BurnupPoint> {
    let Some(start) = parse_date(start_date) else {
        return vec![];
    };
    let Some(end) = parse_date(end_date) else {
        return vec![];
    };
    if items.is_empty() || start >= end {
        return vec![];
    }
    let days = (end - start).num_days().max(0);
    let mut completions: HashMap<String, usize> = HashMap::new();
    for item in items.iter().filter(|i| is_done(&i.status)) {
        let d = item
            .modified_at
            .get(0..10)
            .unwrap_or(start_date)
            .to_string();
        *completions.entry(d).or_default() += 1;
    }
    let mut completed = 0.0;
    (0..=days)
        .map(|i| {
            let date = start + chrono::Duration::days(i);
            let date_s = date.to_string();
            completed += *completions.get(&date_s).unwrap_or(&0) as f64;
            BurnupPoint {
                date: date_s,
                completed,
                scope: items.len() as f64,
            }
        })
        .collect()
}

fn compute_velocity_data(
    sprints: &[&Requirement],
    requirements: &[Requirement],
) -> Vec<VelocityPoint> {
    sprints
        .iter()
        .rev()
        .take(8)
        .rev()
        .map(|sprint| {
            let items = sprint_items(sprint, requirements);
            let points = items
                .iter()
                .filter(|i| is_done(&i.status))
                .map(|i| i.weight.max(1))
                .sum();
            VelocityPoint {
                sprint_label: sprint
                    .custom_fields
                    .get("sprint_number")
                    .map(|n| format!("S{n}"))
                    .unwrap_or_else(|| sprint.spec_id.clone()),
                points,
            }
        })
        .collect()
}

fn status_svg(counts: &BTreeMap<String, usize>, total: usize) -> String {
    let mut y = 122.0;
    let mut bars = String::new();
    for (status, count) in counts {
        let pct = (*count as f64 / total as f64) * 100.0;
        let w = 530.0 * pct / 100.0;
        let color = status_color(status);
        bars.push_str(&format!(
            r#"<g><text x="320" y="{y}" class="muted">{}</text><text x="830" y="{y}" class="value">{} ({:.0}%)</text><rect x="320" y="{}" width="530" height="10" rx="5" class="track"/><rect x="320" y="{}" width="{w:.1}" height="10" rx="5" fill="{color}"/></g>"#,
            esc(status),
            count,
            pct,
            y + 9.0,
            y + 9.0
        ));
        y += 42.0;
    }
    chart_shell(
        "Status distribution",
        &format!("{total} active requirements"),
        420.0,
        &format!(
            r##"<circle cx="160" cy="210" r="92" fill="none" stroke="var(--border)" stroke-width="28"/>
<circle cx="160" cy="210" r="92" fill="none" stroke="var(--accent)" stroke-width="28" stroke-dasharray="578" stroke-linecap="round" opacity=".88"/>
<text x="160" y="204" text-anchor="middle" class="hero">{total}</text><text x="160" y="232" text-anchor="middle" class="muted">requirements</text>{bars}"##
        ),
    )
}

fn sprint_svg(
    sprint: &Requirement,
    items: &[&Requirement],
    burndown: &[BurndownPoint],
    burnup: &[BurnupPoint],
    velocity: &[VelocityPoint],
) -> String {
    let progress_done = items.iter().filter(|i| is_done(&i.status)).count();
    let body = format!(
        r#"<text x="28" y="86" class="muted">{} assigned items · {} completed · fallback time-series uses current status modified dates when status history is unavailable</text>
{}{}{}"#,
        items.len(),
        progress_done,
        line_chart(
            "Burndown",
            24.0,
            116.0,
            burndown,
            |p| p.remaining,
            |p| p.ideal,
            "#3b82f6",
            "#6b7280",
            "Remaining",
            "Ideal"
        ),
        burnup_chart("Burn-up", 24.0, 374.0, burnup),
        velocity_chart("Velocity", 24.0, 632.0, velocity),
    );
    chart_shell(
        &format!("{} · {}", sprint.spec_id, esc(&sprint.title)),
        "Sprint metrics",
        888.0,
        &body,
    )
}

fn feature_svg(title: &str, groups: &BTreeMap<String, (usize, usize)>) -> String {
    let height = (160 + groups.len() * 46).max(260) as f64;
    let mut body = String::new();
    let mut y = 122.0;
    for (feature, (total, completed)) in groups.iter().take(12) {
        let pct = if *total == 0 {
            0.0
        } else {
            *completed as f64 / *total as f64 * 100.0
        };
        body.push_str(&format!(
            r#"<text x="28" y="{y}" class="label">{}</text><text x="840" y="{y}" class="value">{}/{} ({:.0}%)</text><rect x="28" y="{}" width="824" height="12" rx="6" class="track"/><rect x="28" y="{}" width="{:.1}" height="12" rx="6" fill="var(--accent)"/>"#,
            esc(feature),
            completed,
            total,
            pct,
            y + 11.0,
            y + 11.0,
            824.0 * pct / 100.0
        ));
        y += 46.0;
    }
    chart_shell(title, "Completion by feature", height, &body)
}

fn line_chart<F, G>(
    title: &str,
    x0: f64,
    y0: f64,
    data: &[BurndownPoint],
    actual: F,
    ideal: G,
    actual_color: &str,
    ideal_color: &str,
    actual_label: &str,
    ideal_label: &str,
) -> String
where
    F: Fn(&BurndownPoint) -> f64,
    G: Fn(&BurndownPoint) -> f64,
{
    let h = 208.0;
    if data.len() < 2 {
        return mini_empty(title, x0, y0, h, "Not enough sprint data.");
    }
    let max_y = data
        .iter()
        .map(|p| actual(p).max(ideal(p)))
        .fold(1.0, f64::max);
    let plot_w = W - x0 * 2.0 - PAD.left - PAD.right;
    let plot_h = h - PAD.top - PAD.bottom;
    let x = |i: usize| x0 + PAD.left + (i as f64 / (data.len() - 1) as f64) * plot_w;
    let y = |v: f64| y0 + PAD.top + plot_h - (v / max_y) * plot_h;
    let actual_points = data
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{:.1},{:.1}", x(i), y(actual(p))))
        .collect::<Vec<_>>()
        .join(" ");
    let ideal_points = data
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{:.1},{:.1}", x(i), y(ideal(p))))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"<g><text x="{x0}" y="{}" class="section">{}</text>{}<polyline points="{ideal_points}" fill="none" stroke="{ideal_color}" stroke-width="1.8" stroke-dasharray="6 4" opacity=".72"/><polyline points="{actual_points}" fill="none" stroke="{actual_color}" stroke-width="2.6"/><text x="{}" y="{}" class="muted">{} · {}</text></g>"#,
        y0 + 16.0,
        esc(title),
        grid(x0, y0, h, max_y),
        x0 + PAD.left,
        y0 + h - 4.0,
        esc(actual_label),
        esc(ideal_label),
    )
}

fn burnup_chart(title: &str, x0: f64, y0: f64, data: &[BurnupPoint]) -> String {
    let converted = data
        .iter()
        .map(|p| BurndownPoint {
            date: p.date.clone(),
            remaining: p.completed,
            ideal: p.scope,
        })
        .collect::<Vec<_>>();
    line_chart(
        title,
        x0,
        y0,
        &converted,
        |p| p.remaining,
        |p| p.ideal,
        "#10b981",
        "#f59e0b",
        "Completed",
        "Scope",
    )
}

fn velocity_chart(title: &str, x0: f64, y0: f64, data: &[VelocityPoint]) -> String {
    let h = 208.0;
    if data.is_empty() {
        return mini_empty(title, x0, y0, h, "No velocity data available.");
    }
    let max_y = data.iter().map(|p| p.points).max().unwrap_or(1).max(1) as f64;
    let plot_w = W - x0 * 2.0 - PAD.left - PAD.right;
    let plot_h = h - PAD.top - PAD.bottom;
    let gap = 8.0;
    let bar_w = ((plot_w - gap * (data.len() - 1) as f64) / data.len() as f64).min(58.0);
    let total_w = data.len() as f64 * bar_w + (data.len() - 1) as f64 * gap;
    let start_x = x0 + PAD.left + (plot_w - total_w) / 2.0;
    let mut bars = String::new();
    for (i, p) in data.iter().enumerate() {
        let bh = p.points as f64 / max_y * plot_h;
        let bx = start_x + i as f64 * (bar_w + gap);
        let by = y0 + PAD.top + plot_h - bh;
        bars.push_str(&format!(
            r##"<rect x="{bx:.1}" y="{by:.1}" width="{bar_w:.1}" height="{bh:.1}" rx="4" fill="#8b5cf6" opacity=".82"/><text x="{:.1}" y="{:.1}" text-anchor="middle" class="value">{}</text><text x="{:.1}" y="{:.1}" text-anchor="middle" class="muted">{}</text>"##,
            bx + bar_w / 2.0,
            by - 5.0,
            p.points,
            bx + bar_w / 2.0,
            y0 + h - 4.0,
            esc(&p.sprint_label)
        ));
    }
    format!(
        r#"<g><text x="{x0}" y="{}" class="section">{}</text>{}{bars}</g>"#,
        y0 + 16.0,
        esc(title),
        grid(x0, y0, h, max_y)
    )
}

fn grid(x0: f64, y0: f64, h: f64, max_y: f64) -> String {
    let plot_w = W - x0 * 2.0 - PAD.left - PAD.right;
    let plot_h = h - PAD.top - PAD.bottom;
    [0.0, 0.25, 0.5, 0.75, 1.0]
        .iter()
        .map(|frac| {
            let y = y0 + PAD.top + plot_h * (1.0 - frac);
            format!(
                r#"<line x1="{}" y1="{y:.1}" x2="{}" y2="{y:.1}" class="grid"/><text x="{}" y="{:.1}" text-anchor="end" class="muted">{}</text>"#,
                x0 + PAD.left,
                x0 + PAD.left + plot_w,
                x0 + PAD.left - 7.0,
                y + 3.0,
                (max_y * frac).round() as i64
            )
        })
        .collect::<String>()
}

fn mini_empty(title: &str, x0: f64, y0: f64, h: f64, msg: &str) -> String {
    format!(
        r#"<g><text x="{x0}" y="{}" class="section">{}</text><rect x="{x0}" y="{}" width="872" height="{}" rx="8" class="empty"/><text x="460" y="{}" text-anchor="middle" class="muted">{}</text></g>"#,
        y0 + 16.0,
        esc(title),
        y0 + 32.0,
        h - 40.0,
        y0 + h / 2.0,
        esc(msg)
    )
}

fn chart_shell(title: &str, subtitle: &str, height: f64, body: &str) -> String {
    format!(
        r##"<svg class="aida-chart-svg" role="img" aria-label="{}" viewBox="0 0 920 {height}" xmlns="http://www.w3.org/2000/svg">
<style>
.aida-chart-svg{{width:100%;height:auto;display:block;background:linear-gradient(180deg,var(--bg-elev),var(--assistant));border:1px solid var(--border);border-radius:8px;color:var(--text);font-family:Inter,ui-sans-serif,system-ui,sans-serif}}
.title{{fill:var(--text);font-size:18px;font-weight:700}}.subtitle,.muted{{fill:var(--text-dim);font-size:11px}}.section{{fill:var(--text);font-size:13px;font-weight:700}}.label{{fill:var(--text);font-size:12px;font-weight:600}}.value{{fill:var(--text);font-size:11px;font-weight:700}}.hero{{fill:var(--text);font-size:34px;font-weight:800}}.grid{{stroke:var(--border);stroke-width:1;opacity:.68}}.track{{fill:var(--border);opacity:.52}}.empty{{fill:var(--bg);stroke:var(--border);stroke-width:1}}
</style>
<text x="28" y="34" class="title">{}</text><text x="28" y="56" class="subtitle">{}</text>{body}</svg>"##,
        esc(title),
        esc(title),
        esc(subtitle)
    )
}

fn empty_svg(title: &str, msg: &str) -> String {
    chart_shell(
        title,
        "No data",
        260.0,
        &format!(
            r#"<rect x="28" y="82" width="864" height="132" rx="8" class="empty"/><text x="460" y="145" text-anchor="middle" class="section">{}</text><text x="460" y="168" text-anchor="middle" class="muted">{}</text>"#,
            esc(title),
            esc(msg)
        ),
    )
}

fn status_color(status: &str) -> &'static str {
    match status {
        "Draft" => "#6b7280",
        "Approved" => "#3b82f6",
        "Planned" => "#8b5cf6",
        "InProgress" => "#f59e0b",
        "NeedsAttention" => "#d946ef",
        "Done" => "#84cc16",
        "Completed" => "#10b981",
        "Rejected" => "#ef4444",
        _ => "#94a3b8",
    }
}

fn is_done(status: &str) -> bool {
    matches!(status, "Completed" | "Done")
}

fn is_stateless(req_type: &str) -> bool {
    matches!(req_type, "Folder" | "Meta" | "Sprint")
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.get(0..10)?, "%Y-%m-%d").ok()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(spec_id: &str, status: &str, modified_at: &str) -> Requirement {
        Requirement {
            id: spec_id.to_string(),
            spec_id: spec_id.to_string(),
            title: spec_id.to_string(),
            status: status.to_string(),
            req_type: "Story".into(),
            feature: "Core".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            modified_at: modified_at.into(),
            yaml_path: String::new(),
            weight: 1,
            custom_fields: HashMap::new(),
            relationships: vec![],
        }
    }

    #[test]
    fn burndown_ports_reference_completion_fallback() {
        let a = req("A", "Completed", "2026-01-02T00:00:00Z");
        let b = req("B", "InProgress", "2026-01-03T00:00:00Z");
        let items = vec![&a, &b];
        let data = compute_burndown_data(&items, "2026-01-01", "2026-01-03");
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].remaining, 2.0);
        assert_eq!(data[1].remaining, 1.0);
        assert_eq!(data[2].ideal, 0.0);
    }

    #[test]
    fn burnup_ports_reference_scope_line() {
        let a = req("A", "Completed", "2026-01-02T00:00:00Z");
        let b = req("B", "Approved", "2026-01-03T00:00:00Z");
        let items = vec![&a, &b];
        let data = compute_burnup_data(&items, "2026-01-01", "2026-01-03");
        assert_eq!(data[0].scope, 2.0);
        assert_eq!(data[1].completed, 1.0);
        assert_eq!(data[2].scope, 2.0);
    }

    #[test]
    fn chart_payload_roundtrips() {
        let artifact = ChartArtifact {
            title: "Status".into(),
            summary: "ok".into(),
            svg: "<svg/>".into(),
        };
        let encoded = encode_artifact(&artifact);
        assert_eq!(extract_chart_artifact(&encoded), Some(artifact));
    }
}
