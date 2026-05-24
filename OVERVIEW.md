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
- **Tool-call inspector.** Click a tool badge in the UI to see the full
  request + response, not just the name.

## Related work in the AIDA ecosystem

- [`aida-server/src/chat.rs`](../aida/aida-server/src/chat.rs) — earlier
  static-context chat backend, served via the React UI in `aida-web-react/`.
  Generic across projects; no tool use. aida-chat is the agentic
  counterpart, scoped to one project.
