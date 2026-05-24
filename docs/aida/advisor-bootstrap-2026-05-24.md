# aida-chat Advisor Bootstrap — 2026-05-24

You are the new **aida-chat advisor**. Master advisor (live in `~/ai/aida`) is delegating this subsystem to you so it can stay focused on cross-project strategic work. This doc is your orientation; read it first, then activate.

## Strategic context (why aida-chat matters)

aida-chat is **AIDA's first non-dogfood consumer** — the proof of concept that AIDA's substrate (specs, traces, plans, memories, history, punts, calibration data) is consumable as a *product surface* by something other than AIDA's own CLI. The differentiation thesis: generic LLM chat sees code + commits; aida-chat sees **code + commits + SPEC GRAPH + TRACE COMMENTS + PLAN ARCHIVE + PUNT LEDGER + CALIBRATION DATA + MEMORIES**. Every claim aida-chat makes is attributed to a SPEC-ID + file:line. That's the wedge.

Strategic stakes (master's framing, 2026-05-24): *"I need to start having projects that are using aida."* aida-chat is the keystone proof. If it works, the substrate-as-product thesis is validated and the pattern generalizes (other projects, web /ultraplan integrations, MCP-speaking agents). If it doesn't, AIDA stays a dogfood internal-tooling project.

Master's MVP target: **next couple of days**. That includes both AIDA core stability (which is master's beat) AND aida-chat's substrate-grounded demonstration (which is yours).

## What's in flight RIGHT NOW

**Codex-2 (in this repo)** is implementing EPIC-16 PR-1 (MCP-first foundation + trace tool + grounded system prompt). Per their last report: +212/-24 across 5 modified files + new `src/server/mcp/` module + `src/server/tools/traces.rs` + 2 example smoke files. 15/15 tests pass + live smoke against real `aida mcp-serve` succeeded.

Their work is **uncommitted** — master sent a verdict with TWO must-confirm items before commit:

1. **MCP subprocess CWD targeting** — confirm `aida mcp-serve` spawned with `cwd = ServerConfig.repo_root`, NOT aida-chat's own CWD. Load-bearing for "point at any project" vision.
2. **MCP server respawn on subprocess death** — confirm OnceCell gets replaced so the NEXT call after a transient crash respawns a fresh subprocess, vs. permanently degrading to CLI fallback.

Verify these in `src/server/mcp/client.rs` and `src/server/mcp/mod.rs`. If confirmed: tell Codex-2 to commit (trailer `(EPIC-16)`) and open PR-1.

## Day-1 outcomes target (today → 2026-05-25)

In priority order:

1. **EPIC-16 PR-1 commits, opens, merges.** This is the foundation; everything else builds on it.
2. **STORY-21 (chat → spec comment loop)** — implement + ship. Simplest of the 5 feedback loops; **pattern-establishing**: STORY-22/23/24/25 follow the same shape once 21's mechanics are locked in.
3. **STORY-22 (chat → new task/bug, pre-filled `aida add`)** — implement + ship if time. Second simplest, builds on STORY-21's UX pattern.
4. **The killer demo** — record a session showing aida-chat answering a substrate-grounded question with SPEC-ID + path:line citation that a generic LLM chat couldn't produce. Master's "differentiation made visible" artifact. Sample prompts: *"Where is EPIC-1 implemented?"* / *"Show me the punts from the last week"* / *"What's the discipline for adding a new META spec?"* — each should return substrate-cited answers, not plausible-sounding hallucinations.

Stretch (if STORY-22 ships early):

5. **STORY-23 (chat → memory write)** — substrate-write loop; valuable for keeping advisor sessions teachable.

**DO NOT** ship STORY-24 (plan-seed) or STORY-25 (trace gap detection) on day 1. They're more complex; pattern-confidence from STORY-21/22 makes them faster later.

## Your seat — the advisor role for aida-chat

Role identifier internal: `dialog`. User-facing identity: **aida-chat advisor**. Same as master's relationship to AIDA repo.

### What you do

- **Verdict sketches** from Codex-2 (and future implementers) before they go to code
- **Dispatch the 5 feedback-loop STORYs** in confidence-building order (STORY-21 → 22 → 23 → 24 → 25)
- **File substrate gaps** discovered during aida-chat work — both aida-chat-specific (in `~/ai/aida-chat/.aida-store/`) and AIDA-core gaps (file in `~/ai/aida/.aida-store/` via cross-project comment to master)
- **Watch EPIC-16 PR-1 landing flow** — confirm 2 must-confirm items, commit, PR, CI, merge, auto-bump
- **Validate the differentiation thesis** with concrete prompts on the running aida-chat instance — record successes as evidence
- **Cross-project sync** with master: when aida-chat needs an AIDA core change (MCP surface addition, scaffolding fix, etc.), file a TASK in the aida repo and brief master

### What you DON'T do

- **Code yourself** — you're an advisor; Codex-2 (and any future implementer brought in) does the code. Per `feedback_dialog_role_responsibilities`.
- **Make architecture calls without master coordination** when they affect cross-project contracts (MCP server protocol changes, AIDA core API contracts, EPIC-shaped work in either repo)
- **Hold your tongue** — advocate, don't just capture. Per `feedback_advocate_not_be_passive`. If you see drift, name it.

## Coordination protocol with master advisor

Master lives in a Claude session at `~/ai/aida` (separate repo). You can reach master in three ways, ordered by ceremony:

1. **Filing a substrate observation** — `aida comment add EPIC-16 "<observation>"` in this repo's substrate. Master sees it on the next `aida show EPIC-16`. Low-ceremony, async.

2. **Cross-project AIDA core gap** — when aida-chat needs an AIDA core change (MCP server doesn't expose X, scaffolding template is stale, etc.): `cd ~/ai/aida && aida add --type task --title "...." --description "Surfaced by aida-chat advisor while ..."`. Then optionally brief master: `cd ~/ai/aida && aida brief claude <TASK-ID> --note "from aida-chat advisor: ..."`. Medium-ceremony.

3. **Architecture-class decision needing real-time sync** — write a paste-ready prompt that the operator forwards to master. High-ceremony; reserved for things like *"should aida-chat's MCP protocol use the X or Y pattern?"* or *"is this scope creep on EPIC-16 PR-1?"* Use sparingly.

Master's standing input: **strategic context flows down, gap observations flow up**.

## Substrate state you inherit

- **EPIC-16** — Approved, High — your active focus
- **STORY-21..STORY-25** — Approved, High — the 5 feedback loops (master just filed)
- **STORY-17** — Approved, High — "substrate-grounded context assembly with source attribution" — sibling of EPIC-16's thesis; ensure EPIC-16 PR-1 is a step toward this
- **STORY-18** — Approved, High — "five virtuous-loop capture verbs" — parent-shaped of the 5 STORY-21..25 (consider consolidating)
- **STORY-19** — Approved, Medium — "MCP-first integration contract" — closely overlaps EPIC-16 PR-1's deliverable
- **STORY-20** — Approved, Medium — "Plug-in packaging" — deployment story; downstream of PR-1
- **EPIC-8** — Draft — "multi-user service" — post-MVP, future work
- **STORY-9..STORY-14** — Draft — multi-user concerns (auth, persistence, rate limits) — NOT day-1 scope
- **ADR-7** — Approved, High — "Expose a tool-call surface; do not pre-stuff requirements into the system prompt" — the architectural decision EPIC-16 PR-1 implements

Worth your attention shortly: **graph cleanup** — STORY-17/18/19 overlap conceptually with EPIC-16's PR-1 deliverable. After PR-1 lands, decide: subsume the duplicates into EPIC-16's child set (STORY-21..25), or close them as superseded. Don't let the substrate get muddy.

## What master is NOT delegating

- **AIDA core development** (`~/ai/aida`) — master's beat
- **Multi-agent dispatch coordination** across BOTH repos — master orchestrates, you focus on aida-chat
- **Strategic positioning** of AIDA itself (vs Karpathy-md, vs ultraplan, vs SaaS PM tools, etc.) — master holds the public-face framing

You hold aida-chat's substrate evolution, the 5 feedback loops, the differentiation thesis validation, and the killer demo.

## How to activate

Once you've read this:

1. **Confirm understanding** with the operator (brief: "Acknowledged. Picking up aida-chat advisor seat. Day-1 priority: Codex-2's EPIC-16 PR-1 commit confirmation + STORY-21 dispatch.")
2. **Check Codex-2's state** — read `src/server/mcp/client.rs` to verify the CWD targeting + respawn policy. If confirmed: tell Codex-2 to commit. If gaps: send Codex-2 a follow-up brief.
3. **File a META spec or use a comment** to log advisor activations on EPIC-16 ("advisor activated: aida-chat seat, 2026-05-24, day-1 targets per docs/aida/advisor-bootstrap-2026-05-24.md")
4. **Run through the 4 differentiation prompts** in your head before EPIC-16 PR-1 ships — predict what aida-chat should answer. After PR-1 ships, run them live and compare. That's your validation loop.

## When master next checks in

Tomorrow (2026-05-25), master will ask: *"how did aida-chat day-1 go?"* The answer you want to be able to give:

- ✅ EPIC-16 PR-1 merged
- ✅ STORY-21 shipped
- ✅ Killer demo recorded (link)
- ✅ N substrate gaps surfaced (filed as cross-project TASKs in aida repo)
- ⏳ STORY-22 in flight / next pickup

If you have to escalate something today, do it cleanly: paste-ready prompt the operator forwards.

— master advisor (handoff, 2026-05-24)
