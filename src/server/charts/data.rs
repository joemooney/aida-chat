// trace:EPIC-29 | ai:claude
//
// Algorithmic core for agile charts. Ported from
// `~/ai/aida/aida-web-react/src/lib/sprint-utils.ts` — translation
// only, no inventions. Function names match the source for grep-ability.
//
// Status taxonomy follows AIDA's canonical enum order:
//   Draft → Approved → Planned → InProgress → NeedsAttention →
//   Done / Completed / Rejected
// (the React reference uses the same ordering in STATUS_ORDER.)

use std::collections::BTreeMap;

use super::store::{Requirement, Sprint};

// =========================================================================
// Status distribution
// =========================================================================

/// Canonical status order for the donut + legend. Matches
/// aida-web-react `lib/constants.ts::STATUS_ORDER`.
pub const STATUS_ORDER: &[&str] = &[
    "Draft",
    "Approved",
    "Planned",
    "InProgress",
    "NeedsAttention",
    "Done",
    "Completed",
    "Rejected",
];

/// Hex color per status — same palette as
/// `aida-web-react/.../StatusChart.tsx::statusColors`.
pub fn status_color(status: &str) -> &'static str {
    match status {
        "Draft" => "#6b7280",
        "Approved" => "#3b82f6",
        "Planned" => "#8b5cf6",
        "InProgress" => "#f59e0b",
        "NeedsAttention" => "#d946ef",
        "Done" => "#84cc16",
        "Completed" => "#10b981",
        "Rejected" => "#ef4444",
        _ => "#475569",
    }
}

#[derive(Debug, Clone)]
pub struct StatusCounts {
    /// `status name → count`, ordered by `STATUS_ORDER` then alpha for unknowns.
    pub buckets: Vec<(String, usize)>,
    pub total: usize,
}

pub fn compute_status_counts(items: &[&Requirement]) -> StatusCounts {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in items {
        *counts.entry(r.status.clone()).or_insert(0) += 1;
    }
    let mut buckets: Vec<(String, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for &s in STATUS_ORDER {
        if let Some(&n) = counts.get(s) {
            if n > 0 {
                buckets.push((s.to_string(), n));
                seen.insert(s.to_string());
            }
        }
    }
    for (k, v) in &counts {
        if !seen.contains(k) && *v > 0 {
            buckets.push((k.clone(), *v));
        }
    }
    StatusCounts {
        buckets,
        total: items.len(),
    }
}

// =========================================================================
// Burn-down — port of computeBurndownData
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct BurndownPoint {
    pub date: String,
    pub remaining: f64,
    pub ideal: f64,
}

/// Per-day remaining-work line + ideal sloped line for a sprint window.
/// Sister to React `computeBurndownData(items, startDate, endDate)`.
pub fn compute_burndown(items: &[&Requirement], start: &str, end: &str) -> Vec<BurndownPoint> {
    let total = items.len();
    let days = days_between(start, end);
    if total == 0 || days == 0 {
        return Vec::new();
    }

    // date → completions on that date (from history journal, or
    // modified_at fallback for items already Completed without a
    // matching history entry).
    let mut completions: BTreeMap<String, usize> = BTreeMap::new();
    for item in items {
        if item.status != "Completed" {
            continue;
        }
        if let Some(d) = item.completed_date() {
            *completions.entry(d).or_insert(0) += 1;
        }
    }

    let mut out: Vec<BurndownPoint> = Vec::with_capacity(days + 1);
    let mut remaining = total as i64;
    for i in 0..=days {
        let date = add_days(start, i as i64);
        let ideal_v = total as f64 - (total as f64 * i as f64) / days as f64;
        remaining -= completions.get(&date).copied().unwrap_or(0) as i64;
        out.push(BurndownPoint {
            date,
            remaining: remaining.max(0) as f64,
            ideal: round1(ideal_v),
        });
    }
    out
}

// =========================================================================
// Burn-up — port of computeBurnupData
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct BurnupPoint {
    pub date: String,
    pub completed: f64,
    pub scope: f64,
}

pub fn compute_burnup(items: &[&Requirement], start: &str, end: &str) -> Vec<BurnupPoint> {
    let scope = items.len();
    let days = days_between(start, end);
    if scope == 0 || days == 0 {
        return Vec::new();
    }
    let mut completions: BTreeMap<String, usize> = BTreeMap::new();
    for item in items {
        if item.status != "Completed" {
            continue;
        }
        if let Some(d) = item.completed_date() {
            *completions.entry(d).or_insert(0) += 1;
        }
    }
    let mut out: Vec<BurnupPoint> = Vec::with_capacity(days + 1);
    let mut cum = 0_i64;
    for i in 0..=days {
        let date = add_days(start, i as i64);
        cum += completions.get(&date).copied().unwrap_or(0) as i64;
        out.push(BurnupPoint {
            date,
            completed: cum as f64,
            scope: scope as f64,
        });
    }
    out
}

// =========================================================================
// Velocity — port of computeVelocityData
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct VelocityPoint {
    pub label: String,
    /// Sum of completed weights (story points). Weight defaults to 1
    /// per item when not set — matches React `i.weight ?? 1`.
    pub points: u64,
}

/// One bar per sprint, with the label = `S{number}` if a sprint_number
/// is set, else the first 8 chars of the sprint title.
pub fn compute_velocity<'a>(
    sprints: &[Sprint<'a>],
    items_for_sprint: impl Fn(&Sprint<'a>) -> Vec<&'a Requirement>,
) -> Vec<VelocityPoint> {
    sprints
        .iter()
        .map(|s| VelocityPoint {
            label: match s.sprint_number {
                Some(n) => format!("S{n}"),
                None => {
                    let t: String = s.req.title.chars().take(8).collect();
                    if t.is_empty() {
                        s.req.spec_id.clone()
                    } else {
                        t
                    }
                }
            },
            points: items_for_sprint(s)
                .into_iter()
                .filter(|i| i.status == "Completed")
                .map(|i| i.weight.unwrap_or(1) as u64)
                .sum(),
        })
        .collect()
}

// =========================================================================
// Feature progress — port of FeatureProgress.tsx
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureProgressRow {
    pub feature: String,
    pub completed: u64,
    pub total: u64,
}

impl FeatureProgressRow {
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            0
        } else {
            ((self.completed as f64 / self.total as f64) * 100.0).round() as u32
        }
    }
}

/// Grouped by `requirement.feature` (or `"Uncategorized"` for empty);
/// sorted by total descending. Matches FeatureProgress.tsx.
pub fn compute_feature_progress(items: &[&Requirement]) -> Vec<FeatureProgressRow> {
    let mut by_feature: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for r in items {
        let key = r
            .feature
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("Uncategorized")
            .to_string();
        let entry = by_feature.entry(key).or_insert((0, 0));
        entry.0 += 1; // total
        if r.status == "Completed" {
            entry.1 += 1; // completed
        }
    }
    let mut rows: Vec<FeatureProgressRow> = by_feature
        .into_iter()
        .map(|(feature, (total, completed))| FeatureProgressRow {
            feature,
            total,
            completed,
        })
        .collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.feature.cmp(&b.feature)));
    rows
}

// =========================================================================
// Sprint progress + state
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct SprintProgress {
    pub total: u64,
    pub completed: u64,
    pub percent: u32,
    pub total_points: u64,
    pub completed_points: u64,
}

pub fn compute_sprint_progress(items: &[&Requirement]) -> SprintProgress {
    let mut total_pts = 0_u64;
    let mut done_pts = 0_u64;
    let mut done = 0_u64;
    for r in items {
        let w = r.weight.unwrap_or(1) as u64;
        total_pts += w;
        if r.status == "Completed" {
            done += 1;
            done_pts += w;
        }
    }
    let total = items.len() as u64;
    let percent = if total == 0 {
        0
    } else {
        ((done as f64 / total as f64) * 100.0).round() as u32
    };
    SprintProgress {
        total,
        completed: done,
        percent,
        total_points: total_pts,
        completed_points: done_pts,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprintState {
    Active,
    Past,
    Future,
    Unknown,
}

// =========================================================================
// Date helpers
// =========================================================================

/// Count of days between two YYYY-MM-DD dates (end - start). Tolerant
/// to trailing time / fraction. Returns 0 for malformed input.
pub fn days_between(start: &str, end: &str) -> usize {
    let (Some(a), Some(b)) = (parse_ymd(start), parse_ymd(end)) else {
        return 0;
    };
    let a = days_since_epoch(a);
    let b = days_since_epoch(b);
    if b >= a {
        (b - a) as usize
    } else {
        0
    }
}

/// Date string `n` days after `start` (YYYY-MM-DD format on the wire).
pub fn add_days(start: &str, n: i64) -> String {
    let Some(d) = parse_ymd(start) else {
        return start.to_string();
    };
    let total = days_since_epoch(d) + n;
    let (y, m, d) = epoch_to_ymd(total);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    if s.len() < 10 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn days_in_month(y: i64, m: u32) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Days since 1970-01-01 (signed). Roughly Unix epoch but date-only.
fn days_since_epoch((y, m, d): (i64, u32, u32)) -> i64 {
    let mut total: i64 = 0;
    if y >= 1970 {
        for yr in 1970..y {
            total += if is_leap(yr) { 366 } else { 365 };
        }
    } else {
        for yr in y..1970 {
            total -= if is_leap(yr) { 366 } else { 365 };
        }
    }
    for mm in 1..m {
        total += days_in_month(y, mm);
    }
    total + (d as i64 - 1)
}

fn epoch_to_ymd(mut days: i64) -> (i64, u32, u32) {
    let mut y: i64 = 1970;
    if days >= 0 {
        loop {
            let yd = if is_leap(y) { 366 } else { 365 };
            if days < yd {
                break;
            }
            days -= yd;
            y += 1;
        }
    } else {
        while days < 0 {
            y -= 1;
            let yd = if is_leap(y) { 366 } else { 365 };
            days += yd;
        }
    }
    let mut m: u32 = 1;
    loop {
        let md = days_in_month(y, m);
        if days < md {
            break;
        }
        days -= md;
        m += 1;
    }
    (y, m, days as u32 + 1)
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::super::store::{Change, HistoryEntry, Requirement};
    use super::*;
    use std::collections::HashMap;

    fn r(spec: &str, status: &str, feature: Option<&str>) -> Requirement {
        Requirement {
            id: format!("uuid-{spec}"),
            spec_id: spec.into(),
            title: spec.into(),
            status: status.into(),
            req_type: "Story".into(),
            feature: feature.map(String::from),
            weight: None,
            created_at: String::new(),
            modified_at: String::new(),
            relationships: vec![],
            history: vec![],
            custom_fields: HashMap::new(),
        }
    }

    fn r_completed(spec: &str, on_date: &str) -> Requirement {
        let mut req = r(spec, "Completed", None);
        req.history = vec![HistoryEntry {
            timestamp: format!("{on_date}T12:00:00Z"),
            author: String::new(),
            changes: vec![Change {
                field_name: "status".into(),
                old_value: serde_yaml::Value::String("InProgress".into()),
                new_value: serde_yaml::Value::String("Completed".into()),
            }],
        }];
        req
    }

    #[test]
    fn ymd_arithmetic_round_trips() {
        assert_eq!(days_between("2026-05-20", "2026-05-25"), 5);
        assert_eq!(days_between("2026-05-25", "2026-05-20"), 0);
        assert_eq!(days_between("2026-02-28", "2026-03-01"), 1);
        // leap year boundary
        assert_eq!(days_between("2024-02-28", "2024-03-01"), 2);
        assert_eq!(add_days("2026-05-20", 5), "2026-05-25");
        assert_eq!(add_days("2026-12-31", 1), "2027-01-01");
        assert_eq!(add_days("2024-02-28", 1), "2024-02-29");
    }

    #[test]
    fn burndown_drops_when_items_complete() {
        let items_vec = vec![
            r_completed("S-1", "2026-03-02"),
            r_completed("S-2", "2026-03-04"),
            r("S-3", "InProgress", None),
        ];
        let items: Vec<&Requirement> = items_vec.iter().collect();
        let points = compute_burndown(&items, "2026-03-01", "2026-03-06");
        assert_eq!(points.len(), 6);
        // Day 0: nothing completed yet.
        assert_eq!(points[0].remaining, 3.0);
        // Day 1 (2026-03-02): S-1 completes → 2 remaining.
        assert_eq!(points[1].date, "2026-03-02");
        assert_eq!(points[1].remaining, 2.0);
        // Day 3 (2026-03-04): S-2 completes → 1 remaining.
        assert_eq!(points[3].remaining, 1.0);
        // Final day: still 1 (S-3 never completed).
        assert_eq!(points[5].remaining, 1.0);
        // Ideal: descends linearly from 3 to 0.
        assert_eq!(points[0].ideal, 3.0);
        assert_eq!(points[5].ideal, 0.0);
    }

    #[test]
    fn burnup_climbs_with_completion() {
        let items_vec = vec![
            r_completed("S-1", "2026-03-02"),
            r_completed("S-2", "2026-03-04"),
            r("S-3", "InProgress", None),
        ];
        let items: Vec<&Requirement> = items_vec.iter().collect();
        let points = compute_burnup(&items, "2026-03-01", "2026-03-06");
        assert_eq!(points.len(), 6);
        assert_eq!(points[0].completed, 0.0);
        assert_eq!(points[0].scope, 3.0);
        assert_eq!(points[1].completed, 1.0);
        assert_eq!(points[3].completed, 2.0);
        assert_eq!(points[5].completed, 2.0);
        // Scope stays constant at 3 across the sprint window.
        for p in &points {
            assert_eq!(p.scope, 3.0);
        }
    }

    #[test]
    fn status_counts_uses_canonical_order() {
        let items_vec = vec![
            r("R-1", "Completed", None),
            r("R-2", "Completed", None),
            r("R-3", "InProgress", None),
            r("R-4", "Draft", None),
        ];
        let items: Vec<&Requirement> = items_vec.iter().collect();
        let counts = compute_status_counts(&items);
        assert_eq!(counts.total, 4);
        // Order: Draft, InProgress, Completed.
        let order: Vec<&str> = counts.buckets.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(order, vec!["Draft", "InProgress", "Completed"]);
    }

    #[test]
    fn feature_progress_groups_and_sorts() {
        let items_vec = vec![
            r("R-1", "Completed", Some("auth")),
            r("R-2", "Completed", Some("auth")),
            r("R-3", "InProgress", Some("auth")),
            r("R-4", "Completed", Some("billing")),
            r("R-5", "Approved", None),
        ];
        let items: Vec<&Requirement> = items_vec.iter().collect();
        let rows = compute_feature_progress(&items);
        // Three groups: auth (3), billing (1), Uncategorized (1).
        assert_eq!(rows.len(), 3);
        // Sorted by total desc.
        assert_eq!(rows[0].feature, "auth");
        assert_eq!(rows[0].total, 3);
        assert_eq!(rows[0].completed, 2);
        assert_eq!(rows[0].percent(), 67);
    }

    #[test]
    fn empty_inputs_return_empty_outputs() {
        let empty: Vec<&Requirement> = vec![];
        assert!(compute_burndown(&empty, "2026-03-01", "2026-03-10").is_empty());
        assert!(compute_burnup(&empty, "2026-03-01", "2026-03-10").is_empty());
        let c = compute_status_counts(&empty);
        assert_eq!(c.total, 0);
        assert!(c.buckets.is_empty());
        assert!(compute_feature_progress(&empty).is_empty());
    }
}
