// trace:EPIC-29 | ai:claude
//
// AIDA substrate reader. Walks `.aida-store/objects/**/*.yaml` (the
// distributed git-canonical store) and yields a typed in-memory model.
//
// Why not `aida list --json` or MCP `list_requirements`: both return
// summary projections — no `weight`, `custom_fields`, `relationships`,
// or `history`. Burn-down needs status-change history; sprint
// membership needs the relationship graph. Reading YAML directly is
// the cleanest path until aida-core exposes the rich shape over the
// wire (filed as a future-work hook in the architecture doc).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("no .aida-store directory found under {0}")]
    Missing(PathBuf),
    #[error("yaml parse error at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("walk error: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub spec_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub req_type: String,
    #[serde(default)]
    pub feature: Option<String>,
    /// Story points. AIDA doesn't currently emit this in the YAML for
    /// most items, so velocity falls back to "1 point per completed
    /// item" — matches the React reference's `i.weight ?? 1`.
    #[serde(default)]
    pub weight: Option<u32>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub custom_fields: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Relationship {
    /// Raw YAML node so we can interpret AIDA's `!Custom <name>` tagged
    /// scalars without a brittle serde enum mapping. See `rel_kind`.
    pub rel_type: serde_yaml::Value,
    pub target_id: String,
}

impl Relationship {
    /// Resolve `rel_type` to a normalized `Tag:Value` string.
    /// Examples:
    ///   `!Custom sprint_contains` → `"Custom:sprint_contains"`
    ///   bare `Parent`             → `"Parent"`
    pub fn rel_kind(&self) -> Option<String> {
        match &self.rel_type {
            serde_yaml::Value::Tagged(t) => {
                let tag = t.tag.to_string();
                let stripped = tag.trim_start_matches('!').to_string();
                match &t.value {
                    serde_yaml::Value::String(s) => Some(format!("{stripped}:{s}")),
                    _ => Some(stripped),
                }
            }
            serde_yaml::Value::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// True iff this is `Custom("sprint_contains")` — sprint → item.
    pub fn is_sprint_contains(&self) -> bool {
        self.rel_kind().as_deref() == Some("Custom:sprint_contains")
    }

    /// True iff this is `Custom("sprint_assignment")` — item → sprint.
    pub fn is_sprint_assignment(&self) -> bool {
        self.rel_kind().as_deref() == Some("Custom:sprint_assignment")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Change {
    pub field_name: String,
    #[serde(default)]
    pub old_value: serde_yaml::Value,
    #[serde(default)]
    pub new_value: serde_yaml::Value,
}

impl Requirement {
    /// Date (YYYY-MM-DD) on which this requirement first transitioned
    /// to `Completed`, by scanning `history` for the status change.
    /// Falls back to `modified_at` for items that lack the journal
    /// entry — matches the React reference.
    pub fn completed_date(&self) -> Option<String> {
        for entry in &self.history {
            for change in &entry.changes {
                if change.field_name == "status"
                    && string_value(&change.new_value).as_deref() == Some("Completed")
                {
                    return Some(yyyy_mm_dd(&entry.timestamp));
                }
            }
        }
        if self.status == "Completed" && !self.modified_at.is_empty() {
            return Some(yyyy_mm_dd(&self.modified_at));
        }
        None
    }

    /// Read a custom-field string. Returns None if missing or not a string.
    pub fn custom_field_str(&self, key: &str) -> Option<&str> {
        self.custom_fields.get(key).and_then(|v| v.as_str())
    }
}

fn string_value(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn yyyy_mm_dd(ts: &str) -> String {
    ts.chars().take(10).collect()
}

/// Sprint view: the requirement plus its dates and the resolved
/// member SPEC-IDs (from `sprint_contains` → target uuids → reqs).
#[derive(Debug, Clone)]
pub struct Sprint<'a> {
    pub req: &'a Requirement,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sprint_number: Option<i64>,
    pub member_ids: Vec<String>,
}

impl<'a> Sprint<'a> {
    pub fn state(&self, today: &str) -> super::data::SprintState {
        use super::data::SprintState;
        let (start, end) = match (&self.start_date, &self.end_date) {
            (Some(s), Some(e)) => (s, e),
            _ => return SprintState::Unknown,
        };
        if today < start.as_str() {
            SprintState::Future
        } else if today > end.as_str() {
            SprintState::Past
        } else {
            SprintState::Active
        }
    }
}

#[derive(Debug, Clone)]
pub struct AidaStore {
    pub items: Vec<Requirement>,
    /// uuid → index into `items` (relationships reference UUIDs).
    pub by_uuid: HashMap<String, usize>,
}

impl AidaStore {
    /// Load every YAML file under `<repo_root>/.aida-store/objects/`.
    /// Returns an empty store (not an error) on a project that doesn't
    /// have an aida-store checked out — the chart layer surfaces this
    /// as an empty-state. To distinguish "no aida-store" from "empty
    /// aida-store", check `Self::has_store(repo_root)` first.
    pub fn load(repo_root: &Path) -> Result<Self, StoreError> {
        let objects = repo_root.join(".aida-store").join("objects");
        if !objects.is_dir() {
            return Err(StoreError::Missing(repo_root.to_path_buf()));
        }
        let mut items: Vec<Requirement> = Vec::new();
        let mut by_uuid: HashMap<String, usize> = HashMap::new();
        for entry in WalkDir::new(&objects).into_iter().filter_map(Result::ok) {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let bytes = std::fs::read(p)?;
            // Skip empty / placeholder files.
            if bytes.iter().all(|b| b.is_ascii_whitespace()) {
                continue;
            }
            let req: Requirement = match serde_yaml::from_slice(&bytes) {
                Ok(r) => r,
                Err(source) => {
                    // Soft-fail individual files so a single malformed
                    // YAML doesn't poison the whole load. Log and skip.
                    eprintln!(
                        "[charts] skipping unparseable yaml {}: {source}",
                        p.display()
                    );
                    continue;
                }
            };
            let idx = items.len();
            if !req.id.is_empty() {
                by_uuid.insert(req.id.clone(), idx);
            }
            items.push(req);
        }
        Ok(Self { items, by_uuid })
    }

    pub fn has_store(repo_root: &Path) -> bool {
        repo_root.join(".aida-store").join("objects").is_dir()
    }

    /// Every requirement whose `req_type` matches (case-insensitive).
    pub fn by_type(&self, t: &str) -> Vec<&Requirement> {
        self.items
            .iter()
            .filter(|r| r.req_type.eq_ignore_ascii_case(t))
            .collect()
    }

    /// Sprints (requirements of type "Sprint"), in stable order:
    /// numbered sprints first by number ascending, then unnumbered.
    pub fn sprints(&self) -> Vec<Sprint<'_>> {
        let mut sprints: Vec<Sprint<'_>> = self
            .by_type("Sprint")
            .into_iter()
            .map(|r| self.sprint_view(r))
            .collect();
        sprints.sort_by(|a, b| match (a.sprint_number, b.sprint_number) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.req.spec_id.cmp(&b.req.spec_id),
        });
        sprints
    }

    fn sprint_view<'a>(&'a self, req: &'a Requirement) -> Sprint<'a> {
        // Members from `sprint_contains` (sprint → item) — the dominant
        // AIDA idiom. Resolve target UUIDs to spec_ids via by_uuid.
        let mut member_ids: Vec<String> = Vec::new();
        for rel in &req.relationships {
            if rel.is_sprint_contains() {
                if let Some(&idx) = self.by_uuid.get(&rel.target_id) {
                    member_ids.push(self.items[idx].spec_id.clone());
                }
            }
        }
        // Also pick up legacy item → sprint (`sprint_assignment`) — any
        // item that points at THIS sprint.
        for other in &self.items {
            for rel in &other.relationships {
                if rel.is_sprint_assignment() && rel.target_id == req.id {
                    member_ids.push(other.spec_id.clone());
                }
            }
        }
        member_ids.sort();
        member_ids.dedup();
        Sprint {
            req,
            start_date: req.custom_field_str("start_date").map(str::to_string),
            end_date: req.custom_field_str("end_date").map(str::to_string),
            sprint_number: req
                .custom_field_str("sprint_number")
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| {
                    req.custom_fields
                        .get("sprint_number")
                        .and_then(|v| v.as_i64())
                }),
            member_ids,
        }
    }

    /// Resolve a spec_id to its requirement.
    pub fn by_spec(&self, spec_id: &str) -> Option<&Requirement> {
        self.items.iter().find(|r| r.spec_id == spec_id)
    }

    /// All requirements that belong to a given sprint (resolved by
    /// the sprint's `Sprint::member_ids`).
    pub fn sprint_items(&self, sprint: &Sprint<'_>) -> Vec<&Requirement> {
        let set: std::collections::HashSet<&str> =
            sprint.member_ids.iter().map(|s| s.as_str()).collect();
        self.items
            .iter()
            .filter(|r| set.contains(r.spec_id.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(spec: &str, status: &str, ty: &str, feature: Option<&str>) -> Requirement {
        Requirement {
            id: format!("uuid-{spec}"),
            spec_id: spec.into(),
            title: spec.into(),
            status: status.into(),
            req_type: ty.into(),
            feature: feature.map(String::from),
            weight: None,
            created_at: "2026-05-20T00:00:00Z".into(),
            modified_at: "2026-05-22T00:00:00Z".into(),
            relationships: vec![],
            history: vec![],
            custom_fields: HashMap::new(),
        }
    }

    #[test]
    fn completed_date_uses_history_when_available() {
        let mut r = req("STORY-1", "Completed", "Story", None);
        r.history = vec![HistoryEntry {
            timestamp: "2026-05-21T12:00:00Z".into(),
            author: "joe".into(),
            changes: vec![Change {
                field_name: "status".into(),
                old_value: serde_yaml::Value::String("InProgress".into()),
                new_value: serde_yaml::Value::String("Completed".into()),
            }],
        }];
        assert_eq!(r.completed_date().as_deref(), Some("2026-05-21"));
    }

    #[test]
    fn completed_date_falls_back_to_modified_at() {
        let r = req("STORY-2", "Completed", "Story", None);
        assert_eq!(r.completed_date().as_deref(), Some("2026-05-22"));
    }

    #[test]
    fn completed_date_none_for_open_items() {
        let r = req("STORY-3", "InProgress", "Story", None);
        assert!(r.completed_date().is_none());
    }

    #[test]
    fn parses_real_sprint_yaml() {
        // Smoke-parse the fixture committed at the top of this brief —
        // SPRINT-3 from aida core. The YAML uses `!Custom sprint_contains`
        // tagged relationships and `custom_fields.start_date/end_date`.
        let yaml = r#"
id: 404b7457-67a2-4376-a0d3-9c5ea1547951
spec_id: SPRINT-3
title: 'Sprint #3 thru 2025-12-07 Friday'
status: Rejected
req_type: Sprint
created_at: 2025-12-07T17:02:15.896382981Z
modified_at: 2026-05-04T19:32:25.723689667Z
relationships:
- rel_type: !Custom sprint_contains
  target_id: 37be9566-a051-430b-b47b-1890a395a524
custom_fields:
  end_date: 2026-03-13
  sprint_number: '3'
  start_date: 2026-03-06
"#;
        let r: Requirement = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(r.spec_id, "SPRINT-3");
        assert_eq!(r.req_type, "Sprint");
        assert_eq!(r.custom_field_str("start_date"), Some("2026-03-06"));
        assert_eq!(r.custom_field_str("end_date"), Some("2026-03-13"));
        assert_eq!(r.relationships.len(), 1);
        assert!(r.relationships[0].is_sprint_contains());
        assert_eq!(
            r.relationships[0].rel_kind().as_deref(),
            Some("Custom:sprint_contains")
        );
    }
}
