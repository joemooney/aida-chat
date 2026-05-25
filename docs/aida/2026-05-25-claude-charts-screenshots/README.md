# EPIC-29 V1 — Claude chart screenshots (2026-05-25)

Five charts × two substrates = ten SVGs. Each SVG was produced by
`cargo run --example charts_smoke --features ssr` with the
corresponding `AIDA_CHAT_REPO_ROOT` set. Both runs were against unchanged
`.aida-store/` directories, no code touched between them — the same
pipeline produces both sets.

| Chart | aida core substrate | aida-chat substrate |
|---|---|---|
| Status distribution | `status-aida-core.svg` — 7 status buckets across 1463 reqs (live data) | `status-aida-chat.svg` — 4 status buckets across 33 reqs |
| Sprint burn-down | `burndown-aida-core.svg` — SPRINT-3 (2026-03-06 → 2026-03-13), 6 members | `burndown-aida-chat.svg` — **empty state** (no sprints in this substrate) |
| Sprint burn-up | `burnup-aida-core.svg` — same SPRINT-3 window, area-fill under completion line | `burnup-aida-chat.svg` — **empty state** |
| Velocity | `velocity-aida-core.svg` — 3 bars (SPRINT-1/2/3, all 0 completed) with avg line | `velocity-aida-chat.svg` — **empty state** ("No sprints found") |
| Feature progress | `feature_progress-aida-core.svg` — top 8 features by total | `feature_progress-aida-chat.svg` — single `Uncategorized` row at 21% |

Both substrate paths render through the **same code**: there is no
substrate-specific branching in the chart layer. The aida-chat run hits
the empty-state branches gracefully (no panics, no broken layouts) — the
brief's "render gracefully on a no-sprint / no-data project" acceptance
criterion in action.

The chart-smoke tool reports the data it found before rendering:

```
# aida core (1463 reqs, 3 sprints, 140 feature groups)
✓ status: 7 buckets, total 1463
→ 3 sprint(s) in this substrate
→ sprint: SPRINT-3 (6 members, dates Some("2026-03-06")..Some("2026-03-13"))
✓ burn-down: 8 points
✓ burn-up: 8 points
✓ velocity: 3 sprint(s)
✓ feature progress: 140 feature group(s)

# aida-chat (33 reqs, 0 sprints, 1 feature group)
✓ status: 4 buckets, total 33
→ 0 sprint(s) in this substrate
✓ burn-down + burn-up: empty-state (no sprint with dates)
✓ velocity: 0 sprint(s)
✓ feature progress: 1 feature group(s)
```

Open the SVGs in any browser to view. They use `var(--text)` /
`var(--text-dim)` / `var(--accent)` / `var(--bg)` so they pick up
aida-chat's dark theme natively when rendered inside the chat UI.
Outside that context (e.g. in your file browser) they fall back to the
hex defaults baked into each `style="…"` attribute.
