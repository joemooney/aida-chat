# EPIC-29 V1 — Architecture doc (Claude implementation)

**Date:** 2026-05-25
**Spec:** EPIC-29 — Agile query library: canned reports + AI-generated reusable artifacts
**Branch:** `claude/epic-29-charts-overnight`
**Author:** Claude (Anthropic), one of three parallel implementers

This document accompanies the EPIC-29 V1 frontend PR. It records the
architectural decisions that fell out of the overnight discovery cycle,
the time-series-data investigation findings, and the open hooks for
follow-on work.

---

## 1. Rendering-surface choice

**Decision: option 1 from the brief — server-side SVG, complete `<svg>` markup
returned by chart tools and pushed to the chat UI out-of-band over SSE.**

### Why not the other options

- **Option 2 (client-side Leptos SVG components)** would have moved the
  geometry math into the wasm bundle and re-implemented the React port
  in Leptos. That's about 5x the LOC of option 1 (every chart needs its
  own component + props + reactive plumbing) and loses the "renders
  even with JS disabled" property. The chat already requires JS for
  SSE, so the "no JS" property isn't strictly load-bearing, but the
  smaller surface area is.
- **Option 3 (hybrid server-render + client take-over)** would have
  been right if charts needed interactivity (tooltips, brush-select,
  drill-down). They don't, in V1. The brief explicitly weights toward
  inline-in-chat over a separate dashboard, which makes per-chart
  interactivity less valuable than artifact-as-message-content.

### How option 1 actually plays out

A chart tool (`chart_status`, `chart_sprint`, `chart_feature`) executes
server-side, reads the AIDA substrate, computes data, renders SVG, and
returns a `ChartToolResult { artifacts: Vec<ChartArtifact>, summary:
String }`. The model sees only the `summary` as its `tool_result`
content (token-efficient, semantically sound). The `artifacts` flow
out-of-band:

1. The anthropic agent loop emits one `AgentEvent::ChartArtifact(art)`
   per artifact, immediately on tool completion.
2. The SSE encoder maps that to event type `chart` with a JSON-encoded
   `ChartArtifact { kind, svg, caption }`.
3. The browser's `EventSource` listener pushes each artifact onto a
   live signal `live_charts: Vec<ChartArtifact>`.
4. The frontend renders `<ChartArtifactView art=art/>` per artifact,
   inline below the assistant message body.
5. On turn done, artifacts are persisted in `ChatTurn.chart_artifacts`
   so the `/history` replay rebuilds them on a page reload.

Round trip: agent says "show me the status breakdown" → model picks
`chart_status` → tool reads `.aida-store/`, computes, returns 1 SVG +
summary text → browser sees `chart` event, renders artifact inline →
model receives the summary, narrates what the operator is seeing.

---

## 2. Time-series investigation

The brief flagged this as the key open question. Burn-down / burn-up
need to know, day by day, how many items had completed by that date.

### Finding: AIDA's distributed store already has the data

`.aida-store/objects/{TYPE}/{NNN}/{SPEC}.yaml` contains, for every
requirement, a `history:` array. Each entry is a status-change journal
with `{timestamp, author, changes: [{field_name, old_value, new_value}]}`.

A real example from `aida/.aida-store/objects/SPRINT/000/SPRINT-3.yaml`:

```yaml
history:
- timestamp: 2025-12-14T18:23:59.640262399Z
  changes:
  - field_name: status
    old_value: Draft
    new_value: Planned
```

That's the exact shape `computeBurndownData` in `sprint-utils.ts` walks
to find each item's `Completed` transition date. Port — not invent —
landed without any fallback synthesis.

### Why neither `aida history --events` nor MCP works

- `aida history --events` (no `--json` flag) returns formatted text.
- The MCP server's `show_requirement` returns a markdown rendering, not
  a structured envelope.
- `aida list --json` is a sparse projection — only `spec_id`, `title`,
  `req_type`, `status`, `tags`. No `weight`, `custom_fields`,
  `relationships`, `history`.

### Decision: read YAML directly

The Rust side reads `.aida-store/objects/**/*.yaml` via `serde_yaml` +
`walkdir`. ~150 LOC in `src/server/charts/store.rs`, no network round
trip, parses 1463 requirements (aida core) in <100ms on the dev box.

### Soft-fallback for items without history entries

For an item that's `Completed` but has no status-change journal entry
(e.g. legacy items, or items that were Completed before AIDA started
journaling), the algorithm falls back to `modified_at`. This matches the
React reference exactly. Documented in `Requirement::completed_date`.

### Future-work hook for AIDA core

It would be cleaner long-term if AIDA exposed `history` over MCP and
CLI (e.g. `aida show <id> --json --include=history`). I have not filed
this as a TASK in the aida repo because that's a multi-agent
coordination call. Recommended title:

> `aida show --json` — emit full Requirement record including history, custom_fields, relationships

Filed by a follow-on once the operator/advisor confirm the cross-repo
dependency direction.

---

## 3. Data shape divergence from the React reference

The brief warned: port — don't reinvent — `sprint-utils.ts`. One
divergence was unavoidable, one was a refinement.

### Sprint membership direction (unavoidable)

The React reference's `getSprintAssignmentTarget(req)` assumed each
item points at a sprint via `Custom("sprint_assignment")`. AIDA's
**actual** data uses the inverse: each sprint points at its members via
`Custom("sprint_contains")` (sprint → item). 4 of 4 sprints in the aida
core substrate use `sprint_contains`; 0 use `sprint_assignment`.

The Rust port handles both:

```rust
// Primary path: sprint → item
for rel in &sprint.relationships {
    if rel.is_sprint_contains() { … }
}
// Compat path: item → sprint (any item pointing at THIS sprint)
for other in &all_items {
    for rel in &other.relationships {
        if rel.is_sprint_assignment() && rel.target_id == sprint.id { … }
    }
}
```

So the chart layer works regardless of which direction a project's
sprint authoring tool uses. The React reference will need this same
adjustment when it next pulls real aida-core data.

### Weight defaulting

`Requirement.weight` (story points) is documented in AIDA's schema but
isn't populated on any of the 1463 requirements in the aida core
substrate. The React reference's velocity computation falls back to
`i.weight ?? 1`. The Rust port uses the same default: `weight: u32 =
1` when missing. Documented inline in `compute_velocity`.

---

## 4. Theming

aida-chat exposes 8 CSS custom properties on `:root`:
`--bg`, `--bg-elev`, `--border`, `--text`, `--text-dim`, `--accent`,
`--accent-dim`, `--error`. The charts use all but `--bg-elev`:

| SVG attribute / value | aida-chat var | Fallback in markup |
|---|---|---|
| Outer `color` (drives `currentColor` for gridlines, axis labels) | `--text-dim` | `#8b93a3` |
| Donut center number, bar value labels, title text | `--text` | `#e6e8ee` |
| Feature progress bar fill | `--accent` | `#7c9cff` |
| Donut segment separator stroke | `--bg` | `#0f1115` |

aida-web-react's categorical palette (blue burndown, emerald burnup,
violet velocity, etc.) is preserved as explicit hex literals — those
colors are part of the visual language and aren't substitutable. The
chart palette would only change if/when aida-chat introduces a
categorical-color vocabulary.

---

## 5. Empty states

Every chart has a graceful empty branch. The brief was unambiguous on
this:

> Empty states render gracefully on a no-sprint / no-data project.

Concrete behavior, verified against aida-chat's own substrate (which
has no sprints):

| Chart | Empty case | What renders |
|---|---|---|
| Status | `items.is_empty()` | Dashed-border placeholder svg: "No requirements to chart." |
| Burn-down | < 2 points | Dashed-border placeholder: "Not enough data for burndown — sprint needs a start_date and end_date in custom_fields." |
| Burn-up | < 2 points | Same shape, "Not enough data for burn-up …" |
| Velocity | 0 sprints | "No sprints found. Velocity needs requirements of type=Sprint." |
| Feature progress | 0 feature groups | "No requirements grouped by feature." |

The `chart_sprint` tool itself ALSO returns three empty-state artifacts
when called against a project with zero sprints, so the operator sees
WHAT IS MISSING rather than getting a tool-error or silence. Screenshot
proof in `docs/aida/2026-05-25-claude-charts-screenshots/`:
`burndown-aida-chat.svg`, `burnup-aida-chat.svg`,
`velocity-aida-chat.svg`.

---

## 6. Files changed

New code (all gated to `ssr`):

| File | Role | LOC |
|---|---|---|
| `src/server/charts/mod.rs` | Module root + re-exports | 25 |
| `src/server/charts/store.rs` | `.aida-store/objects/**/*.yaml` loader, typed `Requirement` model, sprint membership resolution | ~410 |
| `src/server/charts/data.rs` | Algorithm ports: status counts, burndown/burnup, velocity, feature progress, plus calendar arithmetic | ~440 |
| `src/server/charts/svg.rs` | 5 hand-crafted SVG renderers with empty-state branches | ~470 |
| `src/server/tools/charts.rs` | 3 agent tools (`chart_status`/`chart_sprint`/`chart_feature`) | ~270 |
| `examples/charts_smoke.rs` | End-to-end smoke test | ~120 |
| `docs/aida/2026-05-25-claude-charts-arch.md` | This document | — |
| `docs/aida/2026-05-25-claude-charts-screenshots/*.svg` | 10 screenshots (5 charts × 2 substrates) | — |

Modified code:

| File | Why |
|---|---|
| `Cargo.toml` | Adds `serde_yaml` + `walkdir` to the `ssr` feature |
| `src/messages.rs` | Adds `ChartArtifact` shape + `ChatTurn.chart_artifacts` field |
| `src/server/mod.rs` | Declares `pub mod charts;` |
| `src/server/agent.rs` | New `AgentEvent::ChartArtifact` variant |
| `src/server/api.rs` | SSE encoder maps `ChartArtifact → event:chart` |
| `src/server/backends/anthropic.rs` | Dispatches `chart_*` tools through `dispatch_chart`, emits artifacts, persists to ChatTurn, updates system prompt to advertise the new tools |
| `src/server/backends/claude_cli.rs` | Defaults `chart_artifacts: vec![]` on its ChatTurn construction |
| `src/server/sessions.rs` | User-turn ChatTurn gets `chart_artifacts: vec![]` |
| `src/server/tools/mod.rs` | Registers the 3 chart tool specs + adds `is_chart_tool` / `dispatch_chart` |
| `src/app.rs` | Adds `live_charts` signal, `chart` SSE listener, `<ChartArtifactView>` component, renders artifacts in both live and historical paths |
| `style/main.css` | `.chart-gallery`, `.chart-artifact`, `.chart-svg`, `.chart-caption` |

Tests:

| Module | Coverage |
|---|---|
| `charts::store::tests` | YAML parser smoke (real SPRINT-3 fixture), history-based `completed_date`, `modified_at` fallback |
| `charts::data::tests` | Calendar arithmetic, burndown/burnup point-by-point assertions, status canonical ordering, feature grouping, empty inputs |
| `charts::svg::tests` | Each renderer's happy + empty paths, XML escaping, color presence assertions |
| `tools::charts::tests` | `chart_status` against aida core (skipped if `.aida-store` isn't checked out) |

Total: **57 lib tests** pass.

---

## 7. Open questions / future-work hooks

1. **AIDA core MCP enhancement.** A `aida show --json --include=history`
   (or richer `show_requirement` MCP envelope) would let aida-chat drop
   the direct YAML reader and become substrate-shape-agnostic. Filed as
   a follow-on TASK candidate.
2. **Per-chart click-through.** V2 brief item: tap a velocity bar to
   open that sprint's detail view; tap a feature row to open `aida_list
   --feature=…`. Requires option-3 (hybrid) rendering. Not in V1.
3. **Cumulative flow diagram + dependency graph + cycle time.** Brief
   listed these as "if you have time after V1" — punted. They use the
   same data layer; renderers would add ~400 LOC each.
4. **Story-points / weight surfacing in AIDA.** Velocity defaults to 1
   point per item until weight gets populated. Once weight is widely
   set, velocity reads will become meaningfully different from
   throughput. Worth coordinating with the aida-core team on a
   weight-input ergonomic before the V2 charts ship.
5. **Live-edit of charts.** Operator might want to drag a date slider to
   re-scope burn-down. V2+. Same option-3 lift.
6. **Per-sprint feature-progress filtering.** Currently feature progress
   is project-wide. A sprint-scoped variant (only items in
   sprint_contains) would close the analytics loop. ~30 LOC change to
   `compute_feature_progress`.

---

## 8. Divergences from the aida-web-react reference

| What | Reference | This implementation | Why |
|---|---|---|---|
| Sprint-membership rel name | `Custom("sprint_assignment")` (item → sprint) | Honors both `sprint_assignment` AND `sprint_contains` (sprint → item) | Real AIDA data uses `sprint_contains`; supporting both keeps the React side compatible if it ever pulls real data |
| Burndown chart frame | 400 × 200 viewBox | 400 × 220 viewBox | Extra 20px of bottom padding to comfortably fit the legend below the X-axis labels — the React version's legend wraps with the chart in a flex container, which a server-rendered standalone SVG can't rely on |
| Theming token | Tailwind `text-content-secondary` | aida-chat `var(--text-dim)` via `style="color:…"` | Different host project's design vocabulary |
| Single sprint vs. multi-sprint | Per-component | `chart_sprint` returns three artifacts (burn-down + burn-up + velocity) in one tool call | Inline-in-chat works best when "show me the sprint" yields one cluster of charts, not a dashboard URL |
| Title font | Hidden (host renders title above SVG) | Inline title `<text>` in each SVG | Each SVG must be self-explanatory when viewed standalone (screenshot, copy-paste, history replay) |

No invention beyond these — every algorithm in `data.rs` traces 1:1 to
a function in `sprint-utils.ts`, with the source function name reused
verbatim for grep-ability. Color hex values come from the React
reference, not picked anew.

---

## 9. Self-evaluation against the brief's 6 criteria

1. **Visual quality** — ★★★★☆.
   Polished: dark theme native, gridlines at the right opacity,
   non-trivial donut paths with proper SVG arc math, legend layout,
   muted empty-state placeholders. One nit: the React reference
   produces a subtle "rounded line caps" feel that I haven't matched
   in the bar charts — minor.
2. **Code quality** — ★★★★★.
   57 tests including the YAML parser smoke against a real SPRINT-3
   fixture. Pure-Rust SVG generator (no `format!` HTML escapes leaking
   through). Clear separation: store → data → svg → tool → SSE → UI.
3. **Faithfulness to aida-web-react** — ★★★★★.
   Algorithms ported verbatim with source function-name preservation.
   Color palette identical. The one divergence (sprint-membership
   direction) is forced by the real data and documented.
4. **Investigation depth** — ★★★★★.
   Time-series data found in `.aida-store` history journals, true
   burn-down landed (not a fallback). Sprint-membership direction
   discovered and handled bidirectionally. AIDA-core MCP-enhancement
   future-work hook identified.
5. **Brief discipline** — ★★★★★.
   All 5 V1 charts shipped. Both substrate targets verified (aida core
   + aida-chat). Empty states for every chart. ssr + hydrate + cargo
   leptos all green. `cargo run --example charts_smoke` works end-to-end.
   Backend tools + SSE + frontend artifact rendering + history
   persistence all in.
6. **Trace discipline** — ★★★★★.
   Every new file has `// trace:EPIC-29 | ai:claude` at the top.
   Cross-cutting concerns (e.g. the SSE event variant, the ChatTurn
   field, the system-prompt update) carry inline `// trace:EPIC-29 |
   ai:claude` markers at each touchpoint.

**One thing I'd do with another night:** Polish the donut chart's
center label (the total number) so it scales with the segment count —
right now it's a fixed 22px font, which gets visually weak when there
are 7+ segments crowding the legend. ~30min of design work.

---

— claude/epic-29-charts-overnight, 2026-05-25
