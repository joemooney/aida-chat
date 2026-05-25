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

/// `POST /api/sessions/:id/spec` request body (STORY-22). `type` lands
/// on the wire as `type` (the `r#` is just Rust-keyword escaping).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecRequest {
    pub r#type: String,
    pub title: String,
    pub description: String,
}

/// `POST /api/sessions/:id/spec` response body (STORY-22). On success:
/// `ok=true`, `spec_id=Some("BUG-378")`, optional `message`. On failure:
/// `ok=false`, `error=Some(...)`.
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

// trace:STORY-24 | ai:agy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UltraplanRequest {
    pub spec_id: String,
}

// trace:STORY-24 | ai:agy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UltraplanResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod spec_contract_tests {
    // trace:STORY-22 | ai:claude
    use super::*;

    #[test]
    fn spec_request_serializes_type_field_unprefixed() {
        let req = SpecRequest {
            r#type: "bug".into(),
            title: "Login fails on retry".into(),
            description: "Steps to reproduce…".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        // Verify the on-the-wire field name is `type`, not `r#type`.
        assert!(json.contains(r#""type":"bug""#), "json was {json}");
        assert!(json.contains(r#""title":"Login fails on retry""#));
        assert!(json.contains(r#""description":"Steps to reproduce…""#));
    }

    #[test]
    fn spec_request_roundtrip() {
        let req = SpecRequest {
            r#type: "story".into(),
            title: "Add OAuth".into(),
            description: "longer body".into(),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: SpecRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.r#type, "story");
        assert_eq!(back.title, "Add OAuth");
        assert_eq!(back.description, "longer body");
    }

    #[test]
    fn spec_response_success_skips_error_field() {
        let r = SpecResponse {
            ok: true,
            spec_id: Some("BUG-378".into()),
            message: Some("ok".into()),
            error: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""ok":true"#));
        assert!(s.contains(r#""spec_id":"BUG-378""#));
        assert!(!s.contains(r#""error""#), "error should be skipped: {s}");
    }

    #[test]
    fn spec_response_error_path() {
        let json = r#"{"ok":false,"error":"validation failed"}"#;
        let back: SpecResponse = serde_json::from_str(json).unwrap();
        assert!(!back.ok);
        assert_eq!(back.error.as_deref(), Some("validation failed"));
        assert!(back.spec_id.is_none());
        assert!(back.message.is_none());
    }

    #[test]
    fn spec_response_minimal_success_still_parses() {
        // Backend may legitimately omit message on success.
        let json = r#"{"ok":true,"spec_id":"TASK-1"}"#;
        let back: SpecResponse = serde_json::from_str(json).unwrap();
        assert!(back.ok);
        assert_eq!(back.spec_id.as_deref(), Some("TASK-1"));
    }
}
