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
the whole filesystem). aida-chat is the opposite: a *narrow* read-only
agent that can answer questions about one project, surfaced as a chat UI
so a non-engineer can use it. The point isn't power; it's scope.

## Architecture

```
┌──────────────────────────┐         ┌────────────────────────────────────┐
│  Browser (Leptos+wasm)   │         │ aida-chat server (Axum + Leptos SSR)│
│                          │         │                                    │
│  - Chat input            │  POST   │  /api/sessions  ─► InMemorySessions│
│  - Message list          │ ──────► │                                    │
│  - EventSource (SSE)     │   GET   │  /api/chat?session_id=…&q=…        │
│                          │ ──────► │       │                            │
│  - localStorage session  │  SSE    │       ▼                            │
│                          │ ◄────── │  agent::run_turn(...)              │
└──────────────────────────┘ events  │       │                            │
                                     │       ▼                            │
                                     │  Anthropic Messages API (stream)   │
                                     │       │  ↑                         │
                                     │       │  │ tool_result             │
                                     │       ▼  │                         │
                                     │  tools::dispatch                   │
                                     │   ├─ fs::read_file       ◄─ repo   │
                                     │   ├─ fs::list_directory  ◄─ repo   │
                                     │   ├─ grep::grep_repo     ◄─ repo   │
                                     │   └─ aida::aida_*        ◄─ aida   │
                                     └────────────────────────────────────┘
```

## Expanded Tool Surface

A set of dedicated AIDA-integrated capabilities allows the agent to safely read, modify, and analyze the requirement substrate:
- `find_traces`: Scans the repository for inline `// trace:ID` comments to map requirement implementation sites.
- `aida_comment_add`: Appends design notes, technical clarifications, or stakeholder comments directly to an active specification.
- `aida_add`: Files new requirement items (stories, tasks, bugs) directly into the local AIDA canonical requirement store.
- `write_memory`: Commits long-term session learnings, architectural constraints, or project context to the persistent workspace memory.
- `aida_ultraplan`: Generates a structured planning prompt to seed high-fidelity implementation plans for approved specifications.
- `verify_trace_drift`: Checks for discrepancies and trace-graph drift between committed code traces and registered requirements.
- `aida_resource`: Exposes read access to headless, non-requirement workspace assets like plan archives or system summaries.
- `chart_status`: Renders status distribution metrics of the requirement substrate as high-fidelity vector SVGs.
- `chart_feature`: Produces visually rich vector SVGs illustrating progress metrics mapped by parent feature categories.
- `chart_sprint`: Generates burndown/burnup velocity tracking vector SVGs for requirement items assigned to the active sprint.


## Key design decisions

### Custom Rust agent loop, not `claude` CLI subprocess

We don't spawn `claude` per user. We talk to the Anthropic Messages API
directly with a curated `tools: [...]` array. This means:

- The set of things the model can do is exactly the set of tools we publish
  — no `Bash`, no arbitrary file system access, no network egress.
- Sandboxing comes from *our* tool implementations, not from configuring
  a third-party agent's permission system.
- No Node.js dependency for the server.

### Tool surface, not pre-stuffed system prompt

The existing `aida-server/src/chat.rs` in the AIDA repo dumps the full
requirements summary + git context into the system prompt at request time.
That works for small projects but burns tokens and goes stale mid-turn.

aida-chat does the opposite: a short system prompt that tells the model
*what tools exist*, and the model calls them on demand. This keeps the
context small, stays automatically fresh, and scales to large
codebases / large requirement sets.

### Path confinement, centralized

Every filesystem tool routes through `resolve_within_repo(repo_root, path)`:

1. Reject empty paths and absolute paths up front.
2. Join against `repo_root`, then `canonicalize` (resolves `..`, symlinks).
3. Verify the canonical result still starts with the canonical `repo_root`.
4. Reject anything passing through `.git/`.

This is one helper. Every filesystem-touching tool calls it. There is no
"trust the input path" code path.

### Pluggable session store

`SessionStore` is a trait. v0 has `InMemorySessions` (HashMap behind a
Mutex, idle-TTL eviction). A future Postgres/sled-backed implementation
can drop in without touching agent or HTTP layers.

## Core Substrate Grounding and Protocols

### Substrate-Grounded Q&A (ADR-7, EPIC-16)
The agent operates under a **substrate-grounded execution model**. Grounding is enforced via a strict, multi-layered tool surface (specified in **ADR-7**), guaranteeing that all factual claims made by the agent are derived dynamically from code, trace comments, or registered requirements:
- **MCP-First Architecture**: The agent preferentially connects to an active MCP (Model Context Protocol) server to access the requirement substrate, maintaining low latency and protocol compliance.
- **CLI Fallback Execution**: In environments where a standalone MCP server is unavailable, the backend safely falls back to standard, path-confined subprocess calls to the local `aida` CLI.
- **Protocol Resolution (BUG-377)**: High-precision argument validation and strict string escaping ensure that trace-matching commands handle edge cases (such as adjacent punctuation or mixed-cased prefixes) cleanly without breaking process routing.

### Virtuous Capture Loops (STORY-21/22/23/24/25)
To ensure the requirement substrate remains a living, self-enriching artifact, the system implements five virtuous capture loops. These loops turn transient chat interactions into durable workspace resources:
1. **Comment Loop (`aida comment add`)**: Instantly captures design insights or decisions from the chat turn and binds them directly to the related specification.
2. **Spec Loop (`aida add`)**: Permits the assistant to promote discovered tasks, bugs, or user stories directly into the canonical database.
3. **Memory Loop (`write_memory`)**: Stores long-term architectural constraints and workspace-specific contextual patterns.
4. **Plan Seed Loop (`aida ultraplan`)**: Automatically assembles a comprehensive, structured plan brief from the trace-graph and spec details to seed downstream development.
5. **Skill Loop (`aida import-plan`)**: Feeds verified implementations back into the runtime environment as active, executable skills.

### High-Performance Query Routing (EPIC-26)
To achieve sub-100ms response times for common queries and reduce LLM execution costs, the server implements an optimized, multi-tier fast-response pipeline:
- **Query Logging**: Captures precise performance history, latency statistics, and process audit trails.
- **Canned-Answer Matcher**: An in-memory cache that evaluates incoming user requests against a canned-answers registry. Matches are served instantly over Server-Sent Events (SSE), bypassing the agent loop entirely.
- **Unified Skill Registry**: Acts as the extension boundary, routing specialized commands directly to automated scripts or targeted CLI executions.

### Integrated Agile Metrics (EPIC-29)
The system visualizes requirement progress directly within the chat UI using premium vector SVG charts:
- **Dual-Substrate Capability**: Reducers process metrics across both the lightweight local project substrate and dense, distributed multi-repo requirements, handling empty states and large datasets gracefully.
- **V1 Render Pipeline**: High-fidelity chart generation runs on a pure Rust pipeline (porting React sprint-utility algorithms), emitting CSS-themed vector graphics with grid alignment and dynamic colors matching the dark theme.

### Interactive Tool-Call Inspector (STORY-14)
Clickable tool badges are rendered under each assistant response to surface the active audit trail. Clicking a badge reveals the complete substrate exchange (input parameters, return status, and captured output payloads), giving developers instant visibility into the model's evidence chain.

## Roadmap

This is v0. Concrete next steps when multi-user becomes real:

- **Auth.** Whatever your auth story is (oauth, magic links, basic SSO) —
  drop it in front of the API routes.
- **Per-user API key isolation.** Right now there's one server-wide
  `ANTHROPIC_API_KEY`. Multi-user wants per-user keys, or at least billing
  attribution.
- **Persistent sessions.** Implement `SessionStore` against Postgres so
  conversations survive restarts.
- **Rate limits.** Per-session and per-user caps on tool iterations and
  tokens.
- **Conversation export.** Download a session as markdown.

## Related work in the AIDA ecosystem

- [`aida-server/src/chat.rs`](../aida/aida-server/src/chat.rs) — earlier
  static-context chat backend, served via the React UI in `aida-web-react/`.
  Generic across projects; no tool use. aida-chat is the agentic
  counterpart, scoped to one project.
