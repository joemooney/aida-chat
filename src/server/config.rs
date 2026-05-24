// trace:STORY-3 STORY-15 | ai:claude
//
// Server config sourced from env. Centralized so the agent loop and the
// scoped tools share one canonical repo_root and model id.

use std::path::PathBuf;

/// Which backend handles a user turn. Both implement the same internal
/// `AgentEvent` stream (text deltas / tool calls / done / error), so the
/// SSE layer is identical regardless of which one is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Direct streaming calls to api.anthropic.com/v1/messages. Requires
    /// `ANTHROPIC_API_KEY`. Uses the scoped tool surface in
    /// `server::tools` (read_file, grep_repo, aida_*, …).
    Anthropic,
    /// Shells out to the local `claude` CLI in -p / stream-json mode.
    /// Draws on the user's Claude Code subscription instead of API
    /// credits. Uses Claude Code's built-in tools, not ours.
    ClaudeCli,
}

impl Backend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Backend::Anthropic => "anthropic-api",
            Backend::ClaudeCli => "claude-cli",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Which backend handles each user turn.
    pub backend: Backend,
    /// API key for Anthropic. Only required when backend == Anthropic.
    pub anthropic_api_key: Option<String>,
    /// Default model for the anthropic backend. Ignored by claude-cli
    /// (the CLI picks its own model based on the user's Claude Code config).
    pub model: String,
    /// Canonical repo root. All file/grep tools are confined to this.
    /// Also passed as `cwd` to the `claude` subprocess for the CLI backend.
    pub repo_root: PathBuf,
    /// Cap on tool-use iterations per user turn (anthropic backend).
    pub max_tool_iterations: usize,
    /// Maximum tokens per assistant turn (anthropic backend).
    pub max_output_tokens: u32,
    /// Max bytes returned by `read_file` (anthropic backend tool).
    pub max_read_bytes: usize,
    /// Idle TTL before a session is evicted.
    pub session_ttl: std::time::Duration,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|s| !s.trim().is_empty());
        let backend = pick_backend(anthropic_api_key.is_some())?;

        // Validate that the chosen backend actually has what it needs.
        if backend == Backend::Anthropic && anthropic_api_key.is_none() {
            return Err(
                "Backend is anthropic but ANTHROPIC_API_KEY is not set. \
                 Either export ANTHROPIC_API_KEY or set AIDA_CHAT_BACKEND=claude-cli."
                    .to_string(),
            );
        }
        if backend == Backend::ClaudeCli && which("claude").is_none() {
            return Err(
                "Backend is claude-cli but `claude` was not found on PATH. \
                 Install Claude Code or set AIDA_CHAT_BACKEND=anthropic."
                    .to_string(),
            );
        }

        let model = std::env::var("AIDA_CHAT_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-6".to_string());
        let repo_root = match std::env::var("AIDA_CHAT_REPO_ROOT") {
            Ok(s) => PathBuf::from(s),
            Err(_) => std::env::current_dir().map_err(|e| format!("cwd: {e}"))?,
        };
        let repo_root = std::fs::canonicalize(&repo_root)
            .map_err(|e| format!("canonicalize repo_root {}: {e}", repo_root.display()))?;
        Ok(Self {
            backend,
            anthropic_api_key,
            model,
            repo_root,
            max_tool_iterations: 10,
            max_output_tokens: 4096,
            max_read_bytes: 256 * 1024,
            session_ttl: std::time::Duration::from_secs(60 * 60),
        })
    }
}

fn pick_backend(has_anthropic_key: bool) -> Result<Backend, String> {
    match std::env::var("AIDA_CHAT_BACKEND").ok().as_deref() {
        Some("anthropic") | Some("anthropic-api") => Ok(Backend::Anthropic),
        Some("claude-cli") | Some("cli") | Some("claude") => Ok(Backend::ClaudeCli),
        Some(other) => Err(format!(
            "AIDA_CHAT_BACKEND={other:?} is not recognized (use 'anthropic' or 'claude-cli')"
        )),
        None => {
            // Auto-detect: prefer claude-cli if the CLI is available *and*
            // we don't have an API key (most common reason to fall back).
            // If both are available, prefer anthropic-api for back-compat
            // with v0.
            if has_anthropic_key {
                Ok(Backend::Anthropic)
            } else if which("claude").is_some() {
                Ok(Backend::ClaudeCli)
            } else {
                Err(
                    "Neither ANTHROPIC_API_KEY nor a `claude` CLI is available. \
                     Set one of them (or AIDA_CHAT_BACKEND=…) and try again."
                        .to_string(),
                )
            }
        }
    }
}

fn which(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(prog);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
