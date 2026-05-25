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

use std::fmt::Write;

use thiserror::Error;

use super::data::{
    status_color, BurndownPoint, BurnupPoint, FeatureProgressRow, StatusCounts, VelocityPoint,
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
    write_polyline(&mut s, points, &x_at, &y_at, |p| p.ideal, "#6b7280", 1.5, true)?;
    // Actual line (solid blue).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.remaining,
        "#3b82f6",
        2.0,
        false,
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
    writeln!(s, r##"<polygon points="{area}" fill="#10b981" fill-opacity="0.12"/>"##)?;

    // Scope line (amber dashed).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.scope,
        "#f59e0b",
        1.5,
        true,
    )?;
    // Completed line (solid emerald).
    write_polyline(
        &mut s,
        points,
        &x_at,
        &y_at,
        |p| p.completed,
        "#10b981",
        2.0,
        false,
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
        return Ok(empty(
            "No requirements grouped by feature.",
        ));
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

fn write_polyline<T>(
    out: &mut String,
    points: &[T],
    x_at: &dyn Fn(usize) -> f64,
    y_at: &dyn Fn(f64) -> f64,
    project: impl Fn(&T) -> f64,
    color: &str,
    width: f64,
    dashed: bool,
) -> std::fmt::Result {
    let mut pts = String::with_capacity(points.len() * 12);
    for (i, p) in points.iter().enumerate() {
        if !pts.is_empty() {
            pts.push(' ');
        }
        let _ = write!(pts, "{:.1},{:.1}", x_at(i), y_at(project(p)));
    }
    let dash_attr = if dashed { r#" stroke-dasharray="6 3""# } else { "" };
    writeln!(
        out,
        r#"<polyline points="{pts}" fill="none" stroke="{color}" stroke-width="{width}"{dash_attr}/>"#
    )?;
    Ok(())
}

fn legend_swatch(
    out: &mut String,
    x: f64,
    y: f64,
    color: &str,
    label: &str,
    dashed: bool,
) -> std::fmt::Result {
    let dash = if dashed { r#" stroke-dasharray="3 2""# } else { "" };
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
}
