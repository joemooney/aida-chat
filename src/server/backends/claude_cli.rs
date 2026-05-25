// trace:STORY-15 | ai:claude
//
// "claude-cli" backend: keeps **one long-lived `claude` subprocess per
// session**, feeding successive user messages over stdin in
// `--input-format stream-json` and reading responses from stdout.
// Spawning `claude` fresh per turn costs 1-3s of Node startup, hook
// init, MCP server connect, etc. — this avoids that for every turn
// after the first.
//
// Layout:
//
//   LiveProcessRegistry  (process-global, lazy)
//        │   keyed by session UUID
//        ▼
//   one actor task per session
//        │   owns: tokio Child (claude CLI), its stdin/stdout/stderr,
//        │           last-used timestamp, pending turn queue
//        ▼
//   per-turn flow:
//     run_turn -> registry.ensure() -> mpsc::send(TurnRequest) ->
//       actor writes user JSON to stdin -> reads stream-json from
//       stdout until next `result` event -> forwards events to
//       run_turn -> actor loops, ready for the next turn.
//
// Eviction matches the SessionStore idle TTL: a background task runs
// every minute and kills processes that have been idle past TTL. The
// drop of LiveProcessHandle closes the request channel, which
// terminates the actor and kills the child.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::messages::{ChatTurn, Role, ToolCall};
use crate::server::agent::AgentEvent;
use crate::server::config::ServerConfig;
use crate::server::sessions::SessionStore;

const TURN_REQUEST_BUFFER: usize = 8;

// --------------------------------------------------------------------------
// Public entry point
// --------------------------------------------------------------------------

pub async fn run_turn(
    cfg: Arc<ServerConfig>,
    sessions: Arc<dyn SessionStore>,
    session_id: String,
    user_text: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    // Make sure the session exists before doing anything else.
    if sessions.get(&session_id).await.is_none() {
        let _ = tx.send(AgentEvent::Error("no such session".into())).await;
        return;
    }
    if let Err(e) = sessions.append_user(&session_id, &user_text).await {
        let _ = tx.send(AgentEvent::Error(e)).await;
        return;
    }

    // Get the live claude process for this session (spawn lazily).
    let request_tx = match registry().ensure(cfg.clone(), session_id.clone()).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error(e)).await;
            return;
        }
    };

    // Send this turn into the actor and wait for it to complete.
    let (completion_tx, completion_rx) = oneshot::channel();
    let req = TurnRequest {
        user_text,
        event_tx: tx.clone(),
        completion: completion_tx,
    };
    if request_tx.send(req).await.is_err() {
        // Actor died before we could enqueue. Drop the handle so the
        // next turn re-spawns.
        registry().forget(&session_id).await;
        let _ = tx
            .send(AgentEvent::Error("claude live process died".into()))
            .await;
        return;
    }

    let outcome = match completion_rx.await {
        Ok(o) => o,
        Err(_) => {
            registry().forget(&session_id).await;
            let _ = tx
                .send(AgentEvent::Error(
                    "claude live process died mid-turn".into(),
                ))
                .await;
            return;
        }
    };

    match outcome {
        TurnOutcome::Ok {
            final_text,
            tool_calls,
        } => {
            let transcript_turn = ChatTurn {
                role: Role::Assistant,
                text: final_text,
                tool_calls,
            };
            if let Err(e) = sessions
                .commit_assistant_turn(&session_id, vec![], transcript_turn)
                .await
            {
                let _ = tx.send(AgentEvent::Error(e)).await;
                return;
            }
            let _ = tx.send(AgentEvent::Done).await;
        }
        TurnOutcome::Err(msg) => {
            let _ = tx.send(AgentEvent::Error(msg)).await;
        }
    }
}

// --------------------------------------------------------------------------
// Registry — one live process per session
// --------------------------------------------------------------------------

struct LiveProcessRegistry {
    inner: Mutex<HashMap<String, LiveProcessHandle>>,
}

struct LiveProcessHandle {
    request_tx: mpsc::Sender<TurnRequest>,
    last_used: Instant,
}

fn registry() -> &'static Arc<LiveProcessRegistry> {
    static REGISTRY: OnceLock<Arc<LiveProcessRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let reg = Arc::new(LiveProcessRegistry {
            inner: Mutex::new(HashMap::new()),
        });
        // Background reaper: kills processes that haven't been used
        // for `session_ttl`. The TTL is read from cfg passed into
        // ensure() so we don't have a global config; default to 1 hour.
        let r2 = reg.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                iv.tick().await;
                r2.evict_idle(std::time::Duration::from_secs(60 * 60)).await;
            }
        });
        reg
    })
}

impl LiveProcessRegistry {
    async fn ensure(
        &self,
        cfg: Arc<ServerConfig>,
        session_id: String,
    ) -> Result<mpsc::Sender<TurnRequest>, String> {
        let mut map = self.inner.lock().await;
        if let Some(h) = map.get_mut(&session_id) {
            if !h.request_tx.is_closed() {
                h.last_used = Instant::now();
                return Ok(h.request_tx.clone());
            }
            // Stale entry — fall through to respawn.
            map.remove(&session_id);
        }
        let (request_tx, request_rx) = mpsc::channel::<TurnRequest>(TURN_REQUEST_BUFFER);
        spawn_actor(cfg, session_id.clone(), request_rx)?;
        map.insert(
            session_id,
            LiveProcessHandle {
                request_tx: request_tx.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(request_tx)
    }

    async fn forget(&self, session_id: &str) {
        let mut map = self.inner.lock().await;
        map.remove(session_id);
    }

    async fn evict_idle(&self, ttl: std::time::Duration) {
        let now = Instant::now();
        let mut map = self.inner.lock().await;
        map.retain(|_, h| !h.request_tx.is_closed() && now.duration_since(h.last_used) < ttl);
        // The dropped handles close their channels, which terminates
        // the matching actor tasks, which kill the child processes.
    }
}

/// Public hook so the API layer can invalidate the live process when
/// the user explicitly starts a new chat (so a fresh `claude` is spawned).
pub async fn forget_session(session_id: &str) {
    registry().forget(session_id).await;
}

/// Pre-warm the `claude` subprocess for a brand-new session. The user
/// probably won't send their first message for a few seconds, so we
/// can amortize the ~2-3s of Node/hook/MCP startup against that idle
/// time. Errors are swallowed — the registry will surface them on the
/// first real turn instead.
pub async fn prewarm(cfg: Arc<ServerConfig>, session_id: String) {
    let _ = registry().ensure(cfg, session_id).await;
}

// --------------------------------------------------------------------------
// Per-session actor: owns the long-lived `claude` subprocess
// --------------------------------------------------------------------------

struct TurnRequest {
    user_text: String,
    event_tx: mpsc::Sender<AgentEvent>,
    completion: oneshot::Sender<TurnOutcome>,
}

enum TurnOutcome {
    Ok {
        final_text: String,
        tool_calls: Vec<ToolCall>,
    },
    Err(String),
}

fn spawn_actor(
    cfg: Arc<ServerConfig>,
    session_id: String,
    requests: mpsc::Receiver<TurnRequest>,
) -> Result<(), String> {
    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--session-id")
        .arg(&session_id);
    cmd.current_dir(&cfg.repo_root);
    // See backends/claude_cli.rs design notes: strip the API key so
    // claude falls back to OAuth/keychain auth (subscription).
    cmd.env_remove("ANTHROPIC_API_KEY");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn `claude`: {e}. Is Claude Code installed and on PATH?"))?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            let mut reader = BufReader::new(stderr);
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut b = buf.lock().await;
                        b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        // Cap memory growth — only keep the last 8 KB of stderr.
                        if b.len() > 16 * 1024 {
                            let cut = b.len() - 8 * 1024;
                            *b = b.split_off(cut);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    tokio::spawn(actor(
        child, stdin, stdout, stderr_buf, requests, session_id,
    ));
    Ok(())
}

async fn actor(
    mut child: Child,
    mut stdin: ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr_buf: Arc<Mutex<String>>,
    mut requests: mpsc::Receiver<TurnRequest>,
    session_id: String,
) {
    let mut lines = BufReader::new(stdout).lines();

    // Skip the leading `system` init line (and any pre-turn noise) so
    // we're aligned at "ready for first user message" before we accept
    // any turn requests. Errors here are surfaced lazily on the first
    // turn request.
    let prelude_err = wait_for_ready(&mut lines).await;
    if let Some(err) = prelude_err {
        // Drain any pending requests with an error so callers don't hang.
        while let Some(req) = requests.recv().await {
            let _ = req.completion.send(TurnOutcome::Err(err.clone()));
        }
        let _ = child.start_kill();
        return;
    }

    while let Some(req) = requests.recv().await {
        // Push the user message into stdin.
        let payload = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": req.user_text,
            }
        });
        let line = format!("{}\n", payload);
        if let Err(e) = stdin.write_all(line.as_bytes()).await {
            let stderr_text = stderr_buf.lock().await.clone();
            let msg = format!("write to claude stdin: {e}. stderr: {}", stderr_text.trim());
            let _ = req.completion.send(TurnOutcome::Err(msg));
            break; // process is wedged; let the registry respawn next turn
        }
        if let Err(e) = stdin.flush().await {
            let _ = req.completion.send(TurnOutcome::Err(format!("flush: {e}")));
            break;
        }

        // Read stdout until we hit the `result` event for this turn.
        let mut state = StreamState::default();
        let mut died_mid_turn = false;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    handle_line(&line, &mut state, &req.event_tx).await;
                    if state.final_outcome.is_some() {
                        break;
                    }
                }
                Ok(None) => {
                    died_mid_turn = true;
                    break;
                }
                Err(e) => {
                    let _ = req
                        .completion
                        .send(TurnOutcome::Err(format!("read claude stdout: {e}")));
                    let _ = child.start_kill();
                    return;
                }
            }
        }

        if died_mid_turn {
            let stderr_text = stderr_buf.lock().await.clone();
            let trimmed = stderr_text.trim();
            let msg = if trimmed.is_empty() {
                "claude process exited mid-turn".to_string()
            } else {
                format!("claude exited mid-turn: {trimmed}")
            };
            let _ = req.completion.send(TurnOutcome::Err(msg));
            break;
        }

        let outcome = match state.final_outcome.take() {
            Some(Ok(final_text)) => TurnOutcome::Ok {
                final_text,
                tool_calls: std::mem::take(&mut state.tool_calls),
            },
            Some(Err(msg)) => TurnOutcome::Err(msg),
            None => unreachable!(),
        };
        let _ = req.completion.send(outcome);
    }

    // Channel closed (eviction or shutdown). Close stdin so claude
    // exits cleanly, then reap.
    drop(stdin);
    let _ = child.kill().await;
    // Best-effort eviction from the registry (the reaper would also
    // catch it eventually).
    registry().forget(&session_id).await;
}

/// Pull stream-json lines until we see the initial `system/init` event.
/// Returns Some(err) if the process emitted an error or died before
/// becoming ready.
async fn wait_for_ready<R: AsyncBufReadExt + Unpin>(
    lines: &mut tokio::io::Lines<R>,
) -> Option<String> {
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    if ty == "system" {
                        return None;
                    }
                    if ty == "error" {
                        let msg = v
                            .get("message")
                            .and_then(|x| x.as_str())
                            .unwrap_or("claude reported an error before becoming ready");
                        return Some(msg.to_string());
                    }
                }
                // Anything else: keep scanning.
            }
            Ok(None) => return Some("claude stdout closed before init".to_string()),
            Err(e) => return Some(format!("read stdout: {e}")),
        }
    }
}

// --------------------------------------------------------------------------
// stream-json output parser (shared between single-shot and persistent modes)
// --------------------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    tool_calls: Vec<ToolCall>,
    tool_index_by_id: HashMap<String, usize>,
    tool_started_by_id: HashMap<String, Instant>,
    final_outcome: Option<Result<String, String>>,
}

async fn handle_line(line: &str, state: &mut StreamState, tx: &mpsc::Sender<AgentEvent>) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "stream_event" => {
            let event = match v.get("event") {
                Some(e) => e,
                None => return,
            };
            if event.get("type").and_then(|x| x.as_str()) == Some("content_block_delta") {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                if delta.get("type").and_then(|x| x.as_str()) == Some("text_delta") {
                    if let Some(t) = delta.get("text").and_then(|x| x.as_str()) {
                        let _ = tx.send(AgentEvent::TextDelta(t.to_string())).await;
                    }
                }
            }
        }
        "assistant" => {
            let content = match v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                Some(c) => c,
                None => return,
            };
            for block in content {
                if block.get("type").and_then(|x| x.as_str()) == Some("tool_use") {
                    let name = block
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let id = block
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    let call = ToolCall {
                        name,
                        input,
                        output: String::new(),
                        duration_ms: 0,
                        ok: true,
                    };
                    let idx = state.tool_calls.len();
                    state.tool_calls.push(call);
                    if !id.is_empty() {
                        state.tool_index_by_id.insert(id.clone(), idx);
                        state.tool_started_by_id.insert(id, Instant::now());
                    }
                }
            }
        }
        "user" => {
            let content = match v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                Some(c) => c,
                None => return,
            };
            for block in content {
                if block.get("type").and_then(|x| x.as_str()) == Some("tool_result") {
                    let is_error = block
                        .get("is_error")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false);
                    let tool_use_id = block
                        .get("tool_use_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default();
                    if let Some(idx) = state.tool_index_by_id.get(tool_use_id).copied() {
                        if let Some(s) = state.tool_calls.get_mut(idx) {
                            s.ok = !is_error;
                            s.output = tool_result_output(block);
                            s.duration_ms = state
                                .tool_started_by_id
                                .remove(tool_use_id)
                                .map(|started| started.elapsed().as_millis().max(1) as u64)
                                .unwrap_or(1);
                            let _ = tx.send(AgentEvent::ToolCall(s.clone())).await;
                        }
                    }
                }
            }
        }
        "result" => {
            let subtype = v.get("subtype").and_then(|x| x.as_str()).unwrap_or("");
            let is_error = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false);
            let result_text = v
                .get("result")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if subtype == "success" && !is_error {
                state.final_outcome = Some(Ok(result_text));
            } else {
                let msg = if !result_text.is_empty() {
                    result_text
                } else {
                    format!("claude failed (subtype={subtype:?}, is_error={is_error})")
                };
                state.final_outcome = Some(Err(msg));
            }
        }
        _ => {}
    }
}

fn tool_result_output(block: &Value) -> String {
    let Some(content) = block.get("content") else {
        return String::new();
    };
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_parser_emits_completed_tool_call_with_output_and_duration() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut state = StreamState::default();

        handle_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"find_traces","input":{"spec_id":"STORY-14"}}]}}"#,
            &mut state,
            &tx,
        )
        .await;
        assert!(rx.try_recv().is_err(), "tool event should wait for result");

        handle_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"full output","is_error":false}]}}"#,
            &mut state,
            &tx,
        )
        .await;

        let AgentEvent::ToolCall(call) = rx.recv().await.unwrap() else {
            panic!("expected tool event");
        };
        assert_eq!(call.name, "find_traces");
        assert_eq!(call.input["spec_id"], "STORY-14");
        assert_eq!(call.output, "full output");
        assert!(call.duration_ms > 0);
        assert!(call.ok);
    }

    #[test]
    fn tool_result_output_reads_text_blocks() {
        let block = json!({
            "content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ]
        });

        assert_eq!(tool_result_output(&block), "first\nsecond");
    }
}
