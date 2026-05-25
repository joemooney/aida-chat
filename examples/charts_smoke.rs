//! End-to-end smoke for the EPIC-29 chart pipeline.
//!
//! Reads the AIDA substrate at the configured repo root, computes all
//! five V1 charts, and writes each one as an `.svg` file under a
//! target directory.
//!
//! ```
//! # Default: ./.aida-store next to the binary's CWD
//! cargo run --example charts_smoke --features ssr
//!
//! # Against aida core
//! AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida \
//!   AIDA_CHAT_CHARTS_OUT=/tmp/charts-aida \
//!   cargo run --example charts_smoke --features ssr
//!
//! # Against aida-chat itself (when run from the project root)
//! AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida-chat \
//!   AIDA_CHAT_CHARTS_OUT=/tmp/charts-aida-chat \
//!   cargo run --example charts_smoke --features ssr
//! ```
//!
//! Exits non-zero on the first failure. Not part of `cargo test` since
//! it needs a real `.aida-store` directory.

use std::path::PathBuf;
use std::process::ExitCode;

use aida_chat::server::charts::{
    data::{
        compute_burndown, compute_burnup, compute_cfd, compute_cycle_time, compute_dep_graph,
        compute_feature_progress, compute_status_counts, compute_velocity,
    },
    render_burndown_svg, render_burnup_svg, render_cfd_svg, render_cycle_time_svg,
    render_dep_graph_svg, render_feature_progress_svg, render_status_svg, render_velocity_svg,
    AidaStore,
};

fn main() -> ExitCode {
    let repo_root = std::env::var("AIDA_CHAT_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("cwd"));
    let out_dir = std::env::var("AIDA_CHAT_CHARTS_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/aida-chat-charts-smoke"));

    println!("→ repo_root: {}", repo_root.display());
    println!("→ out_dir:   {}", out_dir.display());
    if !AidaStore::has_store(&repo_root) {
        eprintln!(
            "FAIL: no `.aida-store/` directory under {}. \
             aida-chat charts read AIDA's distributed store directly — \
             point AIDA_CHAT_REPO_ROOT at an AIDA-initialized project.",
            repo_root.display()
        );
        return ExitCode::FAILURE;
    }
    let store = match AidaStore::load(&repo_root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FAIL: load store: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("→ loaded {} requirements", store.items.len());

    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("FAIL: mkdir {}: {e}", out_dir.display());
        return ExitCode::FAILURE;
    }

    let items: Vec<_> = store.items.iter().collect();

    // 1. Status distribution (always renderable)
    let status_counts = compute_status_counts(&items);
    write_svg(&out_dir, "status", render_status_svg(&status_counts));
    println!(
        "✓ status: {} buckets, total {}",
        status_counts.buckets.len(),
        status_counts.total
    );

    // 2 + 3. Burn-down + burn-up — pick a sprint:
    //   * prefer the active sprint
    //   * else the most-recently-numbered sprint with dates
    //   * else first sprint with dates
    //   * else the first sprint at all (which falls into empty-state)
    let sprints = store.sprints();
    println!("→ {} sprint(s) in this substrate", sprints.len());
    let today = chrono_today();
    let chosen = sprints
        .iter()
        .find(|s| s.state(&today) == aida_chat::server::charts::data::SprintState::Active)
        .or_else(|| {
            sprints
                .iter()
                .filter(|s| s.start_date.is_some() && s.end_date.is_some())
                .max_by_key(|s| s.sprint_number)
        })
        .or_else(|| sprints.first());

    if let Some(sprint) = chosen {
        println!(
            "→ sprint: {} ({} members, dates {:?}..{:?})",
            sprint.req.spec_id,
            sprint.member_ids.len(),
            sprint.start_date,
            sprint.end_date
        );
        let sprint_items = store.sprint_items(sprint);
        let (start, end) = (
            sprint.start_date.as_deref().unwrap_or(""),
            sprint.end_date.as_deref().unwrap_or(""),
        );
        let bd = compute_burndown(&sprint_items, start, end);
        write_svg(&out_dir, "burndown", render_burndown_svg(&bd));
        let bu = compute_burnup(&sprint_items, start, end);
        write_svg(&out_dir, "burnup", render_burnup_svg(&bu));
        println!("✓ burn-down: {} points", bd.len());
        println!("✓ burn-up: {} points", bu.len());
    } else {
        // Empty-state — render the fallback SVG so the smoke still
        // proves the pipeline + screenshots show what an empty looks like.
        write_svg(&out_dir, "burndown", render_burndown_svg(&[]));
        write_svg(&out_dir, "burnup", render_burnup_svg(&[]));
        println!("✓ burn-down + burn-up: empty-state (no sprint with dates)");
    }

    // 4. Velocity across all sprints
    let velocity = compute_velocity(&sprints, |s| store.sprint_items(s));
    write_svg(&out_dir, "velocity", render_velocity_svg(&velocity));
    println!("✓ velocity: {} sprint(s)", velocity.len());

    // 5. Feature progress
    let feature_rows = compute_feature_progress(&items);
    write_svg(
        &out_dir,
        "feature_progress",
        render_feature_progress_svg(&feature_rows),
    );
    println!("✓ feature progress: {} feature group(s)", feature_rows.len());

    // -------- V2 charts (trace:EPIC-29 | ai:claude) --------

    // 6. Cumulative flow (CFD), default 30-day window.
    let cfd_points = compute_cfd(&items, &today, 30);
    write_svg(&out_dir, "cfd", render_cfd_svg(&cfd_points));
    let active_statuses: std::collections::BTreeSet<&str> = cfd_points
        .iter()
        .flat_map(|p| p.by_status.keys().map(|k| k.as_str()))
        .collect();
    println!(
        "✓ CFD: {} day(s) × {} active status bucket(s)",
        cfd_points.len(),
        active_statuses.len()
    );

    // 7. Dependency graph. Pick a sensible root: env override, then
    //    the first Epic encountered, else the first requirement.
    let root_spec_env = std::env::var("AIDA_CHAT_CHARTS_DEP_ROOT").ok();
    let dep_root = root_spec_env.as_deref().or_else(|| {
        store
            .items
            .iter()
            .find(|r| r.req_type.eq_ignore_ascii_case("Epic"))
            .map(|r| r.spec_id.as_str())
            .or_else(|| store.items.first().map(|r| r.spec_id.as_str()))
    });
    if let Some(root) = dep_root {
        let graph = compute_dep_graph(
            root,
            2,
            |s| store.by_spec(s),
            |u| store.by_uuid.get(u).map(|&i| &store.items[i]),
        );
        write_svg(&out_dir, "dep_graph", render_dep_graph_svg(&graph));
        println!(
            "✓ dep graph rooted at {root}: {} node(s), {} edge(s){}",
            graph.nodes.len(),
            graph.edges.len(),
            if graph.truncated { " (truncated)" } else { "" }
        );
    } else {
        // Truly empty store — render the empty-state SVG so the file
        // exists for screenshot consistency.
        let empty = aida_chat::server::charts::data::DepGraph {
            nodes: vec![],
            edges: vec![],
            truncated: false,
        };
        write_svg(&out_dir, "dep_graph", render_dep_graph_svg(&empty));
        println!("✓ dep graph: empty-state (no requirements)");
    }

    // 8. Cycle-time histogram, default 90-day window.
    let cycle_stats = compute_cycle_time(&items, &today, 90);
    write_svg(&out_dir, "cycle_time", render_cycle_time_svg(&cycle_stats));
    println!(
        "✓ cycle time: n={}{}{}",
        cycle_stats.sample_size,
        cycle_stats
            .median_days
            .map(|m| format!(", median={m:.0}d"))
            .unwrap_or_default(),
        cycle_stats
            .p90_days
            .map(|p| format!(", p90={p:.0}d"))
            .unwrap_or_default(),
    );

    println!("→ wrote 8 svg files to {}", out_dir.display());
    ExitCode::SUCCESS
}

fn write_svg(dir: &std::path::Path, name: &str, svg: Result<String, impl std::fmt::Display>) {
    match svg {
        Ok(svg) => {
            let path = dir.join(format!("{name}.svg"));
            if let Err(e) = std::fs::write(&path, &svg) {
                eprintln!("WARN: write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("WARN: render {name}: {e}"),
    }
}

fn chrono_today() -> String {
    use chrono::Utc;
    Utc::now().format("%Y-%m-%d").to_string()
}
