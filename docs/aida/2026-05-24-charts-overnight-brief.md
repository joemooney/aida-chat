# Overnight Deep-Research Brief — Agile Charts for aida-chat (EPIC-29 slice E + B seed)

**Audience:** Codex, Claude, AGY (independent, parallel submissions — three implementers, three branches, three deliverables; operator + advisor compare and decide which wins or composes the winning hybrid).

**Timeline:** Overnight (2026-05-24 PM → 2026-05-25 AM). One full development cycle. Report deliverables by morning standup.

**Why this brief is shorter than it seems:** aida-web-react already has the entire chart family (burn-down, burn-up, velocity, status, feature progress, sprint summary). This is a **port + improvement**, not invention from scratch. The visual IP, algorithmic IP, and substrate-shape conventions all exist. Your job is to bring them to aida-chat (Rust + Leptos) at production quality.

---

## Goal

Ship a high-end, professional, on-the-shelf agile-metrics graphing capability into aida-chat so it can answer agile queries with **visual artifacts**, not just text. When operator types "show me the sprint burndown" or "what's the velocity trend?" or "what's still blocked?", aida-chat renders the chart inline in the conversation.

**Quality bar:** the rendered charts should be good enough to demo to a stranger as a polished product surface. They need to **look** professional — typography, spacing, color, axis treatment, empty-states, no-data fallbacks, dark-theme consistency. The differentiation thesis depends on this: aida-chat as the first non-dogfood AIDA consumer must demonstrate *visible* substrate value, and charts are the most visceral demonstration available.

**Strategic stakes:** operator wants AIDA core itself to be the test consumer (aida-chat pointed at `~/ai/aida`'s substrate would demo on real, dense, sprint-organized data). The charts must work against any aida-project's substrate, not just aida-chat's own.

---

## Discovery prerequisites (read FIRST, ~30 min)

Before designing anything, read these files in `~/ai/aida/aida-web-react/`:

### Charting components (visual IP)

- `src/components/sprint/charts/BurndownChart.tsx`
- `src/components/sprint/charts/BurnupChart.tsx`
- `src/components/sprint/charts/VelocityChart.tsx`
- `src/components/sprint/charts/SprintCharts.tsx` (composer)
- `src/components/dashboard/StatusChart.tsx`
- `src/components/dashboard/FeatureProgress.tsx`
- `src/components/dashboard/MetricsCards.tsx`
- `src/components/dashboard/SprintSummary.tsx`

Observe: hand-crafted SVG via `viewBox`, no external chart dep, Tailwind utility classes for theming, `currentColor` for stroke/fill so the dark-mode palette inherits cleanly.

### Data-shaping (algorithmic IP)

- `src/lib/sprint-utils.ts` — `computeBurndownData`, `computeBurnupData`, `computeVelocityData`, `getSprintDates`, `getSprintState`, `computeSprintProgress`, `getSprintAssignmentTarget`, `isSprintAssignment`. **Port the algorithms — don't reinvent them.**

### Substrate shape

- AIDA stores sprints as **requirements with type `Sprint`** (or with `sprint_number` custom field — verify in `~/ai/aida/.aida-store/objects/`).
- Sprint membership = `Custom("sprint_assignment")` relationship from item → sprint.
- Story points = `requirement.weight` (integer).
- Time bounds = `custom_fields.start_date` / `end_date` (ISO date strings).
- Status state machine = `Draft | Approved | InProgress | Completed | Rejected | Planned` (verify against the canonical enum).

### aida-chat target

- `~/ai/aida-chat/src/server/backends/anthropic.rs` — agent loop and tool dispatch.
- `~/ai/aida-chat/src/server/tools/` — current tool surface (read tools confined to repo_root; aida_* tools via MCP + CLI fallback).
- `~/ai/aida-chat/src/app.rs` — Leptos UI; you'll be adding chart-rendering components here or in a sibling module.
- `~/ai/aida-chat/src/messages.rs` — wire-shape definitions.
- `~/ai/aida-chat/style/main.css` — existing dark theme, CSS vars (`--bg`, `--border`, `--accent`, etc.) you should reuse.

---

## Investigation areas (decide and document your answers)

### 1. Time-series data availability

Burn-down/burn-up are inherently time-series: you need to know "how many points were remaining on each day of the sprint?" AIDA may not store status-change history directly. Investigate:

- Does `aida history --events --json` produce timestamped events? (run it; see the shape)
- Does the aida-store orphan branch's `git log` over `.aida-store/objects/*.yaml` give you a usable change feed?
- Are `created_at` / `updated_at` fields present on every requirement?
- Is there a status-change journal anywhere?

If the data is genuinely absent for true historical burn-down, propose:
- A pragmatic fallback (linear interpolation from creation date to "now" for in-flight items)
- AIDA core extension (file as TASK in aida repo: "expose status-change history via MCP / CLI")
- Both, with the fallback shipped today and the AIDA-core extension as the path to "true" data

### 2. Rendering surface

Three viable approaches:
1. **Server-side SVG** — Rust serializes complete `<svg>` markup, streamed via SSE/HTTP; client just embeds. Lowest moving parts, charts render even with JS disabled.
2. **Client-side Leptos** — Leptos components produce SVG DOM nodes in the hydrate build. Same theming primitives as the chat UI; reactive to data changes.
3. **Hybrid** — server-rendered initial SVG, client takes over for interactions (tooltips, drill-down).

Pick one and justify. The aida-chat existing code is hybrid (SSR + hydrate) so option 3 is the natural fit, but option 1 is the smallest viable slice. Don't over-engineer.

### 3. Chart inclusion in chat responses

Two surfaces:
1. **Inline chart in the chat message** — when the model invokes a chart tool, the tool returns SVG (or a chart spec) that renders as part of the assistant's response.
2. **Standalone chart panel** — slash-command or button surfaces a dashboard view alongside the chat.

The brief weights toward (1) — the differentiation is *conversation that produces charts*, not a separate dashboard. But your call.

### 4. Theming

aida-chat is dark-themed; aida-web-react is also dark-themed but uses Tailwind tokens (`text-content-secondary`, `fill-content-muted`). Map the aida-web-react palette to aida-chat's CSS vars. Charts must look native to aida-chat — no Tailwind classes leaking in.

### 5. Empty-state and no-data fallback

What does "show me the sprint burndown" render when:
- There's no active sprint? → empty state with link to "create a sprint"
- The sprint has no items? → empty state explaining
- Time-series data is unavailable? → fallback chart + caveat note

aida-web-react's `BurndownChart` already does this; mirror the pattern.

---

## Implementation scope (the deliverable)

### Required charts (V1)

In priority order — ship them in this order, stop wherever the night ends:

1. **Status distribution** (donut or horizontal bar). Source: live `aida list --json` or MCP `list_requirements({})`. Always renderable; no time-series needed. This is the easiest chart and proves the rendering pipeline works.
2. **Sprint burn-down** — port of `BurndownChart.tsx`. Requires active sprint detection + per-day remaining-work computation.
3. **Sprint burn-up** — port of `BurnupChart.tsx`. Same data shape as burn-down + scope-creep line.
4. **Velocity** — port of `VelocityChart.tsx`. Per-sprint completed-points bars across the last N sprints.
5. **Feature progress** — port of `FeatureProgress.tsx`. Per-epic completion bars.

If you have time after V1:
6. Cumulative flow diagram (status counts over time)
7. Dependency graph (mermaid output of `aida rel list` plus traversal)
8. Cycle time histogram (days from Approved → Completed per shipped story)

### Tool integration

Add chart tools to aida-chat's agent surface. Suggested naming:
- `chart_status` — renders status distribution
- `chart_sprint` — renders burn-down + burn-up + velocity for a sprint (parameter: sprint_id; default: active sprint)
- `chart_feature` — renders feature progress for an epic (parameter: epic_id)

Each tool returns either inline SVG (option 1 above) or a structured chart spec the UI renders. Either is fine; be consistent.

Update the anthropic backend's system prompt to advertise the new chart tools and when to use them.

### Acceptance criteria (FULL — no subset-shipping)

- All V1 charts render against live `aida-chat`'s own substrate (drop into `~/ai/aida-chat` and `cargo leptos serve`).
- All V1 charts ALSO render against `aida` core's substrate (point `AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida`; should work without code changes since the data-shaping logic is substrate-shape-aware).
- Dark-theme consistency: charts inherit from aida-chat's CSS vars, no foreign tokens.
- Empty states render gracefully on a no-sprint / no-data project.
- `cargo test --features ssr --lib` stays green plus new tests for data-shaping helpers.
- ssr + hydrate + cargo leptos builds all clean.
- A `cargo run --example charts_smoke` (or equivalent) demonstrates a chart rendering end-to-end.

### Don't

- Don't pull in a chart library dep (recharts, plotly, d3, chart.js, vega, nivo, visx, etc.). Hand-crafted SVG matches the aida-web-react ethos and keeps aida-chat lightweight.
- Don't reinvent the data-shaping logic. Port `sprint-utils.ts` algorithms directly (translation TypeScript → Rust); cite the source file in trace comments.
- Don't build a separate dashboard page (inline-in-chat is V1; dashboards are a later story).
- Don't bundle the BUG-377 cleanup or other unrelated changes — chart scope is the entire scope.
- Don't ship without empty-state handling (looks unprofessional with broken charts on edge cases).

---

## Multi-implementer evaluation

You are **one of three** implementers working on this brief in parallel (Codex + Claude + AGY). All three submissions will be compared on:

1. **Visual quality** — does the rendered chart look like a polished product surface?
2. **Code quality** — is the Rust idiomatic, well-tested, well-factored?
3. **Faithfulness to aida-web-react** — did you port the existing IP correctly, or invent inferior alternatives?
4. **Investigation depth** — does your architecture doc surface the time-series data question with a real recommendation?
5. **Brief-discipline** — did you ship FULL acceptance (every V1 chart, both substrate targets, empty states)?
6. **Trace discipline** — `// trace:EPIC-29 | ai:<your-tool>` on every new file.

The winning submission may be adopted whole, or the operator may compose a hybrid (e.g. your data layer + another's rendering). Submit confidently — the operator and advisor will arbitrate.

---

## Deliverables

By morning standup, each implementer reports back with:

1. **Branch**: `<your-tool>/epic-29-charts-overnight` (e.g. `codex/epic-29-charts-overnight`, `claude/epic-29-charts-overnight`, `agy/epic-29-charts-overnight`)
2. **PR (draft is fine)** against master with the implementation.
3. **Architecture document** at `docs/aida/2026-05-25-<your-tool>-charts-arch.md` covering:
   - Rendering-surface choice + rationale
   - Time-series data investigation findings + chosen approach
   - List of files changed and why
   - Open questions / future-work hooks
   - Any divergences from the aida-web-react reference + why
4. **Screenshots** (or terminal-capture for any CLI surface) of each V1 chart rendered against aida-chat's own substrate AND aida core's substrate. Save under `docs/aida/2026-05-25-<your-tool>-charts-screenshots/`.
5. **`cargo run --example charts_smoke`** that demonstrates the rendering pipeline working end-to-end.

---

## Commit & PR format

Trailer: `(EPIC-29)`.

Commit message: `[AI:<your-tool>] feat(charts): agile metrics graphing — burndown/burnup/velocity/status (EPIC-29)`.

PR title: `EPIC-29 V1: agile metrics charts (<your-tool> implementation)`.

PR description includes:
- Link to your architecture doc
- Link to your screenshots directory
- Self-evaluation against the 6 criteria above (1–5 stars each, brief justification)
- One-line on what you'd do next if you had another night

---

— aida-chat advisor, 2026-05-24
