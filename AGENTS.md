# AGENTS.md

Guidance for AI coding agents (Codex CLI, MCP-compatible agents, etc.)
working in this repository. The block delimited by HTML comment markers
below is auto-generated from `.claude/AIDA.md` on each
`aida scaffold apply` — edit that file (the markers themselves are
intentionally machine-readable, so leave them in place). Anything
outside the marked block is yours to tailor.

## Project overview

aida-chat

<!-- AIDA-AUTOGEN-BEGIN -->
# AIDA Conventions

This file is the single source of truth for AIDA's coding conventions in
this project. CLAUDE.md imports it via `@.claude/AIDA.md`; AGENTS.md
inlines a copy inside auto-generated delimiters. Edit this file to change
the conventions for both.

## Requirements management

This project tracks requirements with [AIDA](https://github.com/joemooney/aida).
**Do not maintain a separate `REQUIREMENTS.md`** — the requirements DB is
canonical.

Requirements database: distributed git-canonical store at `.aida-store/` (orphan branch `aida-store`, plus a rebuildable SQLite cache at `.aida/cache.db`).

Currently tracking **31** requirement(s).

### Daily commands

```bash
aida list                              # list all requirements (cache-backed)
aida list --status draft               # filter by status
aida show <ID>                         # show details (e.g. `aida show FR-0042`)
aida search "<query>"                  # full-text search
aida add --title "..." --type <type> --status draft
aida edit <ID> --status in-progress
aida edit <ID> --status completed
aida comment add <ID> "implementation note..."
aida rel add --from <ID> --to <ID> --type <Parent|Verifies|References>
aida history                           # what was touched recently (digest)
aida statusline                        # one-line: project · role · queue · cache
```

### Requirement-first development

1. **Before coding:** check whether the work has a SPEC-ID. If not, create one
   (`aida add --type <task|story|bug|...> --status approved --title "..."`).
2. **During coding:** add inline trace comments referencing the SPEC-ID.
3. **Before committing:** mark the requirement `completed` (or `in-progress`
   if work continues), and ensure the commit message references it.

## Inline trace comments

Add a comment near the code that implements (or fixes, or verifies) a
requirement:

```rust
// trace:FR-0042 | ai:claude
fn implement_feature() { /* ... */ }
```

Format: `// trace:<SPEC-ID> | ai:<tool>[:<confidence>]`

- `<SPEC-ID>` — e.g. `FR-0042`, `BUG-1-017`, `TASK-0344`
- `<tool>` — `claude`, `codex`, `copilot`, `human`, `aider`, …
- `<confidence>` — optional: `high` (implied), `med` (40-80% AI), `low` (<40% AI)

## Commit message format

```
[AI:tool] type(scope): description (REQ-ID)
```

Examples:

```
[AI:claude] feat(auth): add login validation (FR-0042)
[AI:claude:med] fix(api): handle null response (BUG-0023)
[AI:antigravity+claude] test(hooks): accept mixed authorship (TASK-509)
chore(deps): update dependencies        # no REQ-ID needed
docs: update README                     # no REQ-ID needed
```

Rules:

- `[AI:tool]` required when commit includes AI-assisted code (any file with a
   `// trace:... | ai:tool` comment changed). Use `[AI:tool1+tool2]` for
   mixed-agent authorship, with optional confidence on the whole commit
   (`[AI:tool1+tool2:med]`).
- `type` required: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`,
   `build`, `ci`, `chore`, `revert`.
- `(scope)` optional — component or area affected.
- `(REQ-ID)` required for `feat`/`fix`; optional for `chore`/`docs`.

Set `AIDA_COMMIT_STRICT=true` (or commit through the `/aida-commit` skill) to
enforce; otherwise the commit-msg hook just warns on non-conforming messages.

## Capture proactively, not reactively

The requirements DB is only valuable when it stays in sync with reality.
Treat `/aida-capture` as a habit, not a safety net:

1. **Spec-first when introducing a new theme.** New command, new field on a
   core model, new skill, new architectural pattern — pause and `aida add`
   *before* the implementation commits. ~2 min cost; saves backfill later.
2. **Don't reuse one EPIC as a catchall.** When the work has drifted from
   what the EPIC was originally about, that's a signal to create a new EPIC,
   not stretch the existing one.
3. **Run `/aida-capture` at natural pauses.** End of focused work, before
   compaction, when stepping away. Five-minute pass that catches missed reqs.
4. **Yellow flag at >5 untracked commits.** Five+ feat/fix commits without a
   matching requirement → offer to capture before continuing.
5. **Trace comments must match reality.** A `// trace:EPIC-N` on code that
   has nothing to do with EPIC-N is misinformation that compounds. If you're
   unsure which spec a piece of work belongs to, that's the signal it needs
   its own.

## Glance at the statusbar

`.claude/settings.json` wires `aida statusline` into Claude Code's status
bar. It shows project · active role · queue depth · cache freshness. If the
role you expect isn't there, you forgot to `aida role enter <name>` before
starting the session.

## When `aida pull` refuses (divergent branches)

`aida pull` is two operations in one: a `git pull` of your code branch
and a `git pull --rebase` of the orphan `aida-store` branch. The two
legs are deliberately asymmetric:

- **Code leg**: `git pull --ff-only` — refuses if the branch has
   diverged from origin. Won't surprise your working tree with an
   auto-rebase.
- **Store leg**: `git pull --rebase` — store conflicts are rare and
   the worktree is AIDA-managed.

When the code leg refuses (or raw `git pull` complains about divergent
branches), the recovery recipe:

```bash
git fetch origin "$(git rev-parse --abbrev-ref HEAD)"
git log --oneline @{u}..HEAD     # what you have that origin doesn't
git log --oneline HEAD..@{u}     # what origin has that you don't
git log --name-only @{u}..HEAD --pretty= | sort -u   # files you touched
git log --name-only HEAD..@{u} --pretty= | sort -u   # files they touched
# No overlap → safe: git pull --rebase
# Overlap   → inspect; rebase + resolve, or git rebase --abort
```

To make raw `git pull` Just Work without per-incident decisions (one-time,
machine-global):

```bash
git config --global pull.rebase true
git config --global rebase.autoStash true
git config --global advice.diverging false
```

Trade-off: silent auto-rebase for fewer manual decisions. `autoStash`
preserves uncommitted changes across the rebase. If you'd rather see the
prompt each time, leave these unset and the recipe above is your fallback.

## Review workflow

`aida review prompt --pr N` (or `--specs FR-1,STORY-2,…`) generates a
markdown review prompt that lifts each linked requirement's `## Acceptance`
section verbatim — paste it into a fresh Claude Code review session, or
write it to a file with `--write`.

- **Install `gh` or `glab` for `--pr` mode.** AIDA shells out to
   [`gh pr view`](https://cli.github.com) / [`glab mr view`](https://gitlab.com/gitlab-org/cli)
   to resolve the PR's base + head refs. Without them, AIDA falls back to
   `base=main` and a local review branch named `pr-N` / `mr-N` — that path
   works when the PR was started via `aida session start --owns PR-N`
   (STORY-61), surprising otherwise.
- **Acceptance sections are the contract.** Write a `## Acceptance`,
   `## Verify`, `## Tests`, `## Test cases`, or `## Verification` section
   in every STORY / BUG description so the review prompt has something
   concrete to lift. `aida doctor convention-check` lints for the gap.
<!-- AIDA-AUTOGEN-END -->


## Codex / MCP-specific notes

### MCP integration

If AIDA is configured as an MCP server (`.mcp.json` is auto-scaffolded),
these tools are available:

| Tool | Purpose |
|------|---------|
| `list_requirements` | List requirements with optional status/type filters |
| `show_requirement` | Show full details by SPEC-ID |
| `search_requirements` | Search by keyword across titles + descriptions |
| `add_requirement` | Create a new requirement |
| `update_requirement` | Update status / priority / owner |
| `add_comment` | Add an implementation note |
| `list_features` | List feature categories |

To configure for Codex CLI:

```bash
codex mcp add aida -- aida mcp-serve
```

### Non-interactive workflows (codex exec)

```bash
# Implement a specific requirement
codex exec "Implement FR-042. Use 'aida show FR-042' to see the details first."

# Sprint standup
codex exec "Run 'aida list --status in-progress' and 'git log --since=yesterday'. Generate a standup report."

# Capture untraced work
codex exec "Review today's git commits. For each, check if trace comments exist. Create requirements for untraced code."
```

### Commit attribution

When committing on behalf of Codex, use the `[AI:codex]` prefix per the
commit format spec in the AIDA-AUTOGEN block above.
