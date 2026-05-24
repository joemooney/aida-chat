// trace:STORY-5 | ai:claude
//
// Shared message shapes used by both the wire API (server <-> browser) and
// the agent's internal conversation state. Kept in a feature-free module so
// the hydrate (wasm) build can use them too.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A turn in the visible chat transcript. Tool calls/results are folded into
/// the assistant turn that produced them rather than surfaced as their own
/// turns, so the history endpoint stays simple for the browser to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: Role,
    pub text: String,
    /// Names of tools the agent invoked while producing this assistant turn.
    /// Empty for user turns.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSummary {
    pub name: String,
    pub input_preview: String,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistory {
    pub session_id: String,
    pub turns: Vec<ChatTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Active backend label, e.g. "anthropic-api" or "claude-cli".
    pub backend: String,
}
