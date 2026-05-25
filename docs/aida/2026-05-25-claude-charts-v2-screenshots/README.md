# EPIC-29 V2 — Claude chart screenshots (2026-05-25)

Three new charts × two substrates = six SVGs. All produced by the
extended `cargo run --example charts_smoke --features ssr`:

```bash
# aida core: SPRINT-3 has 6 sprint_contains rels, perfect for the dep graph.
AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida \
  AIDA_CHAT_CHARTS_OUT=/tmp/charts-v2-aida \
  AIDA_CHAT_CHARTS_DEP_ROOT=SPRINT-3 \
  cargo run --example charts_smoke --features ssr

# aida-chat: EPIC-16 (no outgoing rels — shows the sparse-graph case).
AIDA_CHAT_REPO_ROOT=/home/joe/ai/aida-chat \
  AIDA_CHAT_CHARTS_OUT=/tmp/charts-v2-aida-chat \
  AIDA_CHAT_CHARTS_DEP_ROOT=EPIC-16 \
  cargo run --example charts_smoke --features ssr
```

| Chart | `aida-core/` | `aida-chat/` |
|---|---|---|
| `cfd.svg` | 30 days × 8 active status buckets, 1471 reqs replayed via history journal | 30 days × 4 active status buckets, 33 reqs (all recent — most of the chart shows them transitioning from Draft → Approved/Completed in the last week) |
| `dep_graph.svg` | SPRINT-3 root, 11 nodes / 18 edges, truncated at depth 2 — shows a sprint's `sprint_contains` web with the 6 member items + their parents | EPIC-16 root, 1 node (no outgoing relationships) — exercises the sparse-graph case so the layout doesn't break on a trivial graph |
| `cycle_time.svg` | n=0 — **interesting finding**: the aida-core substrate doesn't journal `Approved` transitions for most items; strict reading of the brief (require both Approved + Completed) yields no samples. Empty-state placeholder renders. | n=0, same reason as aida-core |

The cycle-time empty result is a real-data finding worth surfacing.
The brief stipulated "find Approved transition, find Completed
transition, subtract." Strict reading wins; the empty-state branch
handles it cleanly. **Architecture-doc note:** if AIDA's authoring flow
typically goes `Draft → InProgress → Completed` (skipping `Approved`),
this chart will stay sparse until either (a) the substrate convention
changes or (b) we relax the algorithm to use the earliest non-Draft
transition. Latter is a one-line edit; I left it strict per the brief
so the result is auditable.

Open the SVGs in any browser to view. They use `var(--text)` /
`var(--accent)` / `var(--bg)` so they inherit aida-chat's dark theme
in-app, with hex fallbacks for standalone viewing.
