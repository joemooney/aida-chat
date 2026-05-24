// trace:EPIC-16 | ai:claude
//
// JSON-RPC 2.0 wire shapes for the MCP client. We send typed requests
// but parse responses as `serde_json::Value` so a server-side schema
// nudge doesn't make us fail to read a perfectly good reply.
//
// Two small helpers — `parse_tool_call_result` and `parse_resources_list`
// — interpret the MCP-specific response payloads. They live here so
// they can be unit-tested without spinning up a subprocess.

use serde_json::Value;

/// Outcome of `tools/call`: a flattened text payload plus the
/// `isError` boolean. MCP allows multi-block content; we concatenate
/// every `type: "text"` block with newlines.
pub struct ToolCallResult {
    pub text: String,
    pub is_error: bool,
}

pub fn parse_tool_call_result(result: &Value) -> ToolCallResult {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                        b.get("text").and_then(|v| v.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    ToolCallResult { text, is_error }
}

#[derive(Debug, Clone)]
pub struct ResourceMeta {
    pub uri: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

pub fn parse_resources_list(result: &Value) -> Vec<ResourceMeta> {
    result
        .get("resources")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| ResourceMeta {
                    uri: v
                        .get("uri")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    name: v.get("name").and_then(|x| x.as_str()).map(String::from),
                    description: v
                        .get("description")
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    mime_type: v.get("mimeType").and_then(|x| x.as_str()).map(String::from),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Concatenate every `text` block in a `resources/read` reply.
pub fn parse_resource_read(result: &Value) -> String {
    result
        .get("contents")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_text_content_and_error_flag() {
        let r = json!({
            "content": [
                {"type": "text", "text": "Found 3 requirements:"},
                {"type": "text", "text": "- [EPIC-1] foo"}
            ]
        });
        let out = parse_tool_call_result(&r);
        assert_eq!(out.text, "Found 3 requirements:\n- [EPIC-1] foo");
        assert!(!out.is_error);
    }

    #[test]
    fn surfaces_is_error_true() {
        let r = json!({
            "content": [{"type": "text", "text": "bad spec id"}],
            "isError": true
        });
        let out = parse_tool_call_result(&r);
        assert!(out.is_error);
        assert_eq!(out.text, "bad spec id");
    }

    #[test]
    fn parses_resources_list() {
        let r = json!({
            "resources": [
                {"uri": "aida://project/summary", "name": "Project Summary",
                 "description": "Project stats", "mimeType": "text/plain"}
            ]
        });
        let list = parse_resources_list(&r);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uri, "aida://project/summary");
        assert_eq!(list[0].mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn parses_resource_read_text() {
        let r = json!({
            "contents": [
                {"uri": "x", "mimeType": "text/plain", "text": "hello"},
                {"uri": "y", "mimeType": "text/plain", "text": "world"}
            ]
        });
        assert_eq!(parse_resource_read(&r), "hello\nworld");
    }
}
