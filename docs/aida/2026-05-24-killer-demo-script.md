# aida-chat Killer-Demo Script — 2026-05-24

Purpose: visible artifact that aida-chat answers substrate-grounded
questions with SPEC-ID + file:line citation that a generic LLM chat
couldn't produce.

## Recording setup

- **Backend: anthropic** — set `AIDA_CHAT_BACKEND=anthropic` (or omit
  and rely on autodetect with a credited `ANTHROPIC_API_KEY` in `.env`).
  Do NOT record on claude-cli backend — it routes through Claude
  Code's built-in tools and bypasses our MCP-grounded `aida_*` surface,
  defeating the differentiation thesis.
- **Server:** `cargo leptos serve`, http://127.0.0.1:8091.
- **Capture:** browser tab + Loom / OBS for the chat UI; tool-call
  badges should be visible in the captured frame.
- **Substrate state:** post-merge of PR #1 + #2 + #3 (so the MCP infra
  + comment-capture loop are both shipped, demonstrating both static
  read-grounding and the write-loop).
- **Rate-limit calibration:** Tier 1 ($20 of credit) gives 30k input
  tokens/min on Sonnet 4.6. Per-session history accumulates so a single
  4-prompt chat will hit 429 by Prompt 2–3. **Start a fresh chat session
  per prompt** (browser refresh, or the UI's new-chat affordance). Each
  prompt's fresh-session response stays well under the budget.

## Prompts (run each, capture aida-chat's answer, contrast with the
counter-factual a generic LLM would produce)

### Prompt 1 — substrate-grounded code lookup

> "Where is EPIC-16 implemented?"

**aida-chat (expected):** invokes `find_traces` → cites the 4 files
that carry `// trace:EPIC-16` comments:

- `src/server/mcp/client.rs` — long-lived stdio JSON-RPC client
- `src/server/mcp/mod.rs` — module root
- `src/server/mcp/protocol.rs` — JSON-RPC + tool-call parsing
- `src/server/tools/traces.rs` — the `find_traces` tool itself

Plus mentions `src/server/backends/anthropic.rs` (system prompt
referencing the SPEC-ID + path:line citation discipline). Offers to
read any specific file.

**Generic LLM (counter-factual):** "EPIC-16 isn't a code identifier I
can resolve directly. Could you provide the file path or describe what
feature you're looking for?"

**What this demonstrates:** the agent can resolve a SPEC-ID to its
implementation site without the human supplying the file path.
Substrate is the index.

### Prompt 2 — substrate-grounded architectural decision lookup

> "What's the architectural decision behind exposing tools to the agent instead of pre-stuffing the system prompt?"

**aida-chat (expected):** invokes `aida_show ADR-7` → cites the
decision body verbatim: scales with repo size, always-fresh state,
cheaper tokens, auditable via tool-call badges. Mentions the
trade-off (more round-trips per turn) and the considered-and-rejected
alternative (Leptos client for aida-server's `/api/v2/chat`). Notes
ADR-7 is Approved and high-priority.

**Generic LLM (counter-factual):** "Most chat applications include
relevant context directly in the system prompt because it reduces
latency. The trade-off would be context length limits and freshness.
Is there a specific concern you're addressing?"

**What this demonstrates:** the agent surfaces the *project's actual
recorded decision* with reasoning intact, not a plausible-sounding
generic answer.

### Prompt 3 — substrate-grounded list lookup

> "List all the requirements in this project."

**aida-chat (expected):** invokes `aida_list` (no filter) → returns the
full set (currently 32) with SPEC-ID + title + status + priority per
line. The agent may then summarize by category if asked.

**Generic LLM (counter-factual):** "I don't have visibility into your
project's status board. Could you point me at your sprint planning or
issue tracker?"

**What this demonstrates:** the agent has read access to live substrate
without the human exporting a snapshot. Substrate is online.

**Known gap:** the original framing of this prompt — "What's still in
progress?" — currently blocks on **BUG-381** in aida core. MCP
`list_requirements` silently returns empty on any status filter
(rejects `"in-progress"`, `"InProgress"`, both; only no-filter works
because the actual stored status text is "In Progress" with a literal
space). Worse than BUG-377-class because successful-but-empty doesn't
trigger aida-chat's CLI fallback, so the user sees a misleading "no
requirements" answer when the substrate has 5+. Re-frame to the
no-filter prompt above until BUG-381 ships.

### Prompt 4 — substrate-grounded failure-mode question (code + spec
cross-reference)

> "What happens if AIDA's MCP server dies mid-session?"

**aida-chat (expected):** invokes `read_file` on
`src/server/mcp/client.rs` (specifically the failure-model doc-comment
lines 18–28) → cites the policy: reader task detects EOF/broken-pipe,
clears the global slot, next `McpClient::global()` call respawns a
fresh subprocess. `Closed`/`Timeout` errors map to `Unavailable` in
`try_mcp` so CLI fallback covers the in-flight call for the legacy
`aida_*` tools. Mentions the `global_respawns_after_child_death` unit
test as the verification. May reference EPIC-16 as the umbrella spec
that drove the design.

**Generic LLM (counter-factual):** "MCP server crashes typically
result in connection errors. Common patterns include
retry-with-backoff or circuit breakers — could you check your
specific MCP client's behavior?"

**What this demonstrates:** the agent answers a *specific code-level
question* by reading the actual file, citing the actual line range
and the actual unit test that verifies it. No hallucination, no
generic advice.

## Acceptance bar for the demo

A demo passes if every answer aida-chat gives:

1. **Cites at least one specific SPEC-ID or file:line range** taken
   from the substrate (not hallucinated).
2. **Could not be produced by a generic LLM chat** without the
   substrate access.
3. **Shows the tool-call badges in the UI** — the audit trail is part
   of the proof.

## Out of scope for this demo

- Capture loops (STORY-21..25 require their own demo segment once
  STORY-21 ships — script that separately).
- Performance / latency (EPIC-26's wedge; demo separately when that
  ships).
- Multi-project (one aida-chat install talks to one project at a
  time per EPIC-16 scope).

## Recording checklist

- [ ] `ANTHROPIC_API_KEY` in `.env` is the *credited* key (BUG: the
      key in repo today returns 400 credit-balance-too-low; operator
      must replace before recording)
- [ ] `cargo leptos serve` running, backend badge in UI reads
      `anthropic-api`
- [ ] PR #1 + #2 + #3 merged to master, working tree clean
- [ ] Browser tab full-screen, recorder targeting that frame
- [ ] One dry-run pass to make sure all 4 prompts return
      substrate-grounded answers (not "the tool is unavailable" — that
      means MCP or credit is broken; debug before recording)
- [ ] Save raw recording + a captioned cut highlighting the SPEC-ID
      citations
