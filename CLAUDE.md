# CLAUDE.md

Guidance for Claude Code working in this repository. AIDA conventions
(trace format, commit format, daily commands, capture rules) live in
`.claude/AIDA.md` — Claude Code expands the import below automatically,
so you'll see them in context without this file having to duplicate
them.

@.claude/AIDA.md

## Project overview

`aida-chat` is a ChatGPT/Claude-style web UI scoped to a single project
repository. It's a Leptos+Axum app (cargo-leptos build) where the user
asks a question in a textbox and a per-session agent answers — picking
from one of two backends:

**`anthropic-api` backend** ([`src/server/backends/anthropic.rs`](src/server/backends/anthropic.rs))
— a small streaming loop against `/v1/messages` with a tightly-scoped
tool surface we own:

- `read_file`, `list_directory`, `grep_repo` — read-only access confined to the
  repo root (see [`src/server/tools/fs.rs`](src/server/tools/fs.rs) for the
  `resolve_within_repo` path-confinement helper).
- `aida_list`, `aida_show`, `aida_search`, `aida_history` — read-only AIDA
  queries via subprocess with explicit-args allowlist.

**`claude-cli` backend** ([`src/server/backends/claude_cli.rs`](src/server/backends/claude_cli.rs))
— shells out to the local `claude` CLI in `--print --output-format
stream-json` mode. Uses your Claude Code subscription (not API credits)
and Claude Code's built-in tool surface. Conversation continuity is via
`--session-id <uuid>` on first turn, `--resume <uuid>` after. The
subprocess inherits `cwd=repo_root` and has `ANTHROPIC_API_KEY` stripped
from its env so it falls back to OAuth/keychain auth.

The active backend is picked at startup via `AIDA_CHAT_BACKEND` (or
auto-detected from what credentials/CLIs are present) and shown as a
small badge in the chat header.

Architecture overview lives in [OVERVIEW.md](OVERVIEW.md).

## Run

```bash
# One-time: export your Anthropic API key (or copy .env.example -> .env and edit)
echo 'ANTHROPIC_API_KEY=sk-ant-...' > .env

# Dev server with hot reload on http://127.0.0.1:8091
cargo leptos serve

# Just the unit tests (path-confinement, aida arg validators)
cargo test --features ssr --lib

# Production build (optimized wasm + bin)
cargo leptos build --release
```

Port `8091` is registered in `~/.ports` as `aida_chat`.

## Environment

| Var                    | Required                       | Default                   | Notes |
|------------------------|--------------------------------|---------------------------|-------|
| `AIDA_CHAT_BACKEND`    | no                             | autodetect                | `anthropic` or `claude-cli`. Autodetect prefers anthropic when `ANTHROPIC_API_KEY` is set, else claude-cli when the CLI is on PATH. |
| `ANTHROPIC_API_KEY`    | only for `anthropic` backend   | —                         | Loaded from `.env` or shell env. Stripped from the subprocess env when running the claude-cli backend. |
| `AIDA_CHAT_MODEL`      | no                             | `claude-sonnet-4-6`       | Anthropic backend only; claude-cli picks its own. |
| `AIDA_CHAT_REPO_ROOT`  | no                             | `$PWD` at startup         | The agent's tools are confined to this directory. |

## Layout

```
src/
  lib.rs              # crate roots + hydrate entry
  main.rs             # axum + leptos boot
  messages.rs         # wire shapes shared between SSR + wasm
  app.rs              # Leptos UI (chat page, streaming, SSE client)
  server/             # cfg(feature = "ssr")
    config.rs         # env -> ServerConfig (Backend enum, autodetect)
    sessions.rs       # SessionStore trait + InMemorySessions
    agent.rs          # Backend dispatch (AgentEvent enum + run_turn)
    api.rs            # /api/info, /api/sessions, /api/sessions/:id/history, /api/chat (SSE)
    backends/
      anthropic.rs    # Direct /v1/messages streaming loop + our scoped tools
      claude_cli.rs   # `claude -p --output-format stream-json` subprocess
    tools/
      fs.rs           # read_file + list_directory + resolve_within_repo
      grep.rs         # grep_repo (ripgrep wrapper)
      aida.rs         # aida_list / aida_show / aida_search / aida_history
style/main.css        # chat layout + dark theme
```

## Design principles

- **Tools, not context dumps.** Don't pre-stuff the system prompt with all
  requirements. Let the agent call `aida_list`/`aida_show` on demand — keeps
  the context lean and stays accurate as requirements change.
- **Confinement at the tool layer.** Every path the model passes in goes
  through `resolve_within_repo`. There's no "trust the model not to escape"
  path. Aida subcommands go through an explicit allowlist; the model can
  never reach a shell.
- **Pluggable session store.** `SessionStore` is a trait. `InMemorySessions`
  is fine for local single-user use; swap in a persistent impl when
  multi-user / auth lands without touching the agent or API.
- **One crate, two builds.** `--features ssr` for the binary; `--features
  hydrate` for the wasm bundle. `cargo leptos` orchestrates both.

## Known limitations

- No auth; anyone who can reach the port can chat (and burn your API quota).
- Sessions live in memory and evict after 1 hour idle. Restarting the server
  drops all conversations.
- No persistent conversation history across restarts.
- Multi-user resource limits and per-user API key isolation are not
  implemented yet.
