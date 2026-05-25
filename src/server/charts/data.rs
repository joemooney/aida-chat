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

// =========================================================================
// V2: Cumulative flow diagram (CFD) — port of "stacked area of status
// counts over time". Per-day buckets across a configurable window.
// trace:EPIC-29 | ai:claude
// =========================================================================

/// One day's slice of the CFD. `by_status` is keyed by status name in
/// the canonical `STATUS_ORDER` (with unknown statuses appended
/// alphabetically). Missing keys = 0 items in that bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct CfdPoint {
    pub date: String,
    pub by_status: BTreeMap<String, u32>,
}

/// Compute a CFD across the last `window_days` days. Optionally filter
/// items by req_type (case-insensitive). The algorithm walks each item's
/// history journal to determine which status it was in on each day in
/// the window — uses the same time-series source as V1's true burn-down.
///
/// Today is the last bucket; today − (window_days − 1) is the first.
pub fn compute_cfd(items: &[&Requirement], today: &str, window_days: u32) -> Vec<CfdPoint> {
    if window_days == 0 || items.is_empty() {
        return Vec::new();
    }

    // Precompute each item's status timeline so the per-day lookup is
    // O(timeline.len()) instead of O(history.len()).
    let timelines: Vec<ItemTimeline> = items.iter().map(|r| build_status_timeline(r)).collect();

    let start = add_days(today, -(window_days as i64 - 1));
    let mut out: Vec<CfdPoint> = Vec::with_capacity(window_days as usize);
    for i in 0..window_days as i64 {
        let date = add_days(&start, i);
        let mut by_status: BTreeMap<String, u32> = BTreeMap::new();
        for t in &timelines {
            if let Some(status) = t.status_on(&date) {
                *by_status.entry(status.to_string()).or_insert(0) += 1;
            }
        }
        out.push(CfdPoint { date, by_status });
    }
    out
}

/// Per-item status timeline. `entries[i] = (start_date, status)`; the
/// status applies from `start_date` until the next entry's date.
struct ItemTimeline {
    created: String,
    entries: Vec<(String, String)>,
}

impl ItemTimeline {
    fn status_on<'a>(&'a self, date: &str) -> Option<&'a str> {
        if date < self.created.as_str() {
            return None;
        }
        let mut current: Option<&str> = None;
        for (d, s) in &self.entries {
            if d.as_str() <= date {
                current = Some(s.as_str());
            } else {
                break;
            }
        }
        current
    }
}

fn build_status_timeline(r: &Requirement) -> ItemTimeline {
    let created = ymd_prefix(&r.created_at);

    let mut changes: Vec<(String, String)> = Vec::new();
    for entry in &r.history {
        for change in &entry.changes {
            if change.field_name == "status" {
                let date = ymd_prefix(&entry.timestamp);
                if let serde_yaml::Value::String(s) = &change.new_value {
                    changes.push((date, s.clone()));
                }
            }
        }
    }
    changes.sort_by(|a, b| a.0.cmp(&b.0));

    // Initial status: the earliest `old_value`, else current `status`.
    let initial_status = first_initial_status(r).unwrap_or_else(|| r.status.clone());

    let mut entries: Vec<(String, String)> = vec![(created.clone(), initial_status)];
    for (d, s) in changes {
        if let Some(last) = entries.last() {
            if last.1 == s {
                continue;
            }
        }
        entries.push((d, s));
    }

    ItemTimeline { created, entries }
}

/// First `old_value` across all history status changes — i.e. the
/// status the item had before any recorded change.
fn first_initial_status(r: &Requirement) -> Option<String> {
    let mut earliest: Option<(String, String)> = None;
    for entry in &r.history {
        for change in &entry.changes {
            if change.field_name == "status" {
                let ts = ymd_prefix(&entry.timestamp);
                let old = match &change.old_value {
                    serde_yaml::Value::String(s) => s.clone(),
                    _ => continue,
                };
                match &earliest {
                    Some((e_ts, _)) if e_ts.as_str() <= ts.as_str() => {}
                    _ => earliest = Some((ts, old)),
                }
            }
        }
    }
    earliest.map(|(_, s)| s)
}

fn ymd_prefix(ts: &str) -> String {
    ts.chars().take(10).collect()
}

// =========================================================================
// V2: Dependency graph — BFS from a SPEC-ID through relationships.
// trace:EPIC-29 | ai:claude
// =========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct DepNode {
    pub spec_id: String,
    pub status: String,
    /// 0 = root; 1 = direct neighbor; …
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DepEdge {
    pub from_spec: String,
    pub to_spec: String,
    /// Normalized rel-kind, e.g. `"Custom:sprint_contains"` or `"Parent"`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DepGraph {
    pub nodes: Vec<DepNode>,
    pub edges: Vec<DepEdge>,
    /// `true` when BFS stopped at the depth limit with unvisited
    /// neighbors remaining (operator can ask for higher depth).
    pub truncated: bool,
}

/// BFS from `root_spec` through outgoing relationships, up to
/// `max_depth`. Visited set prevents revisits; cycles short-circuit
/// silently (no visualization — out of scope per brief unless
/// `aida rel list` itself produces them, which it doesn't).
///
/// Caller supplies two lookups so this function stays independent of
/// the storage layer (testable with handwritten fixtures).
pub fn compute_dep_graph<'a>(
    root_spec: &str,
    max_depth: u32,
    lookup_by_spec: impl Fn(&str) -> Option<&'a Requirement>,
    lookup_by_uuid: impl Fn(&str) -> Option<&'a Requirement>,
) -> DepGraph {
    use std::collections::{HashSet, VecDeque};
    let mut nodes: Vec<DepNode> = Vec::new();
    let mut edges: Vec<DepEdge> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut truncated = false;

    let Some(root) = lookup_by_spec(root_spec) else {
        return DepGraph {
            nodes,
            edges,
            truncated: false,
        };
    };

    queue.push_back((root.spec_id.clone(), 0));
    seen.insert(root.spec_id.clone());
    nodes.push(DepNode {
        spec_id: root.spec_id.clone(),
        status: root.status.clone(),
        depth: 0,
    });

    while let Some((spec_id, depth)) = queue.pop_front() {
        let Some(req) = lookup_by_spec(&spec_id) else {
            continue;
        };
        if depth >= max_depth {
            if !req.relationships.is_empty() {
                truncated = true;
            }
            continue;
        }
        for rel in &req.relationships {
            let Some(target) = lookup_by_uuid(&rel.target_id) else {
                continue;
            };
            let kind = rel.rel_kind().unwrap_or_else(|| "Unknown".into());
            edges.push(DepEdge {
                from_spec: req.spec_id.clone(),
                to_spec: target.spec_id.clone(),
                kind,
            });
            if seen.insert(target.spec_id.clone()) {
                nodes.push(DepNode {
                    spec_id: target.spec_id.clone(),
                    status: target.status.clone(),
                    depth: depth + 1,
                });
                queue.push_back((target.spec_id.clone(), depth + 1));
            }
        }
    }

    DepGraph {
        nodes,
        edges,
        truncated,
    }
}

// =========================================================================
// V2: Cycle-time histogram — days from Approved → Completed.
// trace:EPIC-29 | ai:claude
// =========================================================================

/// Bucket boundaries (inclusive lower, inclusive upper, except the
/// last which is open-ended). Matches the brief verbatim: 0-7, 8-14,
/// 15-30, 31-60, 60+.
pub const CYCLE_TIME_BUCKETS: &[(&str, u64, Option<u64>)] = &[
    ("0–7", 0, Some(7)),
    ("8–14", 8, Some(14)),
    ("15–30", 15, Some(30)),
    ("31–60", 31, Some(60)),
    ("60+", 61, None),
];

#[derive(Debug, Clone, PartialEq)]
pub struct CycleTimeBucket {
    pub label: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CycleTimeStats {
    pub buckets: Vec<CycleTimeBucket>,
    /// Total number of items contributing to the histogram.
    pub sample_size: u32,
    /// Median cycle time in days, when `sample_size > 0`.
    pub median_days: Option<f64>,
    /// 90th-percentile cycle time in days.
    pub p90_days: Option<f64>,
}

/// Bucket all items that completed within `window_days` of `today` by
/// their Approved → Completed duration. Items without BOTH transitions
/// in their history are excluded (strict reading of the brief — empty
/// state if the project doesn't journal Approved transitions).
pub fn compute_cycle_time(items: &[&Requirement], today: &str, window_days: u32) -> CycleTimeStats {
    let cutoff = add_days(today, -(window_days as i64));
    let mut days: Vec<u64> = Vec::new();
    for r in items {
        let Some((approved, completed)) = approved_to_completed_dates(r) else {
            continue;
        };
        if completed.as_str() < cutoff.as_str() {
            continue;
        }
        let d = days_between(&approved, &completed) as u64;
        days.push(d);
    }
    let sample_size = days.len() as u32;
    let mut buckets: Vec<CycleTimeBucket> = CYCLE_TIME_BUCKETS
        .iter()
        .map(|(label, _, _)| CycleTimeBucket {
            label: (*label).to_string(),
            count: 0,
        })
        .collect();
    for &d in &days {
        for (i, (_, lo, hi)) in CYCLE_TIME_BUCKETS.iter().enumerate() {
            let in_bucket = match hi {
                Some(h) => d >= *lo && d <= *h,
                None => d >= *lo,
            };
            if in_bucket {
                buckets[i].count += 1;
                break;
            }
        }
    }
    let (median_days, p90_days) = if days.is_empty() {
        (None, None)
    } else {
        let mut sorted = days.clone();
        sorted.sort();
        (
            Some(percentile(&sorted, 0.50)),
            Some(percentile(&sorted, 0.90)),
        )
    };
    CycleTimeStats {
        buckets,
        sample_size,
        median_days,
        p90_days,
    }
}

fn approved_to_completed_dates(r: &Requirement) -> Option<(String, String)> {
    let mut approved: Option<String> = None;
    let mut completed: Option<String> = None;
    for entry in &r.history {
        for change in &entry.changes {
            if change.field_name != "status" {
                continue;
            }
            let new_v = match &change.new_value {
                serde_yaml::Value::String(s) => s.as_str(),
                _ => continue,
            };
            let date = ymd_prefix(&entry.timestamp);
            if new_v == "Approved" && approved.is_none() {
                approved = Some(date);
            } else if new_v == "Completed" && completed.is_none() {
                completed = Some(date);
            }
        }
    }
    match (approved, completed) {
        (Some(a), Some(c)) if a.as_str() <= c.as_str() => Some((a, c)),
        _ => None,
    }
}

/// Percentile (`p` in [0,1]) over a pre-sorted ascending slice via
/// linear interpolation. Returns `0.0` for an empty slice.
fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    if n == 1 {
        return sorted[0] as f64;
    }
    let rank = p * (n as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let frac = rank - lo as f64;
    sorted[lo] as f64 + (sorted[hi] - sorted[lo]) as f64 * frac
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
        assert!(compute_cfd(&empty, "2026-05-25", 30).is_empty());
        let ct = compute_cycle_time(&empty, "2026-05-25", 90);
        assert_eq!(ct.sample_size, 0);
        assert_eq!(ct.median_days, None);
    }

    // =====================================================================
    // V2: CFD tests
    // =====================================================================

    fn r_history(spec: &str, created: &str, transitions: &[(&str, &str, &str)]) -> Requirement {
        // Each transition = (timestamp, old, new).
        let mut history = vec![];
        for (ts, old, new) in transitions {
            history.push(HistoryEntry {
                timestamp: format!("{ts}T12:00:00Z"),
                author: String::new(),
                changes: vec![Change {
                    field_name: "status".into(),
                    old_value: serde_yaml::Value::String((*old).into()),
                    new_value: serde_yaml::Value::String((*new).into()),
                }],
            });
        }
        let final_status = transitions
            .last()
            .map(|(_, _, n)| (*n).to_string())
            .unwrap_or_else(|| "Draft".into());
        Requirement {
            id: format!("uuid-{spec}"),
            spec_id: spec.into(),
            title: spec.into(),
            status: final_status,
            req_type: "Story".into(),
            feature: None,
            weight: None,
            created_at: format!("{created}T00:00:00Z"),
            modified_at: format!("{created}T00:00:00Z"),
            relationships: vec![],
            history,
            custom_fields: HashMap::new(),
        }
    }

    #[test]
    fn cfd_replays_status_per_day() {
        // S-1: Draft on 2026-05-20 → Approved on 2026-05-22 →
        //      Completed on 2026-05-24.
        let s = r_history(
            "S-1",
            "2026-05-20",
            &[
                ("2026-05-22", "Draft", "Approved"),
                ("2026-05-24", "Approved", "Completed"),
            ],
        );
        let items = vec![&s];
        let cfd = compute_cfd(&items, "2026-05-25", 7);
        // 7 days ending on 2026-05-25 → window starts 2026-05-19.
        assert_eq!(cfd.len(), 7);
        assert_eq!(cfd[0].date, "2026-05-19");
        // Day 0 (pre-creation): nothing.
        assert!(cfd[0].by_status.is_empty());
        // Day 1 (2026-05-20, created): Draft (initial = old_value of first change).
        assert_eq!(cfd[1].by_status.get("Draft"), Some(&1));
        // Day 3 (2026-05-22): Approved.
        assert_eq!(cfd[3].by_status.get("Approved"), Some(&1));
        assert!(cfd[3].by_status.get("Draft").is_none());
        // Day 5 (2026-05-24): Completed.
        assert_eq!(cfd[5].by_status.get("Completed"), Some(&1));
        assert_eq!(cfd[6].by_status.get("Completed"), Some(&1));
    }

    #[test]
    fn cfd_handles_items_with_no_history() {
        // Item with current status only and no journal — should appear
        // in Draft (or whatever its current status is) from creation.
        let mut s = r("S-2", "InProgress", None);
        s.created_at = "2026-05-22T00:00:00Z".into();
        let items = vec![&s];
        let cfd = compute_cfd(&items, "2026-05-25", 7);
        assert!(cfd[0].by_status.is_empty()); // pre-creation
        assert_eq!(cfd[3].by_status.get("InProgress"), Some(&1)); // 2026-05-22
        assert_eq!(cfd[6].by_status.get("InProgress"), Some(&1));
    }

    // =====================================================================
    // V2: Dep-graph tests
    // =====================================================================

    fn rel(target_uuid: &str, kind: &str) -> super::super::store::Relationship {
        use serde_yaml::value::{Tag, TaggedValue};
        super::super::store::Relationship {
            rel_type: serde_yaml::Value::Tagged(Box::new(TaggedValue {
                tag: Tag::new("Custom"),
                value: serde_yaml::Value::String(kind.into()),
            })),
            target_id: target_uuid.into(),
        }
    }

    #[test]
    fn dep_graph_bfs_to_depth_limit() {
        // EPIC-1 → STORY-1 → TASK-1 (chain of length 2 from the root)
        let mut epic = r("EPIC-1", "Approved", None);
        epic.relationships = vec![rel("uuid-STORY-1", "parent_of")];
        let mut story = r("STORY-1", "InProgress", None);
        story.relationships = vec![rel("uuid-TASK-1", "parent_of")];
        let task = r("TASK-1", "Completed", None);
        let items: Vec<&Requirement> = vec![&epic, &story, &task];
        let by_spec = |s: &str| items.iter().copied().find(|r| r.spec_id == s);
        let by_uuid = |u: &str| items.iter().copied().find(|r| r.id == u);

        // Depth 1: only EPIC-1 + STORY-1, with truncated=true.
        let g = compute_dep_graph("EPIC-1", 1, by_spec, by_uuid);
        let names: Vec<&str> = g.nodes.iter().map(|n| n.spec_id.as_str()).collect();
        assert_eq!(names, vec!["EPIC-1", "STORY-1"]);
        assert!(g.truncated);

        // Depth 2: all three.
        let g = compute_dep_graph("EPIC-1", 2, by_spec, by_uuid);
        let names: Vec<&str> = g.nodes.iter().map(|n| n.spec_id.as_str()).collect();
        assert_eq!(names, vec!["EPIC-1", "STORY-1", "TASK-1"]);
        assert!(!g.truncated);

        // Edges are populated with normalized rel-kind.
        assert_eq!(g.edges.len(), 2);
        assert_eq!(g.edges[0].kind, "Custom:parent_of");
    }

    #[test]
    fn dep_graph_handles_cycles() {
        // A → B → A (cycle). BFS must terminate.
        let mut a = r("A", "Draft", None);
        a.relationships = vec![rel("uuid-B", "depends_on")];
        let mut b = r("B", "Draft", None);
        b.relationships = vec![rel("uuid-A", "depends_on")];
        let items: Vec<&Requirement> = vec![&a, &b];
        let by_spec = |s: &str| items.iter().copied().find(|r| r.spec_id == s);
        let by_uuid = |u: &str| items.iter().copied().find(|r| r.id == u);
        let g = compute_dep_graph("A", 5, by_spec, by_uuid);
        let names: Vec<String> = g.nodes.iter().map(|n| n.spec_id.clone()).collect();
        assert_eq!(names, vec!["A", "B"]);
        // Cycle produces 2 edges (A→B, B→A) but only 2 nodes.
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn dep_graph_unknown_root_returns_empty() {
        let g = compute_dep_graph("NOPE-1", 2, |_| None, |_| None);
        assert!(g.nodes.is_empty());
        assert!(g.edges.is_empty());
    }

    // =====================================================================
    // V2: Cycle-time tests
    // =====================================================================

    #[test]
    fn cycle_time_buckets_approved_to_completed() {
        let a = r_history(
            "S-1",
            "2026-05-01",
            &[
                ("2026-05-02", "Draft", "Approved"),
                ("2026-05-05", "Approved", "Completed"),
            ],
        ); // 3 days → "0-7"
        let b = r_history(
            "S-2",
            "2026-05-01",
            &[
                ("2026-05-02", "Draft", "Approved"),
                ("2026-05-15", "Approved", "Completed"),
            ],
        ); // 13 days → "8-14"
        let c = r_history(
            "S-3",
            "2026-04-01",
            &[
                ("2026-04-02", "Draft", "Approved"),
                ("2026-05-20", "Approved", "Completed"),
            ],
        ); // 48 days → "31-60"
        let d_no_approved = r_history(
            "S-4",
            "2026-05-01",
            &[("2026-05-05", "Draft", "Completed")],
        ); // skipped (no Approved transition)
        let items = vec![&a, &b, &c, &d_no_approved];
        let stats = compute_cycle_time(&items, "2026-05-25", 90);
        assert_eq!(stats.sample_size, 3);
        // S-1 (3d) in bucket 0
        assert_eq!(stats.buckets[0].count, 1);
        // S-2 (13d) in bucket 1
        assert_eq!(stats.buckets[1].count, 1);
        // S-3 (48d) in bucket 3 (31-60)
        assert_eq!(stats.buckets[3].count, 1);
        // No items in "60+"
        assert_eq!(stats.buckets[4].count, 0);
        // Median: middle of [3, 13, 48] = 13
        assert_eq!(stats.median_days, Some(13.0));
        // P90 with linear interp: rank = 0.9 * 2 = 1.8 → 13 + 0.8 * (48-13) = 41
        assert_eq!(stats.p90_days, Some(41.0));
    }

    #[test]
    fn cycle_time_window_filter_drops_old_items() {
        let recent = r_history(
            "S-1",
            "2026-05-01",
            &[
                ("2026-05-02", "Draft", "Approved"),
                ("2026-05-20", "Approved", "Completed"),
            ],
        );
        let ancient = r_history(
            "S-2",
            "2025-01-01",
            &[
                ("2025-01-02", "Draft", "Approved"),
                ("2025-01-10", "Approved", "Completed"),
            ],
        );
        let items = vec![&recent, &ancient];
        let stats = compute_cycle_time(&items, "2026-05-25", 30);
        assert_eq!(stats.sample_size, 1, "ancient item should be filtered out");
    }
}
