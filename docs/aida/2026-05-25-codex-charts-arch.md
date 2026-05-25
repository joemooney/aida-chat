# Codex Charts Architecture — EPIC-29

## Rendering Surface

This implementation uses a hybrid surface:

- Chart tools run on the server and return a structured `ChartArtifact` payload containing complete SVG.
- The Anthropic backend recognizes that payload and attaches it to the tool-call summary.
- The Leptos chat UI renders the artifact inline in the assistant turn, next to the tool badge and before the final markdown response.

This avoids relying on the model to paste raw SVG into its answer. It also preserves the existing markdown sanitizer, while still rendering charts inline as first-class response artifacts.

## Time-Series Investigation

Findings:

- `aida history --events` exposes timestamped event text, but not machine-readable JSON in the installed CLI shape; `aida history --events --json` is not accepted.
- `.aida/cache.db` has broad requirement index data: IDs, SPEC-IDs, status, type, feature, timestamps, and YAML path.
- Sprint metadata and membership are in `.aida-store` YAML. AIDA core currently uses `!Custom sprint_contains` relationships from sprint to item; the brief also mentioned item-to-sprint `sprint_assignment`, so the loader supports both relationship directions.
- Requirement YAML in the tested substrates did not expose a general status-change journal suitable for true historical burn-down.

Chosen approach:

- Ship the pragmatic fallback now: burn-down and burn-up use current status plus `modified_at` for completed items, matching the fallback already present in `aida-web-react/src/lib/sprint-utils.ts`.
- Document the caveat in the sprint chart subtitle.
- Future AIDA core work should expose status-change history through CLI/MCP in structured JSON so charts can become true historical series.

## Files Changed

- `src/server/tools/charts.rs`: chart tools, AIDA cache/YAML loader, sprint-utils ports, and hand-authored SVG renderers.
- `src/server/tools/mod.rs`: registers `chart_status`, `chart_sprint`, and `chart_feature`.
- `src/server/backends/anthropic.rs`: captures chart artifacts from tool output and advertises chart tools in the system prompt.
- `src/messages.rs`: adds `ChartArtifact` and optional chart data on tool summaries.
- `src/app.rs`: renders chart artifacts inline in live and persisted assistant turns.
- `style/main.css`: native dark-theme styling for chart artifact frames.
- `examples/charts_smoke.rs`: end-to-end chart generation smoke for both substrates.
- `Cargo.toml`: adds `rusqlite` and `serde_yaml` for cache/YAML reading.
- `src/lib.rs`: raises recursion limit for the larger Leptos hydrate view tree.

## Divergences From aida-web-react

- React renders SVG components client-side. aida-chat renders server-generated SVG artifacts because tool calls already execute server-side and the chat transcript needs durable artifacts.
- Status distribution is rendered as a polished horizontal-bar/card hybrid instead of the React donut. This keeps text legible inside a narrow chat column while preserving the same status color mapping.
- Feature progress groups by `feature` unless an `epic_id` filter is supplied. The filter is conservative because current cache rows do not include full relationship graph data.
- Burn-down/burn-up operate on item counts rather than story points when weights are absent. If YAML exposes `weight`, the loader uses it for velocity.

## Verification

- `cargo test --features ssr --lib`
- `cargo build --features ssr --lib`
- `cargo build --features hydrate --lib`
- `cargo leptos build`
- `cargo run --example charts_smoke --features ssr`
- `AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida AIDA_CHART_SMOKE_OUT=/home/joe/ai/aida-chat/target/charts_smoke_aida cargo run --example charts_smoke --features ssr`

Generated captures:

- `docs/aida/2026-05-25-codex-charts-screenshots/aida-chat/`
- `docs/aida/2026-05-25-codex-charts-screenshots/aida-core/`

## Open Questions

- AIDA core should expose structured status-change history through MCP/CLI for true burn-down, cumulative flow, and cycle-time charts.
- `chart_feature(epic_id)` should eventually use first-class relationship traversal instead of cache/tag heuristics.
- Tool-call inspector work can make chart artifacts expandable alongside full request/response details once STORY-14 lands.
