# Codex System Improvement Audit — 2026-05-26

Context: post-merge audit after EPIC-26, EPIC-29, STORY-14, STORY-23, and STORY-25 landed on `master`.

## What I Fixed Immediately

- Made `cargo clippy --features ssr --lib -- -D warnings` pass on a follow-up cleanup branch.
- The warnings were low-risk but useful signal:
  - empty Leptos `view! {}` expressions in STORY-25 drift UI,
  - a manual `if let Err` that should use `?`,
  - a chart SVG helper with too many arguments,
  - an identical chart label-color branch,
  - a needless `as_bytes().len()` call in the memory validator.

## Highest-Leverage Follow-Ups

1. **Add CI for the current verification baseline.**

   Recommended required checks:
   - `cargo test --features ssr --lib`
   - `cargo build --features hydrate --lib`
   - `cargo leptos build`
   - `cargo clippy --features ssr --lib -- -D warnings`

   Rationale: the post-merge branch already had clippy regressions even though functional tests were green. This is cheap, catches cross-PR polish debt, and protects the demo surface.

2. **Harden query-log completion semantics.**

   Current `/api/chat` starts a query row before canned-answer matching and marks completion when the SSE stream emits `Done` or `Error`. Rows can remain unfinished if:
   - canned-answer library load fails after insert,
   - the client disconnects before the terminal event is polled,
   - the agent task dies without emitting a terminal event.

   Recommended shape: add `status` (`started|ok|error|abandoned`) and `error` fields, plus a cleanup/reconciliation pass for stale open rows. This turns EPIC-26’s log into reliable operational data instead of best-effort telemetry.

3. **Split drift verification into gather + classify stages.**

   STORY-25 is useful, but the endpoint is currently all-or-nothing: if one Anthropic classifier call fails or returns invalid JSON, the whole request fails. Better V2:
   - return per-site errors as findings with `severity="unknown"` or a separate `error`,
   - classify sites concurrently with a small semaphore,
   - expose token/site budget controls,
   - cache `aida_show` + trace context for repeated operator checks.

4. **Cap or page persisted tool-call audit payloads.**

   STORY-14 deliberately stores full tool input/output. That is correct for auditability, but `/api/sessions/:id/history` now has a future payload-size risk. V1 can trust current read caps, but the next reliability pass should add:
   - max persisted output size with explicit truncation metadata, or
   - a separate `/tool-calls/:id` detail endpoint and compact history rows.

5. **Promote auth/rate-limit drafts before multi-user exposure.**

   `STORY-9`, `STORY-11`, and `STORY-12` are still draft. The system now has write endpoints for AIDA comments/specs, memory files, and LLM-backed drift checks. Before exposing beyond a local trusted operator, those should move ahead of additional product surface.

6. **Make capture endpoint plumbing less repetitive.**

   `/comment`, `/spec`, `/memory`, and `/verify-drift` all repeat the same session check and `{ok,error}` mapping. A small helper would reduce contract drift risk as STORY-24 and future capture loops land.

7. **Add a runtime health/debug endpoint.**

   Useful fields:
   - backend,
   - repo_root,
   - MCP available / last error,
   - canned-answer count,
   - query-log DB path,
   - active session count,
   - claude-cli live process count when applicable.

   This would make demo debugging much faster than inferring state from logs.

## Suggested Next PR Order

1. CI baseline PR with the four commands above.
2. EPIC-26 query-log reliability PR: status/error fields and stale-open-row reconciliation.
3. STORY-25 drift verifier resilience PR: partial results instead of all-or-nothing.
4. Security hardening PR: auth/rate limit story promotion plus minimal local-token gate if needed.

