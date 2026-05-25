// trace:STORY-3 STORY-15 | ai:claude
//
// "anthropic-api" backend: streaming agent loop against the Anthropic
// Messages API. See backends/claude_cli.rs for the alternative.

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::messages::{ChatTurn, Role, ToolCallSummary};
use crate::server::agent::AgentEvent;
use crate::server::config::ServerConfig;
use crate::server::sessions::{AgentMessage, AssistantBlock, SessionStore, ToolResult};
use crate::server::tools::{self, all_tool_specs, preview_input};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

pub async fn run_turn(
    cfg: Arc<ServerConfig>,
    sessions: Arc<dyn SessionStore>,
    session_id: String,
    user_text: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    if let Err(e) = sessions.append_user(&session_id, &user_text).await {
        let _ = tx.send(AgentEvent::Error(e)).await;
        return;
    }
    let session = match sessions.get(&session_id).await {
        Some(s) => s,
        None => {
            let _ = tx.send(AgentEvent::Error("session disappeared".into())).await;
            return;
        }
    };

    let mut working: Vec<AgentMessage> = session.history.clone();
    let mut new_entries: Vec<AgentMessage> = vec![]; // suffix appended this turn
    let mut tool_summaries: Vec<ToolCallSummary> = vec![];
    let mut final_text = String::new();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("reqwest client");

    let api_key = cfg
        .anthropic_api_key
        .as_deref()
        .expect("anthropic backend reached without an api key (config bug)");

    for iteration in 0..cfg.max_tool_iterations {
        let body = build_request_body(&cfg, &working);
        let stream_text_tx = tx.clone();
        let round = match anthropic_round(
            &client,
            api_key,
            body,
            stream_text_tx,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(AgentEvent::Error(e)).await;
                return;
            }
        };

        // Persist this round into both the working history (so a follow-up
        // call sees it) and the per-turn appendix (so we can commit it
        // back to the session store at the end).
        let blocks = round.assistant_blocks.clone();
        working.push(AgentMessage::Assistant {
            content: blocks.clone(),
        });
        new_entries.push(AgentMessage::Assistant { content: blocks.clone() });

        // Track text for the final visible transcript turn. Each round
        // overwrites; the *last* round's text is what the user sees.
        final_text = round.text;

        match round.stop_reason.as_deref() {
            Some("end_turn") | Some("stop_sequence") => break,
            Some("tool_use") => {
                let tool_uses: Vec<_> = round
                    .assistant_blocks
                    .into_iter()
                    .filter_map(|b| match b {
                        AssistantBlock::ToolUse { id, name, input } => Some((id, name, input)),
                        _ => None,
                    })
                    .collect();
                if tool_uses.is_empty() {
                    // Model said tool_use but produced no tool_use blocks.
                    // Treat as end of turn.
                    break;
                }
                let mut results = vec![];
                for (id, name, input) in tool_uses {
                    let preview = preview_input(&input);
                    let (output, ok) = match tools::dispatch(&cfg, &name, &input).await {
                        Ok(s) => (s, true),
                        Err(e) => (format!("error: {e}"), false),
                    };
                    let summary = ToolCallSummary {
                        name: name.clone(),
                        input_preview: preview,
                        ok,
                        chart: tools::charts::extract_chart_artifact(&output),
                    };
                    tool_summaries.push(summary.clone());
                    let _ = tx.send(AgentEvent::ToolCall(summary)).await;
                    results.push(ToolResult {
                        tool_use_id: id,
                        content: output,
                        is_error: !ok,
                    });
                }
                working.push(AgentMessage::ToolResults {
                    results: results.clone(),
                });
                new_entries.push(AgentMessage::ToolResults { results });
            }
            Some("max_tokens") => {
                final_text.push_str(
                    "\n\n[truncated: max_tokens reached]",
                );
                break;
            }
            other => {
                let _ = tx
                    .send(AgentEvent::Error(format!(
                        "unexpected stop_reason: {other:?}"
                    )))
                    .await;
                return;
            }
        }

        if iteration + 1 == cfg.max_tool_iterations {
            let _ = tx
                .send(AgentEvent::Error(format!(
                    "tool iteration cap ({}) reached without a final answer",
                    cfg.max_tool_iterations
                )))
                .await;
            return;
        }
    }

    let transcript_turn = ChatTurn {
        role: Role::Assistant,
        text: final_text,
        tool_calls: tool_summaries,
    };
    if let Err(e) = sessions
        .commit_assistant_turn(&session_id, new_entries, transcript_turn)
        .await
    {
        let _ = tx.send(AgentEvent::Error(e)).await;
        return;
    }
    let _ = tx.send(AgentEvent::Done).await;
}

// --------------------------------------------------------------------------
// One Anthropic round
// --------------------------------------------------------------------------

struct AnthropicRound {
    assistant_blocks: Vec<AssistantBlock>,
    stop_reason: Option<String>,
    /// The text content concatenated across all text blocks in this round.
    text: String,
}

async fn anthropic_round(
    client: &reqwest::Client,
    api_key: &str,
    body: Value,
    text_tx: mpsc::Sender<AgentEvent>,
) -> Result<AnthropicRound, String> {
    let resp = client
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_API_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("anthropic request: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("anthropic returned {status}: {body}"));
    }

    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut state = StreamState::default();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("anthropic stream: {e}"))?;
        parser.push(&chunk);
        while let Some(event) = parser.next_event() {
            if let Err(e) = handle_event(&event, &mut state, &text_tx).await {
                return Err(e);
            }
        }
    }
    parser.flush();
    while let Some(event) = parser.next_event() {
        handle_event(&event, &mut state, &text_tx).await?;
    }

    let assistant_blocks = state
        .blocks
        .into_iter()
        .filter_map(|b| b.finalize())
        .collect::<Vec<_>>();
    let text = assistant_blocks
        .iter()
        .filter_map(|b| match b {
            AssistantBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    Ok(AnthropicRound {
        assistant_blocks,
        stop_reason: state.stop_reason,
        text,
    })
}

fn build_request_body(cfg: &ServerConfig, history: &[AgentMessage]) -> Value {
    let tools_json: Vec<Value> = all_tool_specs()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    let messages: Vec<Value> = history.iter().map(history_to_message_json).collect();

    json!({
        "model": cfg.model,
        "max_tokens": cfg.max_output_tokens,
        "system": system_prompt(cfg),
        "tools": tools_json,
        "messages": messages,
        "stream": true,
    })
}

fn history_to_message_json(m: &AgentMessage) -> Value {
    match m {
        AgentMessage::User { text } => json!({
            "role": "user",
            "content": [ { "type": "text", "text": text } ],
        }),
        AgentMessage::Assistant { content } => {
            let content_json: Vec<Value> = content
                .iter()
                .map(|b| match b {
                    AssistantBlock::Text(t) => json!({ "type": "text", "text": t }),
                    AssistantBlock::ToolUse { id, name, input } => json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }),
                })
                .collect();
            json!({ "role": "assistant", "content": content_json })
        }
        AgentMessage::ToolResults { results } => {
            let content_json: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": r.tool_use_id,
                        "content": r.content,
                        "is_error": r.is_error,
                    })
                })
                .collect();
            json!({ "role": "user", "content": content_json })
        }
    }
}

fn system_prompt(cfg: &ServerConfig) -> String {
    format!(
        "You are AIDA Chat, an assistant scoped to a single project repository.

You can answer questions about:
  - The contents of files in this repo (use list_directory, read_file, grep_repo).
  - The project's tracked requirements stored in AIDA (use aida_list, aida_show, aida_search, aida_history).
  - The mapping from SPEC-IDs to code (use find_traces — see below).
  - Substrate artefacts beyond requirements (use aida_resource — see below).

Attribution is the differentiator:
  - Every factual claim about this project must be attributed: cite the SPEC-ID for requirements claims and `path:line` for code claims. Plain narration without a SPEC-ID or path:line should be the exception, not the default.
  - To map a SPEC-ID to where it is implemented, call find_traces before reaching for grep_repo — it is the faster and more accurate path for that question.
  - For plan archives, project summary, requirements tree, or anything you can't get from aida_list/aida_show, call aida_resource with action='list' first to see what is exposed, then action='read' on a specific URI.

Guidelines:
  - Prefer calling tools to get current information rather than relying on assumptions.
  - When the user asks 'what epics / stories / bugs are open?' or anything about requirements, prefer the aida_* tools.
  - When the user asks for agile metrics, status distribution, sprint burndown/burn-up, velocity, or feature progress, use chart_status, chart_sprint, or chart_feature so the answer includes an inline chart artifact.
  - When the user clearly asks to save a substantive discussion summary, design note, or decision to a SPEC, use aida_comment_add sparingly.
  - When the user clearly asks to file work or a defect as a new SPEC, use aida_add sparingly.
  - When the user asks about code or documentation contents, use grep_repo to locate things, then read_file to inspect specific files.
  - Be concise. Don't paste large file contents back to the user unless they ask.

The repo root is: {}",
        cfg.repo_root.display()
    )
}

// --------------------------------------------------------------------------
// SSE stream parsing (Anthropic Messages stream protocol)
// --------------------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    blocks: Vec<PartialBlock>,
    stop_reason: Option<String>,
}

enum PartialBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

impl PartialBlock {
    fn finalize(self) -> Option<AssistantBlock> {
        match self {
            PartialBlock::Text { text } if !text.is_empty() => Some(AssistantBlock::Text(text)),
            PartialBlock::Text { .. } => None,
            PartialBlock::ToolUse {
                id,
                name,
                input_json,
            } => {
                let input: Value = if input_json.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&input_json).unwrap_or_else(|_| json!({}))
                };
                Some(AssistantBlock::ToolUse { id, name, input })
            }
        }
    }
}

async fn handle_event(
    ev: &SseEvent,
    state: &mut StreamState,
    text_tx: &mpsc::Sender<AgentEvent>,
) -> Result<(), String> {
    let event_name = ev.event.as_deref().unwrap_or("");
    let data = match serde_json::from_str::<Value>(&ev.data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // tolerate noise / heartbeats
    };
    match event_name {
        "content_block_start" => {
            let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let block_type = data
                .get("content_block")
                .and_then(|b| b.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let block = match block_type {
                "text" => PartialBlock::Text {
                    text: String::new(),
                },
                "tool_use" => {
                    let id = data
                        .get("content_block")
                        .and_then(|b| b.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = data
                        .get("content_block")
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    PartialBlock::ToolUse {
                        id,
                        name,
                        input_json: String::new(),
                    }
                }
                _ => PartialBlock::Text {
                    text: String::new(),
                },
            };
            ensure_index(&mut state.blocks, index);
            state.blocks[index] = block;
        }
        "content_block_delta" => {
            let index = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let delta = data.get("delta").cloned().unwrap_or(Value::Null);
            let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if index >= state.blocks.len() {
                return Ok(());
            }
            match (&mut state.blocks[index], delta_type) {
                (PartialBlock::Text { text }, "text_delta") => {
                    if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                        let _ = text_tx.send(AgentEvent::TextDelta(t.to_string())).await;
                    }
                }
                (PartialBlock::ToolUse { input_json, .. }, "input_json_delta") => {
                    if let Some(t) = delta.get("partial_json").and_then(|v| v.as_str()) {
                        input_json.push_str(t);
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            // nothing to do — finalize happens after stream ends.
        }
        "message_delta" => {
            if let Some(sr) = data
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
            {
                state.stop_reason = Some(sr.to_string());
            }
        }
        "message_start" | "message_stop" | "ping" => {}
        "error" => {
            let msg = data
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("anthropic stream error");
            return Err(msg.to_string());
        }
        _ => {}
    }
    Ok(())
}

fn ensure_index(blocks: &mut Vec<PartialBlock>, index: usize) {
    while blocks.len() <= index {
        blocks.push(PartialBlock::Text {
            text: String::new(),
        });
    }
}

// --------------------------------------------------------------------------
// Minimal SSE parser for the bytes we get back from Anthropic.
//
// Lines:   "event: <name>"   sets the event for the next blank-line-terminated block
//          "data: <chunk>"   appended (data fields are concatenated with newlines per spec, but
//                            Anthropic puts the full JSON on one data: line, so simple append works)
//          ""                terminates the current event
// --------------------------------------------------------------------------

#[derive(Default)]
struct SseParser {
    buffer: String,
    pending_event: Option<String>,
    pending_data: String,
    ready: std::collections::VecDeque<SseEvent>,
}

struct SseEvent {
    event: Option<String>,
    data: String,
}

impl SseParser {
    fn push(&mut self, chunk: &Bytes) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            self.buffer.push_str(s);
            self.drain_lines();
        }
    }

    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
            self.drain_lines();
        }
    }

    fn drain_lines(&mut self) {
        loop {
            let Some(nl) = self.buffer.find('\n') else {
                break;
            };
            let line = self.buffer[..nl].to_string();
            self.buffer.drain(..=nl);
            let trimmed = line.trim_end_matches('\r');
            if trimmed.is_empty() {
                if !self.pending_data.is_empty() || self.pending_event.is_some() {
                    self.ready.push_back(SseEvent {
                        event: self.pending_event.take(),
                        data: std::mem::take(&mut self.pending_data),
                    });
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("event:") {
                self.pending_event = Some(rest.trim().to_string());
            } else if let Some(rest) = trimmed.strip_prefix("data:") {
                if !self.pending_data.is_empty() {
                    self.pending_data.push('\n');
                }
                self.pending_data.push_str(rest.trim_start());
            }
            // Ignore unknown field types ("id:", "retry:", etc).
        }
    }

    fn next_event(&mut self) -> Option<SseEvent> {
        self.ready.pop_front()
    }
}
