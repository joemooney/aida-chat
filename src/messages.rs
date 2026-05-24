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
    /// Full tool audit trail for this assistant turn. Empty for user turns.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

// trace:STORY-14 | ai:codex
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    pub duration_ms: u64,
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

/// `POST /api/sessions/:id/memory` request body (STORY-23). Same
/// idiom as `SpecRequest`: `type` lands on the wire as `type`; the
/// `r#` is just Rust-keyword escaping. Field order mirrors the brief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRequest {
    pub name: String,
    pub description: String,
    pub r#type: String,
    pub body: String,
}

/// `POST /api/sessions/:id/memory` response body (STORY-23). On success:
/// `ok=true`, `path=Some("/some/dir/foo.md")`, optional `message`. On
/// failure: `ok=false`, `error=Some(...)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<String>,
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

    #[test]
    fn memory_request_serializes_type_field_unprefixed() {
        let req = MemoryRequest {
            name: "remember-this".into(),
            description: "A useful correction".into(),
            r#type: "feedback".into(),
            body: "Use the local substrate.".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""type":"feedback""#), "json was {json}");
        let back: MemoryRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.r#type, "feedback");
    }

    #[test]
    fn memory_response_success_skips_error_field() {
        let r = MemoryResponse {
            ok: true,
            path: Some("/tmp/memory/foo.md".into()),
            message: Some("Memory saved".into()),
            error: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""path":"/tmp/memory/foo.md""#));
        assert!(!s.contains(r#""error""#), "error should be skipped: {s}");
    }

    #[test]
    fn tool_call_roundtrip_preserves_full_shape() {
        let call = ToolCall {
            name: "find_traces".into(),
            input: serde_json::json!({"spec_id": "EPIC-16"}),
            output: "trace hits".into(),
            duration_ms: 42,
            ok: true,
        };

        let json = serde_json::to_string(&call).unwrap();
        assert!(!json.contains("input_preview"));
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(back, call);
    }

    #[test]
    fn chat_turn_roundtrip_preserves_multiple_tool_calls() {
        let turn = ChatTurn {
            role: Role::Assistant,
            text: "done".into(),
            tool_calls: vec![
                ToolCall {
                    name: "aida_show".into(),
                    input: serde_json::json!({"spec_id": "STORY-14"}),
                    output: "story body".into(),
                    duration_ms: 7,
                    ok: true,
                },
                ToolCall {
                    name: "grep_repo".into(),
                    input: serde_json::json!({"pattern": "ToolCall"}),
                    output: "error: bad input".into(),
                    duration_ms: 3,
                    ok: false,
                },
            ],
        };

        let json = serde_json::to_string(&turn).unwrap();
        let back: ChatTurn = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_calls.len(), 2);
        assert_eq!(back.tool_calls[0].input["spec_id"], "STORY-14");
        assert_eq!(back.tool_calls[1].output, "error: bad input");
        assert!(!json.contains("input_preview"));
    }
}

#[cfg(test)]
mod memory_contract_tests {
    // trace:STORY-23 | ai:claude
    use super::*;

    #[test]
    fn memory_request_serializes_type_field_unprefixed() {
        let req = MemoryRequest {
            name: "login-edge-case".into(),
            description: "stash this corrective pattern".into(),
            r#type: "feedback".into(),
            body: "When the model proposes X, prefer Y because…".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        // Verify the on-the-wire field name is `type`, not `r#type`.
        assert!(json.contains(r#""type":"feedback""#), "json was {json}");
        assert!(json.contains(r#""name":"login-edge-case""#));
        assert!(json.contains(r#""description":"stash this corrective pattern""#));
        assert!(json.contains(r#""body":"When the model proposes X, prefer Y because…""#));
    }

    #[test]
    fn memory_request_roundtrip() {
        let req = MemoryRequest {
            name: "use-explicit-paths".into(),
            description: "principle".into(),
            r#type: "user".into(),
            body: "lorem".into(),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: MemoryRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "use-explicit-paths");
        assert_eq!(back.description, "principle");
        assert_eq!(back.r#type, "user");
        assert_eq!(back.body, "lorem");
    }

    #[test]
    fn memory_response_success_skips_error_field() {
        let r = MemoryResponse {
            ok: true,
            path: Some("/home/joe/.claude/projects/aida-chat/memory/login-edge-case.md".into()),
            message: Some("written".into()),
            error: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""ok":true"#));
        assert!(s.contains(r#""path":"/home/joe/.claude"#));
        assert!(!s.contains(r#""error""#), "error should be skipped: {s}");
    }

    #[test]
    fn memory_response_error_path() {
        let json = r#"{"ok":false,"error":"slug already exists"}"#;
        let back: MemoryResponse = serde_json::from_str(json).unwrap();
        assert!(!back.ok);
        assert_eq!(back.error.as_deref(), Some("slug already exists"));
        assert!(back.path.is_none());
        assert!(back.message.is_none());
    }

    #[test]
    fn memory_response_minimal_success_still_parses() {
        // Backend may legitimately return only `ok` and `path`.
        let json = r#"{"ok":true,"path":"/tmp/foo.md"}"#;
        let back: MemoryResponse = serde_json::from_str(json).unwrap();
        assert!(back.ok);
        assert_eq!(back.path.as_deref(), Some("/tmp/foo.md"));
        assert!(back.message.is_none());
    }
}
