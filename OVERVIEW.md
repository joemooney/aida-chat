# aida-chat: scoped chat UI for a single project

## Vision

A ChatGPT/Claude-style web UI you can point at any project repository to get
answers grounded in *just that repo* and *just that project's tracked
requirements*. Per-user sessions, streamed token-by-token, with the model
deciding what to look up via a small, audit-friendly tool surface.

## Target users

- Solo developers who want a project-scoped Q&A on top of their codebase + AIDA.
- Eventually: PMs / stakeholders who want to ask "what's the status of X" or
  "where is feature Y implemented" without ever cloning the repo.

## Why not just use Claude Code?

Claude Code is a general-purpose agent with broad tool access (bash, edit,
the whole filesystem). aida-chat is the opposite: a *narrow* read-only-by-default
agent that can answer questions about one project, surfaced as a chat UI
so a non-engineer can use it. The point isn't power; it's scope.

## Architecture

```
┌──────────────────────────┐         ┌────────────────────────────────────┐
│  Browser (Leptos+wasm)   │         │ aida-chat server (Axum + Leptos SSR)│
│                          │         │                                    │
│  - Chat input            │  POST   │  /api/sessions  ─► InMemorySessions│
│  - Streaming text + tool │ ──────► │  /api/sessions/:id/comment ─► aida │
│    badges (clickable)    │   GET   │  /api/sessions/:id/spec    ─► aida │
│  - Inline chart artifacts│ ──────► │  /api/sessions/:id/memory  ─► fs   │
│  - Capture-form          │   POST  │  /api/sessions/:id/verify-drift    │
│    affordances           │ ──────► │  /api/chat?session_id=…&q=…        │
│                          │  SSE    │       │                            │
│  - localStorage session  │ ◄────── │       ▼                            │
└──────────────────────────┘ events  │  canned matcher (.aida/canned-…)   │
                                     │       │ miss                       │
                                     │       ▼                            │
                                     │  agent::run_turn(...)              │
                                     │       │                            │
                                     │       ▼                            │
                                     │  Anthropic Messages API (stream)   │
                                     │       │  ↑                         │
                                     │       │  │ tool_result             │
                                     │       ▼  │                         │
                                     │  tools::dispatch                   │
                                     │   ├─ fs::{read_file,list_directory}│
                                     │   ├─ grep::grep_repo               │
                                     │   ├─ traces::find_traces           │
                                     │   ├─ aida::aida_{list,show,search, │
                                     │   │     history,resource,          │
                                     │   │     comment_add,add,ultraplan} │
                                     │   ├─ memory::write_memory          │
                                     │   ├─ drift::verify_trace_drift     │
                                     │   └─ charts::chart_{status,sprint, │
                                     │         feature,cfd,dep_graph,     │
                                     │         cycle_time}                │
                                     │                                    │
                                     │  ↳ aida_* → MCP (aida mcp-serve)   │
                                     │             with CLI fallback      │
                                     │  ↳ logged to .aida-chat/queries.db │
                                     └────────────────────────────────────┘
```

## Key design decisions

### Custom Rust agent loop, not `claude` CLI subprocess

We don't spawn `claude` per user. We talk to the Anthropic Messages API
directly with a curated `tools: [...]` array. This means:

- The set of things the model can do is exactly the set of tools we publish
  — no `Bash`, no arbitrary file system access, no network egress.
- Sandboxing comes from *our* tool implementations, not from configuring
  a third-party agent's permission system.
- No Node.js dependency for the server.

See [ADR-6](https://github.com/joemooney/aida-chat) for the full rationale.

### Tool surface, not pre-stuffed system prompt

The existing `aida-server/src/chat.rs` in the AIDA repo dumps the full
requirements summary + git context into the system prompt at request time.
That works for small projects but burns tokens and goes stale mid-turn.

aida-chat does the opposite: a short system prompt that tells the model
*what tools exist*, and the model calls them on demand. This keeps the
context small, stays automatically fresh, and scales to large
codebases / large requirement sets.

This is [ADR-7](https://github.com/joemooney/aida-chat) — the substrate-grounded
thesis. Every factual claim the model makes is attributable to a tool call
the user can see in the audit trail.

### Path confinement, centralized

Every filesystem tool routes through `resolve_within_repo(repo_root, path)`:

1. Reject empty paths and absolute paths up front.
2. Join against `repo_root`, then `canonicalize` (resolves `..`, symlinks).
3. Verify the canonical result still starts with the canonical `repo_root`.
4. Reject anything passing through `.git/`.

This is one helper. Every filesystem-touching tool calls it. There is no
"trust the input path" code path.

`write_memory` is the one deliberate exception — it writes to
`~/.claude/projects/<slug>/memory/`, *outside* `repo_root`. The exception
is scoped tightly: the destination directory is computed from `cfg.repo_root`
(never from model/user input), filenames are validated against a kebab-slug
regex, and the path is canonicalized + checked to be inside the memory dir
before writing. See `src/server/tools/memory.rs:resolve_within_memory_dir`.

### Pluggable session store

`SessionStore` is a trait. v0 has `InMemorySessions` (HashMap behind a
Mutex, idle-TTL eviction). A future Postgres/sled-backed implementation
can drop in without touching agent or HTTP layers.

### MCP-first integration with CLI fallback

aida-chat reads AIDA's substrate via the Model Context Protocol — one
long-lived `aida mcp-serve` subprocess per aida-chat process, lazy-spawned
on first use. `src/server/mcp/client.rs` implements a custom JSON-RPC 2.0
stdio client (~430 LOC, no external MCP framework dependency).

Failure mode: if the MCP subprocess dies, the reader task clears the
global client slot; the next `McpClient::global()` call respawns a fresh
subprocess. In-flight `Closed`/`Timeout` errors map to `Unavailable`,
which triggers a CLI fallback (`aida list --json`, `aida show <ID>`,
etc.) for the four legacy `aida_*` tools that have a CLI equivalent.
Some tools (e.g. `aida_resource`) are MCP-only and surface a clean error
when MCP is unreachable rather than silently degrading.

The failure-model doc-comment in `src/server/mcp/client.rs:18-28`
documents the policy. The respawn behavior is verified by the
`global_respawns_after_child_death` unit test.

## Substrate capabilities

These are the things aida-chat does that a generic chat UI can't, because
they require a live link to the project's tracked substrate (specs, traces,
plans, history).

### Substrate-grounded answers (EPIC-16)

The system prompt instructs the model to cite a SPEC-ID or `path:line` for
every factual claim. The model calls `find_traces`, `aida_show`,
`aida_search`, `aida_list`, `aida_resource`, `read_file`, `grep_repo` as
needed, and the UI surfaces every tool call as a badge below the
assistant message. The audit trail is part of the user experience, not a
debug feature.

### Capture loops (STORY-21..25)

Each assistant message offers inline capture affordances that turn the
conversation into substrate growth:

- **Save as comment** (STORY-21) — appends to a SPEC via `aida comment add`.
- **Create as new SPEC** (STORY-22) — files a new task/bug/story/epic via
  `aida add`, returning the new SPEC-ID inline.
- **Save as memory** (STORY-23) — writes a markdown memory file to
  `~/.claude/projects/<slug>/memory/` with YAML frontmatter, picked up
  automatically by both Claude Code and aida-chat next session.
- **Seed a plan** (STORY-24, pending; `aida_ultraplan` tool exists, full
  UI is in flight) — assembles a structured planning prompt for a SPEC.
- **Verify trace drift** (STORY-25) — for a given SPEC-ID, finds every
  `// trace:SPEC-ID` in the repo, asks a focused LLM to classify whether
  each trace site still implements the spec; returns a per-site findings
  table with severity tags and an option to file a comment back to the
  SPEC.

### Fast-response routing (EPIC-26 slices A+B)

A SQLite query log at `.aida-chat/queries.db` records every user query
with timestamp, latency, and a `served_from` tag (`canned` / `llm`).
Before entering the agent loop, the canned-answer matcher in
`src/server/canned.rs` checks `.aida/canned-answers.toml` for an
exact-match or case-insensitive-contains hit. Matches are streamed back
via SSE without invoking the LLM. A small template syntax (`{{backend}}`)
keeps canned answers usable for dynamic-state questions.

Slices C (skill registry), D (review workflow), and E (UI star
affordance) are queued for follow-up.

### Agile metrics charts (EPIC-29)

Server-rendered SVG charts produced inline as part of the model's
response. Chart tools query the substrate (via AIDA's MCP server when
available, falling back to direct YAML reads from `.aida-store/objects/`)
and emit a `ChartArtifact` over a dedicated SSE `chart` event, which the
frontend hydrates as an inline `<ChartArtifactView>`. The model sees only
a short summary string in its `tool_result` content — no SVG token cost.

V1 tools (shipped):

- `chart_status` — status distribution donut + legend.
- `chart_sprint` — burndown + burnup + velocity composite for a sprint.
- `chart_feature` — feature progress bars (epic-level rollup).

V2 tools (shipped):

- `chart_cfd` — cumulative flow diagram over a configurable window.
- `chart_dep_graph` — relationship-graph rooted at a SPEC-ID.
- `chart_cycle_time` — histogram of Approved → Completed durations.

Burndown / burnup / CFD / cycle-time are *true* time-series — the data
layer reads the per-requirement `history` array embedded in each
`.aida-store/objects/**/*.yaml`, walking status-transition timestamps.
When a project's substrate has no history yet (fresh import), the
fallback uses `modified_at` for completed items.

The chart algorithms are ported from
[`aida-web-react/src/lib/sprint-utils.ts`](../aida/aida-web-react/src/lib/sprint-utils.ts);
the hand-crafted SVG approach (no chart library dep) matches the React
implementation's ethos.

### Tool-call inspector (STORY-14)

Tool badges in the UI are clickable. Clicking expands an inline
`<ToolCallPanel>` showing the full tool input (pretty-printed JSON), the
full output text, the call duration in human-readable form, and the
success/failure status. Both live (during SSE streaming) and historical
(rendered from `/history`) badges expand. The substrate audit trail
becomes inspectable in the same UI surface the answers live in.

## Configuration

| Env var                 | Default                  | Purpose |
|-------------------------|--------------------------|---------|
| `AIDA_CHAT_BACKEND`     | autodetect               | `anthropic` or `claude-cli`. Autodetect prefers anthropic when `ANTHROPIC_API_KEY` is set. |
| `ANTHROPIC_API_KEY`     | required for anthropic   | Loaded from `.env` or shell env. Stripped from claude-cli subprocess env. |
| `AIDA_CHAT_MODEL`       | `claude-sonnet-4-6`      | Anthropic backend model. |
| `AIDA_CHAT_REPO_ROOT`   | `$PWD` at startup        | The agent's tools are confined to this directory. **Point at any AIDA project here.** |
| `AIDA_CHAT_MCP_COMMAND` | `aida`                   | Command to launch the MCP server. |
| `AIDA_CHAT_MCP_ARGS`    | `mcp-serve`              | Args passed to the MCP command. |

To demo aida-chat against AIDA core itself: `AIDA_CHAT_REPO_ROOT=/path/to/aida cargo leptos serve`.
The dense substrate (1000+ requirements, active sprints, dense trace
network) makes for a richer demo surface than aida-chat's own.

## Roadmap

Day-1 outcomes (shipped):

- Substrate-grounded answers via MCP-first tool surface (EPIC-16).
- Capture loops for comment / new-SPEC / memory writes (STORY-21/22/23).
- Tool-call inspector with timing + full payload (STORY-14).
- Bypass-removal post upstream AIDA fix to BUG-377 (clean MCP write path).

Day-2 outcomes (shipped):

- Agile metrics charts V1 + V2 (EPIC-29).
- Fast-response query routing — query log + canned matcher (EPIC-26 A+B).
- Trace-drift detection (STORY-25).

Pending / in flight:

- STORY-24 chat → plan-seed full-stack (backend + UI affordance for
  `aida ultraplan`).
- EPIC-26 slice C (skill registry — pre-bound tool sequences for common
  query patterns).
- EPIC-26 slice D (FAQ-review workflow — periodic AI-driven proposal of
  new canned/skill candidates from the query log).
- EPIC-26 slice E (star/upvote UI affordance feeding the review pipeline).

Future (multi-user, post-MVP — EPIC-8 + children):

- **Auth.** OAuth, magic links, basic SSO in front of the API routes.
- **Per-user API key isolation.** Currently one server-wide
  `ANTHROPIC_API_KEY`. Multi-user wants per-user keys, or at least
  billing attribution.
- **Persistent sessions.** Implement `SessionStore` against Postgres so
  conversations survive restarts.
- **Rate limits.** Per-session and per-user caps on tool iterations and
  tokens.
- **Conversation export.** Download a session as markdown.

## Related work in the AIDA ecosystem

- [`aida-web-react`](../aida/aida-web-react/) — earlier React+TypeScript
  dashboard for the same substrate. Static, no chat. aida-chat ports the
  algorithmic IP (sprint-utils.ts → charts module) and adds an agentic
  chat surface on top.
- [`aida-server/src/chat.rs`](../aida/aida-server/src/chat.rs) — earlier
  static-context chat backend, served via the React UI. Generic across
  projects; no tool use. aida-chat is the agentic counterpart, scoped to
  one project at a time.
- AIDA core (`~/ai/aida`) — the requirements substrate this consumes.
  aida-chat is AIDA's first non-dogfood consumer; the
  substrate-as-product positioning is validated empirically by what this
  project can do.
