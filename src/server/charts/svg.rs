// trace:EPIC-29 | ai:claude
//
// Hand-crafted SVG chart renderers. Ported visual ethos from
// `~/ai/aida/aida-web-react/src/components/{sprint/charts,dashboard}/*.tsx`:
// fixed 400×220 viewBox, 36/20/20/30 (L/R/T/B) padding for axis labels,
// gridlines at 0/25/50/75/100% of maxY with 0.08 opacity, data dots r=2.5,
// stroke widths 2 (actual) / 1.5 dashed (ideal/scope).
//
// Native to aida-chat's dark theme via inline-styled `<svg style="color:var(--text-dim)">`
// — `currentColor` references the chat's `--text-dim` token, gridlines inherit
// cleanly. Chart-specific colors (blue, emerald, amber, violet) are explicit hex
// because the aida-chat palette doesn't expose a comparable categorical scale yet.
//
// Each renderer takes a parsed data slice and produces a complete `<svg>` element.
// Empty-state branches return a small `<svg>` with a centered muted message — never
// a panic, never a blank string.

use std::collections::BTreeMap;
use std::fmt::Write;

use thiserror::Error;

use super::data::{
    status_color, BurndownPoint, BurnupPoint, CfdPoint, CycleTimeStats, DepGraph,
    FeatureProgressRow, StatusCounts, VelocityPoint, STATUS_ORDER,
};

#[derive(Debug, Error)]
pub enum SvgError {
    #[error("svg fmt: {0}")]
    Fmt(#[from] std::fmt::Error),
}

const W: f64 = 400.0;
const H: f64 = 220.0;
const PAD_TOP: f64 = 24.0;
const PAD_RIGHT: f64 = 20.0;
const PAD_BOTTOM: f64 = 32.0;
const PAD_LEFT: f64 = 36.0;
const CHART_W: f64 = W - PAD_LEFT - PAD_RIGHT;
const CHART_H: f64 = H - PAD_TOP - PAD_BOTTOM;

const SVG_OPEN: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" "#,
    r#"viewBox="0 0 400 220" "#,
    r#"role="img" "#,
    r#"style="width:100%;max-width:480px;color:var(--text-dim,#8b93a3);"#,
    r#"font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:9px;""#,
    r#">"#,
);

// =========================================================================
// Shared helpers
// =========================================================================

fn grid(out: &mut String, max_y: f64) -> std::fmt::Result {
    // Horizontal grid lines + Y labels at 0/25/50/75/100% of max_y.
    for frac in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = PAD_TOP + CHART_H * (1.0 - frac);
        writeln!(
            out,
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="currentColor" stroke-opacity="0.08"/>"#,
            PAD_LEFT,
            y,
            W - PAD_RIGHT,
            y
        )?;
        let label = (max_y * frac).round() as i64;
        writeln!(
            out,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="currentColor" font-size="9">{label}</text>"#,
            PAD_LEFT - 4.0,
            y + 3.0
        )?;
    }
    Ok(())
}

fn title(out: &mut String, text: &str) -> std::fmt::Result {
    writeln!(
        out,
        r#"<text x="{:.1}" y="14" fill="currentColor" font-size="11" font-weight="600" letter-spacing="0.04em">{}</text>"#,
        PAD_LEFT - 6.0,
        escape_xml(text)
    )
}

fn empty(message: &str) -> String {
    let mut s = String::with_capacity(256);
    s.push_str(SVG_OPEN);
    let _ = writeln!(
        s,
        r#"<rect x="0.5" y="0.5" width="399" height="219" fill="none" stroke="currentColor" stroke-opacity="0.12" stroke-dasharray="4 4" rx="8"/>"#
    );
    let _ = writeln!(
        s,
        r#"<text x="200" y="110" text-anchor="middle" fill="currentColor" font-size="11" font-style="italic">{}</text>"#,
        escape_xml(message)
    );
    s.push_str("</svg>");
    s
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// =========================================================================
// 1. Status donut
// =========================================================================

pub fn render_status_svg(counts: &StatusCounts) -> Result<String, SvgError> {
    if counts.total == 0 {
        return Ok(empty("No requirements to chart."));
    }

    let mut s = String::with_capacity(2048);
    s.push_str(SVG_OPEN);
    title(&mut s, "Status distribution")?;

    // Donut geometry: center on the left third; legend on the right.
    let cx = 92.0;
    let cy = 118.0;
    let r_outer = 60.0;
    let r_inner = 36.0;

    let total = counts.total as f64;
    let mut cumulative = 0.0;
    for (status, n) in &counts.buckets {
        let frac_start = cumulative / total;
        cumulative += *n as f64;
        let frac_end = cumulative / total;
        let color = status_color(status);
        let path = donut_arc_path(cx, cy, r_inner, r_outer, frac_start, frac_end);
        writeln!(
            s,
            r#"<path d="{path}" fill="{color}" stroke="var(--bg,#0f1115)" stroke-width="1"/>"#
        )?;
    }

    // Center total label.
    writeln!(
        s,
        r#"<text x="{cx:.1}" y="{cy:.1}" text-anchor="middle" dy="0" fill="var(--text,#e6e8ee)" font-size="22" font-weight="700">{}</text>"#,
        counts.total
    )?;
    writeln!(
        s,
        r#"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9" letter-spacing="0.08em">TOTAL</text>"#,
        cy + 14.0
    )?;

    // Legend on the right.
    let legend_x = 184.0;
    let mut legend_y = PAD_TOP + 12.0;
    for (status, n) in &counts.buckets {
        let pct = ((*n as f64 / total) * 100.0).round() as i64;
        let color = status_color(status);
        writeln!(
            s,
            r#"<rect x="{legend_x:.1}" y="{:.1}" width="9" height="9" rx="2" fill="{color}"/>"#,
            legend_y - 8.0
        )?;
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="var(--text,#e6e8ee)" font-size="10" font-family="ui-sans-serif,system-ui,sans-serif">{}</text>"#,
            legend_x + 14.0,
            legend_y,
            escape_xml(status)
        )?;
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="currentColor" font-size="10">{} ({pct}%)</text>"#,
            W - PAD_RIGHT,
            legend_y,
            n
        )?;
        legend_y += 18.0;
    }

    s.push_str("</svg>");
    Ok(s)
}

/// Path data for a donut segment from `frac_start` to `frac_end` (each
/// in [0, 1]). Starts at 12 o'clock and goes clockwise.
fn donut_arc_path(
    cx: f64,
    cy: f64,
    r_inner: f64,
    r_outer: f64,
    frac_start: f64,
    frac_end: f64,
) -> String {
    // Full-circle case: render as a ring with two arcs that don't
    // collapse to a 0-degree wedge.
    if (frac_end - frac_start - 1.0).abs() < 1e-6 {
        let outer_top = (cx, cy - r_outer);
        let outer_bot = (cx, cy + r_outer);
        let inner_top = (cx, cy - r_inner);
        let inner_bot = (cx, cy + r_inner);
        return format!(
            "M {} {} A {r_outer} {r_outer} 0 1 1 {} {} A {r_outer} {r_outer} 0 1 1 {} {} \
             M {} {} A {r_inner} {r_inner} 0 1 0 {} {} A {r_inner} {r_inner} 0 1 0 {} {} Z",
            outer_top.0,
            outer_top.1,
            outer_bot.0,
            outer_bot.1,
            outer_top.0,
            outer_top.1,
            inner_top.0,
            inner_top.1,
            inner_bot.0,
            inner_bot.1,
            inner_top.0,
            inner_top.1,
        );
    }

    let a_start = frac_start * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
    let a_end = frac_end * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
    let p1 = (cx + r_outer * a_start.cos(), cy + r_outer * a_start.sin());
    let p2 = (cx + r_outer * a_end.cos(), cy + r_outer * a_end.sin());
    let p3 = (cx + r_inner * a_end.cos(), cy + r_inner * a_end.sin());
    let p4 = (cx + r_inner * a_start.cos(), cy + r_inner * a_start.sin());
    let large = if (frac_end - frac_start) > 0.5 { 1 } else { 0 };
    format!(
        "M {:.2} {:.2} A {r_outer} {r_outer} 0 {large} 1 {:.2} {:.2} L {:.2} {:.2} A {r_inner} {r_inner} 0 {large} 0 {:.2} {:.2} Z",
        p1.0, p1.1, p2.0, p2.1, p3.0, p3.1, p4.0, p4.1
    )
}

// =========================================================================
// 2. Burn-down
// =========================================================================

pub fn render_burndown_svg(points: &[BurndownPoint]) -> Result<String, SvgError> {
    if points.len() < 2 {
        return Ok(empty(
            "Not enough data for burndown — sprint needs a start_date and end_date in custom_fields.",
        ));
    }
    let max_y = points
        .iter()
        .map(|p| p.remaining.max(p.ideal))
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let n = points.len();
    let x_at = |i: usize| PAD_LEFT + (i as f64 / (n - 1) as f64) * CHART_W;
    let y_at = |v: f64| PAD_TOP + CHART_H - (v / max_y) * CHART_H;

    let mut s = String::with_capacity(4096);
    s.push_str(SVG_OPEN);
    title(&mut s, "Burn-down")?;
    grid(&mut s, max_y)?;

    // Ideal line (dashed gray).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.ideal,
        LineStyle {
            color: "#6b7280",
            width: 1.5,
            dashed: true,
        },
    )?;
    // Actual line (solid blue).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.remaining,
        LineStyle {
            color: "#3b82f6",
            width: 2.0,
            dashed: false,
        },
    )?;

    // Data dots on actual.
    for (i, p) in points.iter().enumerate() {
        writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="2.5" fill="#3b82f6"/>"##,
            x_at(i),
            y_at(p.remaining)
        )?;
    }

    // X-axis labels: first, middle, last.
    let label_idxs = [0, n / 2, n - 1];
    for i in label_idxs {
        let date = &points[i].date;
        let short = if date.len() >= 10 { &date[5..10] } else { date };
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9">{}</text>"#,
            x_at(i),
            H - 6.0,
            escape_xml(short)
        )?;
    }

    // Legend.
    let legend_y = H - 6.0;
    legend_swatch(
        &mut s,
        PAD_LEFT + CHART_W * 0.5 - 60.0,
        legend_y - 14.0,
        "#3b82f6",
        "Actual",
        false,
    )?;
    legend_swatch(
        &mut s,
        PAD_LEFT + CHART_W * 0.5 + 12.0,
        legend_y - 14.0,
        "#6b7280",
        "Ideal",
        true,
    )?;

    s.push_str("</svg>");
    Ok(s)
}

// =========================================================================
// 3. Burn-up
// =========================================================================

pub fn render_burnup_svg(points: &[BurnupPoint]) -> Result<String, SvgError> {
    if points.len() < 2 {
        return Ok(empty(
            "Not enough data for burn-up — sprint needs a start_date and end_date in custom_fields.",
        ));
    }
    let max_y = points
        .iter()
        .map(|p| p.completed.max(p.scope))
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let n = points.len();
    let x_at = |i: usize| PAD_LEFT + (i as f64 / (n - 1) as f64) * CHART_W;
    let y_at = |v: f64| PAD_TOP + CHART_H - (v / max_y) * CHART_H;

    let mut s = String::with_capacity(4096);
    s.push_str(SVG_OPEN);
    title(&mut s, "Burn-up")?;
    grid(&mut s, max_y)?;

    // Completed area (emerald, alpha 0.12) — polygon under the line.
    let mut area = format!("{:.1},{:.1} ", x_at(0), y_at(0.0));
    for (i, p) in points.iter().enumerate() {
        area.push_str(&format!("{:.1},{:.1} ", x_at(i), y_at(p.completed)));
    }
    area.push_str(&format!("{:.1},{:.1}", x_at(n - 1), y_at(0.0)));
    writeln!(
        s,
        r##"<polygon points="{area}" fill="#10b981" fill-opacity="0.12"/>"##
    )?;

    // Scope line (amber dashed).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.scope,
        LineStyle {
            color: "#f59e0b",
            width: 1.5,
            dashed: true,
        },
    )?;
    // Completed line (solid emerald).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.completed,
        LineStyle {
            color: "#10b981",
            width: 2.0,
            dashed: false,
        },
    )?;

    for (i, p) in points.iter().enumerate() {
        writeln!(
            s,
            r##"<circle cx="{:.1}" cy="{:.1}" r="2.5" fill="#10b981"/>"##,
            x_at(i),
            y_at(p.completed)
        )?;
    }

    let label_idxs = [0, n / 2, n - 1];
    for i in label_idxs {
        let date = &points[i].date;
        let short = if date.len() >= 10 { &date[5..10] } else { date };
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9">{}</text>"#,
            x_at(i),
            H - 6.0,
            escape_xml(short)
        )?;
    }

    let legend_y = H - 6.0;
    legend_swatch(
        &mut s,
        PAD_LEFT + CHART_W * 0.5 - 70.0,
        legend_y - 14.0,
        "#10b981",
        "Completed",
        false,
    )?;
    legend_swatch(
        &mut s,
        PAD_LEFT + CHART_W * 0.5 + 14.0,
        legend_y - 14.0,
        "#f59e0b",
        "Scope",
        true,
    )?;

    s.push_str("</svg>");
    Ok(s)
}

// =========================================================================
// 4. Velocity
// =========================================================================

pub fn render_velocity_svg(points: &[VelocityPoint]) -> Result<String, SvgError> {
    if points.is_empty() {
        return Ok(empty(
            "No sprints found. Velocity needs requirements of type=Sprint.",
        ));
    }
    let max_y = points
        .iter()
        .map(|p| p.points as f64)
        .fold(1.0_f64, f64::max);
    let avg = if points.is_empty() {
        0.0
    } else {
        points.iter().map(|p| p.points as f64).sum::<f64>() / points.len() as f64
    };

    let bar_gap = 6.0;
    let bar_w = ((CHART_W - bar_gap * (points.len() as f64 - 1.0)) / points.len() as f64).min(48.0);
    let total_w = points.len() as f64 * bar_w + (points.len() as f64 - 1.0) * bar_gap;
    let offset_x = PAD_LEFT + (CHART_W - total_w) / 2.0;
    let y_at = |v: f64| PAD_TOP + CHART_H - (v / max_y) * CHART_H;

    let mut s = String::with_capacity(2048);
    s.push_str(SVG_OPEN);
    title(&mut s, "Velocity")?;
    grid(&mut s, max_y)?;

    // Average line.
    if avg > 0.0 {
        writeln!(
            s,
            r##"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="#a855f7" stroke-width="1.5" stroke-dasharray="6 3" stroke-opacity="0.75"/>"##,
            PAD_LEFT,
            y_at(avg),
            W - PAD_RIGHT,
            y_at(avg)
        )?;
    }

    // Bars + labels.
    for (i, p) in points.iter().enumerate() {
        let bx = offset_x + i as f64 * (bar_w + bar_gap);
        let bh = (p.points as f64 / max_y) * CHART_H;
        let by = PAD_TOP + CHART_H - bh;
        writeln!(
            s,
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="3" fill="#8b5cf6" fill-opacity="0.85"/>"##,
            bx, by, bar_w, bh
        )?;
        if p.points > 0 {
            writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="var(--text,#e6e8ee)" font-size="10" font-weight="600">{}</text>"#,
                bx + bar_w / 2.0,
                by - 4.0,
                p.points
            )?;
        }
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9">{}</text>"#,
            bx + bar_w / 2.0,
            H - 6.0,
            escape_xml(&p.label)
        )?;
    }

    if avg > 0.0 {
        writeln!(
            s,
            r##"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="#a855f7" font-size="9">avg {:.0}</text>"##,
            W - PAD_RIGHT - 2.0,
            y_at(avg) - 4.0,
            avg
        )?;
    }

    s.push_str("</svg>");
    Ok(s)
}

// =========================================================================
// 5. Feature progress (horizontal bars)
// =========================================================================

pub fn render_feature_progress_svg(rows: &[FeatureProgressRow]) -> Result<String, SvgError> {
    if rows.is_empty() {
        return Ok(empty("No requirements grouped by feature."));
    }
    // Auto-size H to fit; cap rows shown to 8 to keep the chart compact.
    let shown: Vec<&FeatureProgressRow> = rows.iter().take(8).collect();
    let n = shown.len();
    let row_h = 22.0;
    let panel_h = PAD_TOP + n as f64 * row_h + 8.0;
    let viewbox_h = panel_h + 8.0;

    let mut s = String::with_capacity(2048);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 {viewbox_h:.0}" role="img" style="width:100%;max-width:480px;color:var(--text-dim,#8b93a3);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:9px;">"#
    );
    title(&mut s, "Feature progress")?;

    let label_x = PAD_LEFT;
    let bar_x = PAD_LEFT + 110.0;
    let bar_w = W - PAD_RIGHT - bar_x;

    for (i, row) in shown.iter().enumerate() {
        let y = PAD_TOP + i as f64 * row_h + 4.0;
        let pct = row.percent() as f64;
        let fill_w = bar_w * pct / 100.0;
        // Truncate long feature names so the bar starts at a fixed x.
        let mut name: String = row.feature.chars().take(16).collect();
        if row.feature.chars().count() > 16 {
            name.push('…');
        }
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="var(--text,#e6e8ee)" font-size="10" font-family="ui-sans-serif,system-ui,sans-serif">{}</text>"#,
            label_x,
            y + 6.0,
            escape_xml(&name)
        )?;
        // Track.
        writeln!(
            s,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="10" rx="5" fill="currentColor" fill-opacity="0.08"/>"#,
            bar_x, y, bar_w
        )?;
        // Fill.
        writeln!(
            s,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="10" rx="5" fill="var(--accent,#7c9cff)"/>"#,
            bar_x, y, fill_w
        )?;
        // Counts on the right.
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="currentColor" font-size="9">{} / {} · {:.0}%</text>"#,
            W - PAD_RIGHT,
            y - 1.0,
            row.completed,
            row.total,
            pct
        )?;
    }
    if rows.len() > shown.len() {
        let trailer_y = PAD_TOP + shown.len() as f64 * row_h + 14.0;
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="currentColor" font-size="9" font-style="italic">+{} more features…</text>"#,
            PAD_LEFT,
            trailer_y,
            rows.len() - shown.len()
        )?;
    }

    s.push_str("</svg>");
    Ok(s)
}

// =========================================================================
// Helpers shared across line charts
// =========================================================================

struct LineStyle<'a> {
    color: &'a str,
    width: f64,
    dashed: bool,
}

fn write_polyline<T>(
    out: &mut String,
    points: &[T],
    x_at: &dyn Fn(usize) -> f64,
    y_at: &dyn Fn(f64) -> f64,
    project: impl Fn(&T) -> f64,
    style: LineStyle<'_>,
) -> std::fmt::Result {
    let mut pts = String::with_capacity(points.len() * 12);
    for (i, p) in points.iter().enumerate() {
        if !pts.is_empty() {
            pts.push(' ');
        }
        let _ = write!(pts, "{:.1},{:.1}", x_at(i), y_at(project(p)));
    }
    let dash_attr = if style.dashed {
        r#" stroke-dasharray="6 3""#
    } else {
        ""
    };
    let color = style.color;
    let width = style.width;
    writeln!(
        out,
        r#"<polyline points="{pts}" fill="none" stroke="{color}" stroke-width="{width}"{dash_attr}/>"#
    )?;
    Ok(())
}

// =========================================================================
// V2 renderers
// =========================================================================

// trace:EPIC-29 | ai:claude
/// Stacked-area chart of status counts over time. Each status renders
/// as its own band, in canonical `STATUS_ORDER`. Y-axis = stack total
/// (which IS the total count of items for that day).
pub fn render_cfd_svg(points: &[CfdPoint]) -> Result<String, SvgError> {
    if points.len() < 2 {
        return Ok(empty(
            "Not enough data for CFD — window must be at least 2 days.",
        ));
    }

    // Collect the set of statuses present anywhere in the window so the
    // legend doesn't list zero-only statuses.
    let mut active_statuses: Vec<String> = Vec::new();
    for &canon in STATUS_ORDER {
        if points
            .iter()
            .any(|p| p.by_status.get(canon).copied().unwrap_or(0) > 0)
        {
            active_statuses.push(canon.to_string());
        }
    }
    // Then append unknown statuses for completeness.
    let mut other_keys: Vec<String> = Vec::new();
    for p in points {
        for k in p.by_status.keys() {
            if !STATUS_ORDER.contains(&k.as_str()) && !other_keys.contains(k) {
                other_keys.push(k.clone());
            }
        }
    }
    active_statuses.extend(other_keys);

    if active_statuses.is_empty() {
        return Ok(empty(
            "No requirements with status data in the window. CFD needs at least one item.",
        ));
    }

    let max_y = points
        .iter()
        .map(|p| p.by_status.values().map(|v| *v as f64).sum())
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let n = points.len();
    let x_at = |i: usize| PAD_LEFT + (i as f64 / (n - 1) as f64) * CHART_W;
    let y_at = |v: f64| PAD_TOP + CHART_H - (v / max_y) * CHART_H;

    let mut s = String::with_capacity(8192);
    s.push_str(SVG_OPEN);
    title(&mut s, "Cumulative flow")?;
    grid(&mut s, max_y)?;

    // Build cumulative-bottom map per day so each band stacks on the
    // previous. Render bands in reverse order so earlier statuses
    // (Draft → Approved → …) appear at the bottom of the stack.
    let mut cum_at_day: Vec<f64> = vec![0.0; n];
    for status in &active_statuses {
        let color = status_color(status);
        // Top polygon line: cum + count(status).
        let mut tops: Vec<f64> = Vec::with_capacity(n);
        for (i, p) in points.iter().enumerate() {
            let count = p.by_status.get(status).copied().unwrap_or(0) as f64;
            cum_at_day[i] += count;
            tops.push(cum_at_day[i]);
        }
        let bottoms: Vec<f64> = tops
            .iter()
            .enumerate()
            .map(|(i, t)| t - points[i].by_status.get(status).copied().unwrap_or(0) as f64)
            .collect();
        // Polygon: top edge left-to-right, then bottom edge right-to-left.
        let mut poly = String::with_capacity(n * 24);
        for (i, t) in tops.iter().enumerate() {
            if !poly.is_empty() {
                poly.push(' ');
            }
            let _ = write!(poly, "{:.1},{:.1}", x_at(i), y_at(*t));
        }
        for (i, b) in bottoms.iter().enumerate().rev() {
            poly.push(' ');
            let _ = write!(poly, "{:.1},{:.1}", x_at(i), y_at(*b));
        }
        writeln!(
            s,
            r#"<polygon points="{poly}" fill="{color}" fill-opacity="0.78" stroke="{color}" stroke-width="0.5"/>"#
        )?;
    }

    // X-axis labels: start / middle / end (MM-DD).
    let label_idxs = [0, n / 2, n - 1];
    for i in label_idxs {
        let date = &points[i].date;
        let short = if date.len() >= 10 { &date[5..10] } else { date };
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9">{}</text>"#,
            x_at(i),
            H - 6.0,
            escape_xml(short)
        )?;
    }

    // Legend below the chart, two columns when more than 4 statuses.
    let legend_y0 = H - 6.0;
    let cols = if active_statuses.len() > 4 { 2 } else { 1 };
    let per_col = active_statuses.len().div_ceil(cols);
    for (i, status) in active_statuses.iter().enumerate() {
        let col = i / per_col;
        let row = i % per_col;
        let lx = PAD_LEFT + col as f64 * (CHART_W / cols as f64);
        let ly =
            legend_y0 - 14.0 - (per_col - 1 - row) as f64 * 11.0 + (per_col as f64 - 1.0) * 11.0;
        let color = status_color(status);
        writeln!(
            s,
            r#"<rect x="{lx:.1}" y="{:.1}" width="8" height="8" rx="1.5" fill="{color}"/>"#,
            ly - 7.0
        )?;
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="currentColor" font-size="9">{}</text>"#,
            lx + 12.0,
            ly,
            escape_xml(status)
        )?;
    }

    s.push_str("</svg>");
    Ok(s)
}

// trace:EPIC-29 | ai:claude
/// Hierarchical dependency-graph layout: columns by depth, nodes
/// distributed vertically within each column. Edges drawn as light
/// curves. Node fill color = status. Compact enough to fit inline in a
/// chat message.
pub fn render_dep_graph_svg(graph: &DepGraph) -> Result<String, SvgError> {
    if graph.nodes.is_empty() {
        return Ok(empty(
            "Dependency graph empty — unknown SPEC-ID, or no outgoing relationships.",
        ));
    }

    // Group nodes by depth.
    let max_depth = graph.nodes.iter().map(|n| n.depth).max().unwrap_or(0);
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); max_depth as usize + 1];
    for (idx, node) in graph.nodes.iter().enumerate() {
        columns[node.depth as usize].push(idx);
    }
    let max_col_size = columns.iter().map(|c| c.len()).max().unwrap_or(1).max(1);

    let cols = columns.len() as f64;
    let node_w = 90.0_f64;
    let node_h = 28.0_f64;
    let h_gap = 14.0_f64;
    let v_gap = 12.0_f64;

    // Auto-size canvas to fit the densest column + the column count.
    let width = (PAD_LEFT + PAD_RIGHT + cols * node_w + (cols - 1.0) * h_gap).max(W);
    let height =
        (PAD_TOP + PAD_BOTTOM + max_col_size as f64 * node_h + (max_col_size as f64 - 1.0) * v_gap)
            .max(160.0);

    let mut s = String::with_capacity(4096);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.0} {height:.0}" role="img" style="width:100%;max-width:560px;color:var(--text-dim,#8b93a3);font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:9px;">"#
    );
    title(&mut s, "Dependency graph")?;

    // Compute node positions.
    let col_xs: Vec<f64> = (0..columns.len())
        .map(|i| PAD_LEFT + i as f64 * (node_w + h_gap))
        .collect();
    let mut node_pos: Vec<(f64, f64)> = vec![(0.0, 0.0); graph.nodes.len()];
    for (col_idx, col) in columns.iter().enumerate() {
        let col_count = col.len() as f64;
        let total_h = col_count * node_h + (col_count - 1.0) * v_gap;
        let y0 = PAD_TOP + (height - PAD_TOP - PAD_BOTTOM - total_h) / 2.0;
        for (row_idx, &node_idx) in col.iter().enumerate() {
            let cx = col_xs[col_idx];
            let cy = y0 + row_idx as f64 * (node_h + v_gap);
            node_pos[node_idx] = (cx, cy);
        }
    }

    // Edges first (under nodes).
    let spec_to_idx: BTreeMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.spec_id.as_str(), i))
        .collect();
    for edge in &graph.edges {
        let Some(&fi) = spec_to_idx.get(edge.from_spec.as_str()) else {
            continue;
        };
        let Some(&ti) = spec_to_idx.get(edge.to_spec.as_str()) else {
            continue;
        };
        let (fx, fy) = node_pos[fi];
        let (tx, ty) = node_pos[ti];
        let from_x = fx + node_w;
        let from_y = fy + node_h / 2.0;
        let to_x = tx;
        let to_y = ty + node_h / 2.0;
        let mid_x = (from_x + to_x) / 2.0;
        writeln!(
            s,
            r#"<path d="M {from_x:.1} {from_y:.1} C {mid_x:.1} {from_y:.1}, {mid_x:.1} {to_y:.1}, {to_x:.1} {to_y:.1}" fill="none" stroke="currentColor" stroke-opacity="0.35" stroke-width="1.2"/>"#
        )?;
        // Arrowhead at target.
        writeln!(
            s,
            r#"<polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="currentColor" fill-opacity="0.45"/>"#,
            to_x - 5.0,
            to_y - 3.0,
            to_x,
            to_y,
            to_x - 5.0,
            to_y + 3.0
        )?;
    }

    // Nodes.
    for (i, node) in graph.nodes.iter().enumerate() {
        let (nx, ny) = node_pos[i];
        let fill = status_color(&node.status);
        let label_color = "var(--text,#e6e8ee)";
        let stroke_w = if node.depth == 0 { 1.6 } else { 1.0 };
        writeln!(
            s,
            r#"<rect x="{nx:.1}" y="{ny:.1}" width="{node_w}" height="{node_h}" rx="6" fill="var(--bg,#0f1115)" stroke="{fill}" stroke-width="{stroke_w}"/>"#
        )?;
        // Spec-ID label (top line, mono).
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="{label_color}" font-size="10" font-weight="600">{}</text>"#,
            nx + 8.0,
            ny + 12.0,
            escape_xml(&node.spec_id)
        )?;
        // Status pill (bottom line, small).
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="{fill}" font-size="8" letter-spacing="0.08em">{}</text>"#,
            nx + 8.0,
            ny + 22.0,
            escape_xml(&node.status.to_uppercase())
        )?;
    }

    if graph.truncated {
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="end" fill="currentColor" font-size="9" font-style="italic">truncated at depth {} — ask for deeper</text>"#,
            width - PAD_RIGHT,
            height - 8.0,
            max_depth
        )?;
    }

    s.push_str("</svg>");
    Ok(s)
}

// trace:EPIC-29 | ai:claude
/// Cycle-time histogram: 5 buckets (0-7, 8-14, 15-30, 31-60, 60+) with
/// median and p90 vertical reference lines. Empty-state when no items
/// with both Approved and Completed transitions were found.
pub fn render_cycle_time_svg(stats: &CycleTimeStats) -> Result<String, SvgError> {
    if stats.sample_size == 0 {
        return Ok(empty(
            "No cycle-time samples. Need items with both Approved → Completed transitions in the window.",
        ));
    }
    let max_y = stats
        .buckets
        .iter()
        .map(|b| b.count as f64)
        .fold(1.0_f64, f64::max);

    let n = stats.buckets.len() as f64;
    let bar_gap = 8.0;
    let bar_w = ((CHART_W - bar_gap * (n - 1.0)) / n).min(56.0);
    let total_w = n * bar_w + (n - 1.0) * bar_gap;
    let offset_x = PAD_LEFT + (CHART_W - total_w) / 2.0;

    let mut s = String::with_capacity(2048);
    s.push_str(SVG_OPEN);
    title(&mut s, "Cycle-time histogram")?;
    grid(&mut s, max_y)?;

    // Bars.
    for (i, bucket) in stats.buckets.iter().enumerate() {
        let bx = offset_x + i as f64 * (bar_w + bar_gap);
        let bh = (bucket.count as f64 / max_y) * CHART_H;
        let by = PAD_TOP + CHART_H - bh;
        writeln!(
            s,
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="3" fill="#0ea5e9" fill-opacity="0.78"/>"##,
            bx, by, bar_w, bh
        )?;
        if bucket.count > 0 {
            writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="var(--text,#e6e8ee)" font-size="10" font-weight="600">{}</text>"#,
                bx + bar_w / 2.0,
                by - 4.0,
                bucket.count
            )?;
        }
        // X-axis bucket label.
        writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" fill="currentColor" font-size="9">{}d</text>"#,
            bx + bar_w / 2.0,
            H - 6.0,
            escape_xml(&bucket.label)
        )?;
    }

    // Median + p90 reference lines, drawn relative to the bucket
    // positions (mapping a day-count back to x is approximate because
    // the buckets aren't a linear axis — we mark them on the bucket
    // that contains the value).
    if let Some(median) = stats.median_days {
        if let Some(bx) = bucket_x(median as u64, offset_x, bar_w, bar_gap) {
            writeln!(
                s,
                r##"<line x1="{bx:.1}" y1="{:.1}" x2="{bx:.1}" y2="{:.1}" stroke="#a855f7" stroke-width="1.5" stroke-dasharray="4 2" stroke-opacity="0.85"/>"##,
                PAD_TOP - 4.0,
                PAD_TOP + CHART_H
            )?;
            writeln!(
                s,
                r##"<text x="{bx:.1}" y="{:.1}" text-anchor="middle" fill="#a855f7" font-size="9" font-weight="600">median {:.0}d</text>"##,
                PAD_TOP - 8.0,
                median
            )?;
        }
    }
    if let Some(p90) = stats.p90_days {
        if let Some(bx) = bucket_x(p90 as u64, offset_x, bar_w, bar_gap) {
            writeln!(
                s,
                r##"<line x1="{bx:.1}" y1="{:.1}" x2="{bx:.1}" y2="{:.1}" stroke="#f59e0b" stroke-width="1.5" stroke-dasharray="4 2" stroke-opacity="0.85"/>"##,
                PAD_TOP - 4.0,
                PAD_TOP + CHART_H
            )?;
            writeln!(
                s,
                r##"<text x="{bx:.1}" y="{:.1}" text-anchor="middle" fill="#f59e0b" font-size="9" font-weight="600">p90 {:.0}d</text>"##,
                PAD_TOP + CHART_H + 14.0,
                p90
            )?;
        }
    }

    writeln!(
        s,
        r#"<text x="{:.1}" y="14" text-anchor="end" fill="currentColor" font-size="9">n={}</text>"#,
        W - PAD_RIGHT,
        stats.sample_size
    )?;

    s.push_str("</svg>");
    Ok(s)
}

/// Map a day count back to the center-x of the bucket that contains it.
fn bucket_x(days: u64, offset_x: f64, bar_w: f64, bar_gap: f64) -> Option<f64> {
    use super::data::CYCLE_TIME_BUCKETS;
    for (i, (_, lo, hi)) in CYCLE_TIME_BUCKETS.iter().enumerate() {
        let in_bucket = match hi {
            Some(h) => days >= *lo && days <= *h,
            None => days >= *lo,
        };
        if in_bucket {
            return Some(offset_x + i as f64 * (bar_w + bar_gap) + bar_w / 2.0);
        }
    }
    None
}

fn legend_swatch(
    out: &mut String,
    x: f64,
    y: f64,
    color: &str,
    label: &str,
    dashed: bool,
) -> std::fmt::Result {
    let dash = if dashed {
        r#" stroke-dasharray="3 2""#
    } else {
        ""
    };
    writeln!(
        out,
        r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{color}" stroke-width="2"{dash}/>"#,
        x,
        y + 4.0,
        x + 14.0,
        y + 4.0
    )?;
    writeln!(
        out,
        r#"<text x="{:.1}" y="{:.1}" fill="currentColor" font-size="9">{}</text>"#,
        x + 18.0,
        y + 7.0,
        escape_xml(label)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::charts::data::*;
    // V2 types — trace:EPIC-29 | ai:claude
    use super::super::data::{CycleTimeBucket, DepEdge, DepNode};

    #[test]
    fn status_empty_state() {
        let counts = StatusCounts {
            buckets: vec![],
            total: 0,
        };
        let svg = render_status_svg(&counts).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("No requirements"));
    }

    #[test]
    fn status_populated_has_donut_segments_and_legend() {
        let counts = StatusCounts {
            buckets: vec![("Completed".into(), 3), ("InProgress".into(), 1)],
            total: 4,
        };
        let svg = render_status_svg(&counts).unwrap();
        // Two segment <path>s, one each color.
        assert!(svg.contains("#10b981")); // Completed
        assert!(svg.contains("#f59e0b")); // InProgress
                                          // Total in the center.
        assert!(svg.contains(">4<"));
        // Legend percent labels.
        assert!(svg.contains("(75%)"));
        assert!(svg.contains("(25%)"));
    }

    #[test]
    fn burndown_renders_polylines_with_two_points() {
        let pts = vec![
            BurndownPoint {
                date: "2026-03-01".into(),
                remaining: 5.0,
                ideal: 5.0,
            },
            BurndownPoint {
                date: "2026-03-05".into(),
                remaining: 2.0,
                ideal: 0.0,
            },
        ];
        let svg = render_burndown_svg(&pts).unwrap();
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("#3b82f6")); // actual
        assert!(svg.contains("#6b7280")); // ideal
        assert!(svg.contains("Actual"));
        assert!(svg.contains("Ideal"));
    }

    #[test]
    fn burndown_empty_state_when_lt_two_points() {
        let svg = render_burndown_svg(&[]).unwrap();
        assert!(svg.contains("Not enough data"));
    }

    #[test]
    fn burnup_renders_area_polygon() {
        let pts = vec![
            BurnupPoint {
                date: "2026-03-01".into(),
                completed: 0.0,
                scope: 5.0,
            },
            BurnupPoint {
                date: "2026-03-05".into(),
                completed: 3.0,
                scope: 5.0,
            },
        ];
        let svg = render_burnup_svg(&pts).unwrap();
        assert!(svg.contains("<polygon"));
        assert!(svg.contains("fill-opacity=\"0.12\""));
    }

    #[test]
    fn velocity_renders_bars() {
        let pts = vec![
            VelocityPoint {
                label: "S1".into(),
                points: 8,
            },
            VelocityPoint {
                label: "S2".into(),
                points: 12,
            },
            VelocityPoint {
                label: "S3".into(),
                points: 5,
            },
        ];
        let svg = render_velocity_svg(&pts).unwrap();
        assert!(svg.contains("<rect")); // bars
        assert!(svg.contains("#8b5cf6")); // bar color
        assert!(svg.contains(">8<")); // value label on S1
        assert!(svg.contains(">S2<")); // x-axis label
        assert!(svg.contains("avg")); // avg line label
    }

    #[test]
    fn velocity_empty_state() {
        let svg = render_velocity_svg(&[]).unwrap();
        assert!(svg.contains("No sprints"));
    }

    #[test]
    fn feature_progress_renders_horizontal_bars() {
        let rows = vec![
            FeatureProgressRow {
                feature: "auth".into(),
                completed: 4,
                total: 5,
            },
            FeatureProgressRow {
                feature: "billing".into(),
                completed: 1,
                total: 3,
            },
        ];
        let svg = render_feature_progress_svg(&rows).unwrap();
        assert!(svg.contains(">auth<"));
        assert!(svg.contains(">billing<"));
        assert!(svg.contains("4 / 5"));
        assert!(svg.contains("1 / 3"));
        assert!(svg.contains("--accent"));
    }

    #[test]
    fn feature_progress_truncates_long_names() {
        let rows = vec![FeatureProgressRow {
            feature: "an-extremely-long-feature-name-that-overflows".into(),
            completed: 1,
            total: 1,
        }];
        let svg = render_feature_progress_svg(&rows).unwrap();
        // Truncated to 16 chars + ellipsis.
        assert!(svg.contains("an-extremely-lon…"));
    }

    #[test]
    fn xml_escapes_user_content() {
        let counts = StatusCounts {
            buckets: vec![("Has<bracket>".into(), 1)],
            total: 1,
        };
        let svg = render_status_svg(&counts).unwrap();
        assert!(!svg.contains("<bracket>"));
        assert!(svg.contains("&lt;bracket&gt;"));
    }

    // =====================================================================
    // V2 renderer tests — trace:EPIC-29 | ai:claude
    // =====================================================================

    fn cfd_point(date: &str, statuses: &[(&str, u32)]) -> CfdPoint {
        let mut by_status = std::collections::BTreeMap::new();
        for (s, n) in statuses {
            by_status.insert((*s).into(), *n);
        }
        CfdPoint {
            date: date.into(),
            by_status,
        }
    }

    #[test]
    fn cfd_empty_state_when_window_too_short() {
        let svg = render_cfd_svg(&[]).unwrap();
        assert!(svg.contains("Not enough data for CFD"));
    }

    #[test]
    fn cfd_renders_one_polygon_per_active_status() {
        let pts = vec![
            cfd_point("2026-05-19", &[("Draft", 3)]),
            cfd_point("2026-05-20", &[("Draft", 2), ("InProgress", 1)]),
            cfd_point("2026-05-21", &[("Draft", 1), ("InProgress", 2)]),
            cfd_point("2026-05-22", &[("Completed", 3)]),
        ];
        let svg = render_cfd_svg(&pts).unwrap();
        // One polygon per active status (Draft, InProgress, Completed) = 3.
        let polygons = svg.matches("<polygon").count();
        assert_eq!(polygons, 3, "expected 3 polygons in {svg}");
        // Status colors all present.
        assert!(svg.contains("#6b7280")); // Draft
        assert!(svg.contains("#f59e0b")); // InProgress
        assert!(svg.contains("#10b981")); // Completed
                                          // Status names in legend.
        assert!(svg.contains(">Draft<"));
        assert!(svg.contains(">InProgress<"));
        assert!(svg.contains(">Completed<"));
    }

    #[test]
    fn dep_graph_empty_state_when_no_nodes() {
        let g = DepGraph {
            nodes: vec![],
            edges: vec![],
            truncated: false,
        };
        let svg = render_dep_graph_svg(&g).unwrap();
        assert!(svg.contains("Dependency graph empty"));
    }

    #[test]
    fn dep_graph_renders_nodes_with_status_colors() {
        let g = DepGraph {
            nodes: vec![
                DepNode {
                    spec_id: "EPIC-1".into(),
                    status: "Approved".into(),
                    depth: 0,
                },
                DepNode {
                    spec_id: "STORY-1".into(),
                    status: "InProgress".into(),
                    depth: 1,
                },
            ],
            edges: vec![DepEdge {
                from_spec: "EPIC-1".into(),
                to_spec: "STORY-1".into(),
                kind: "Custom:parent_of".into(),
            }],
            truncated: false,
        };
        let svg = render_dep_graph_svg(&g).unwrap();
        assert!(svg.contains(">EPIC-1<"));
        assert!(svg.contains(">STORY-1<"));
        // Approved + InProgress colors present as strokes.
        assert!(svg.contains("#3b82f6"));
        assert!(svg.contains("#f59e0b"));
        // Edge path present.
        assert!(svg.contains("<path d=\"M "));
        // Arrowhead polygon.
        assert!(svg.contains("<polygon"));
    }

    #[test]
    fn dep_graph_marks_truncation() {
        let g = DepGraph {
            nodes: vec![DepNode {
                spec_id: "X".into(),
                status: "Draft".into(),
                depth: 0,
            }],
            edges: vec![],
            truncated: true,
        };
        let svg = render_dep_graph_svg(&g).unwrap();
        assert!(svg.contains("truncated at depth"));
    }

    #[test]
    fn cycle_time_empty_state_when_no_samples() {
        let stats = CycleTimeStats {
            buckets: vec![],
            sample_size: 0,
            median_days: None,
            p90_days: None,
        };
        let svg = render_cycle_time_svg(&stats).unwrap();
        assert!(svg.contains("No cycle-time samples"));
    }

    #[test]
    fn cycle_time_renders_bars_and_reference_lines() {
        let stats = CycleTimeStats {
            buckets: vec![
                CycleTimeBucket {
                    label: "0–7".into(),
                    count: 1,
                },
                CycleTimeBucket {
                    label: "8–14".into(),
                    count: 1,
                },
                CycleTimeBucket {
                    label: "15–30".into(),
                    count: 0,
                },
                CycleTimeBucket {
                    label: "31–60".into(),
                    count: 1,
                },
                CycleTimeBucket {
                    label: "60+".into(),
                    count: 0,
                },
            ],
            sample_size: 3,
            median_days: Some(13.0),
            p90_days: Some(41.0),
        };
        let svg = render_cycle_time_svg(&stats).unwrap();
        // Bars colored sky-blue.
        assert!(svg.contains("#0ea5e9"));
        // Median + p90 reference lines + labels.
        assert!(svg.contains("median 13d"));
        assert!(svg.contains("p90 41d"));
        // Sample size annotation.
        assert!(svg.contains("n=3"));
        // X-axis bucket labels with `d` suffix.
        assert!(svg.contains(">0–7d<"));
        assert!(svg.contains(">60+d<"));
    }
}
