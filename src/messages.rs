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

/// `POST /api/sessions/:id/spec` request body (STORY-22).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecRequest {
    #[serde(rename = "type")]
    pub req_type: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /api/sessions/:id/spec` response body (STORY-22).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spec_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// `POST /api/sessions/:id/comment` request body (STORY-21).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentRequest {
    pub spec_id: String,
    pub text: String,
}

/// `POST /api/sessions/:id/comment` response body (STORY-21).
/// On success: `ok=true`, `message=Some(...)`. On failure: `ok=false`,
/// `error=Some(...)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}
