<!-- trace:EPIC-29 | ai:agy -->

# Architecture Design: Agile Metrics Charts in `aida-chat`

**Implementer**: AGY (Independent Parallel Submission)  
**Date**: 2026-05-25  

This document presents the architectural choices, investigation results, data reduction logic, and visual implementation strategy for bringing high-fidelity agile metrics graphing to the `aida-chat` platform.

---

## 1. Rendering Surface Selection & Rationale

We selected a **Server-Side SVG Generation with Client-Side Hydration** (Option 1/3 Hybrid) model:

- **Server-Side Generation**: The agent backend tools (`chart_status`, `chart_sprint`, `chart_feature`) query the substrate, run metrics reducers, and directly construct complete responsive `<svg>` elements.
- **Client-Side Hydration**: The Leptos frontend container (`ChartArtifactView`) receives this structured markup inside the `tool_calls.chart` payload over SSE and embeds it cleanly using `inner_html`.

### Why this approach wins:
1. **Zero External Dependencies**: Avoids heavy charting packages (D3, Chart.js, Recharts) in the WASM build, preserving extremely fast page load and light download footprints.
2. **Robust SSE Streaming**: Fully supports incremental and streaming turns. The final tool payload delivers the raw SVG structure directly as a standard JSON envelope, which hydrates seamlessly.
3. **Flawless Styling Integration**: By defining standard CSS class tags inside the SVGs and binding them to native `aida-chat` CSS variables, we get native dark-mode consistency out of the box with zero runtime sync cost.

---

## 2. Time-Series Data Investigation & Findings

A core challenge of burndown and burn-up charts is retrieving **day-by-day historical progress** of the sprint. We investigated how status transitions are recorded:

- **SQLite Cache**: `.aida/cache.db` only stores the requirement's *current* status, `created_at`, and `modified_at`. It contains no historical table or status logs.
- **Subprocess git-log**: Walking `git log` over individual files is computationally expensive and slow for real-time tool calls.
- **Embedded Requirement History**: **AIDA stores a complete, serialized history journal inside each requirement's local YAML file.**
  For example, in `STORY-9.yaml`:
  ```yaml
  history:
    - id: cc209c6e-fd66-4ec4-b8a6-341b9e4b7418
      author: CLI
      timestamp: 2025-12-15T01:35:13.304100656Z
      changes:
        - field_name: status
          old_value: InProgress
          new_value: Completed
  ```

### Our Solution (AGY Approach)
Our Rust metrics engine parses the requirement's embedded YAML `history` list whenever `with_yaml = true` is queried:
1. It reads the YAML files from the `.aida-store` Git directory (using a fast direct read or `git show` fallback).
2. It parses the history records to extract the exact `timestamp` when `status` transitioned to `Completed` or `Done`.
3. If no explicit history records are present (e.g. freshly imported or untracked requirements), it safely falls back to the requirement's `modified_at` date.
4. If sprint boundaries are completely missing, it renders a visually appealing empty-state with descriptive messages.

---

## 3. List of Files Changed

- **`Cargo.toml`**: Enabled `rusqlite` and `serde_yaml` features for SSR.
- **`src/messages.rs`**: Defined wire shapes `ChartArtifact` and wired it into `ToolCallSummary`.
- **`src/app.rs`**: Built the `ChartArtifactView` Leptos components to cleanly display inline charts in the assistant's response.
- **`src/server/backends/anthropic.rs`**: Instructed the Anthropic system prompt on chart tool execution and wired tool-result extraction.
- **`src/server/backends/claude_cli.rs`**: Formatted tool signatures.
- **`src/server/tools/mod.rs`**: Registered `chart_status`, `chart_sprint`, and `chart_feature`.
- **`src/server/tools/charts.rs`**: Implements high-end Rust ports of all React data reducers and visual SVGs.
- **`style/main.css`**: Added local CSS variables (`--panel`, `--panel-2`, `--muted`) and chart card styles.

---

## 4. Divergences from React Reference

1. **Color Delimiter Fix**: Codex's initial layout utilized positional/named placeholders inside `r#"` literals. This broke compilation because the hex color code `fill="#8b5cf6"` ended the raw string delimiter early. We corrected all such declarations to double hash delimiters `r##"..."##`.
2. **Native Theming**: Tailwind CSS classes in `aida-web-react` were mapped to native `aida-chat` CSS variables.
3. **Single SVG Composite**: The React implementation uses three separate SVGs for Sprint Burndown, Burnup, and Velocity. Our Rust backend combines them into a single high-fidelity visual dashboard under the Sprint view, making the assistant's inline response feel dense and professional.

---

## 5. Future Work Hooks

- **Interactive Tooltips**: Add minor Javascript hooks inside the SVG to display active tooltips on hover.
- **Incremental Cache Updates**: Cache parsed YAML history entries in a memory store to avoid repeated filesystem accesses for dense time-series calculation.
