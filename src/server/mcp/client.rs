// trace:EPIC-16 | ai:claude
//
// Long-lived stdio JSON-RPC client for `aida mcp-serve` (or whatever
// `AIDA_CHAT_MCP_COMMAND` points at). One subprocess per aida-chat
// process — not per session, not per call — spawned the first time
// `McpClient::global()` is awaited.
//
// Layout:
//
//   McpClient (process-global, swappable slot)
//     ├── writer: Mutex<ChildStdin>          serializes outbound JSON
//     ├── inbox:  Mutex<HashMap<u64, oneshot>>
//     │            request-id → waiting caller
//     └── reader task (spawned in spawn_and_init)
//          reads stdout line by line, parses JSON-RPC, looks up the id
//          in inbox, sends Ok/Err to the waiting oneshot.
//
// Failure model:
//
//   - Spawn or `initialize` handshake fails → `McpError::Unavailable`.
//     The global slot caches that failure for a short backoff window so
//     a broken install does not spawn-storm, then retries.
//   - Server dies after init → reader task exits, drops every pending
//     oneshot Sender → waiting `request()` calls resolve with
//     `McpError::Closed`, and clears the global slot. New writes that
//     hit a broken pipe also clear the slot and surface as `Closed`.
//     The singleton respawns on the next `global()` call; CLI fallback
//     covers the in-flight call for tools with a CLI equivalent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

pub use super::protocol::ResourceMeta;
use super::protocol::{parse_resource_read, parse_resources_list, parse_tool_call_result};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
const UNAVAILABLE_BACKOFF: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum McpError {
    #[error("mcp server unavailable: {0}")]
    Unavailable(String),
    #[error("mcp tool returned error (code {code}): {message}")]
    ToolFailed { code: i64, message: String },
    #[error("mcp protocol error: {0}")]
    Protocol(String),
    #[error("mcp request timed out")]
    Timeout,
    #[error("mcp client closed")]
    Closed,
}

type Inbox = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, McpError>>>>>;

pub struct McpClient {
    id: u64,
    next_id: AtomicU64,
    inbox: Inbox,
    writer: Mutex<ChildStdin>,
    /// Keep the subprocess alive for the lifetime of the client. We never
    /// touch this outside tests — it just owns the handle so dropping the
    /// global client tears down the subprocess.
    _child: Mutex<Child>,
}

struct CachedUnavailable {
    message: String,
    retry_after: Instant,
}

struct ClientSlot {
    client: Option<Arc<McpClient>>,
    unavailable: Option<CachedUnavailable>,
}

static CLIENT: Mutex<ClientSlot> = Mutex::const_new(ClientSlot {
    client: None,
    unavailable: None,
});
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

impl McpClient {
    /// Return the process-global MCP client, lazily spawning + initializing
    /// it on first call. On every call while the subprocess is alive, this
    /// returns the cached instance. If startup failed recently, this returns
    /// the cached `Unavailable` until the short backoff expires. `command`,
    /// `args`, and `cwd` after the first live client are ignored — the
    /// global is keyed on neither.
    pub async fn global(
        command: &Path,
        args: &[String],
        cwd: &Path,
    ) -> Result<Arc<McpClient>, McpError> {
        let cmd = command.to_path_buf();
        let cmd_args = args.to_vec();
        let cmd_cwd = cwd.to_path_buf();
        let mut slot = CLIENT.lock().await;

        if let Some(client) = &slot.client {
            return Ok(client.clone());
        }
        if let Some(cached) = &slot.unavailable {
            if Instant::now() < cached.retry_after {
                return Err(McpError::Unavailable(cached.message.clone()));
            }
            slot.unavailable = None;
        }

        match spawn_and_init(cmd, cmd_args, cmd_cwd).await {
            Ok(client) => {
                let client = Arc::new(client);
                slot.client = Some(client.clone());
                Ok(client)
            }
            Err(e) => {
                let message = e.to_string();
                slot.unavailable = Some(CachedUnavailable {
                    message: message.clone(),
                    retry_after: Instant::now() + UNAVAILABLE_BACKOFF,
                });
                Err(McpError::Unavailable(message))
            }
        }
    }

    /// Invoke an MCP tool by name. Returns the concatenated text content
    /// of the tool's reply. `isError: true` responses are surfaced as
    /// `McpError::ToolFailed`.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, McpError> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        let parsed = parse_tool_call_result(&result);
        if parsed.is_error {
            return Err(McpError::ToolFailed {
                code: -1,
                message: if parsed.text.is_empty() {
                    "tool reported isError=true".into()
                } else {
                    parsed.text
                },
            });
        }
        Ok(parsed.text)
    }

    pub async fn list_resources(&self) -> Result<Vec<ResourceMeta>, McpError> {
        let result = self.request("resources/list", json!({})).await?;
        Ok(parse_resources_list(&result))
    }

    pub async fn read_resource(&self, uri: &str) -> Result<String, McpError> {
        let result = self
            .request("resources/read", json!({ "uri": uri }))
            .await?;
        Ok(parse_resource_read(&result))
    }

    #[cfg(test)]
    async fn kill_for_test(&self) {
        let mut child = self._child.lock().await;
        let _ = child.start_kill();
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.inbox.lock().await;
            map.insert(id, tx);
        }
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = format!("{}\n", body);
        {
            let mut w = self.writer.lock().await;
            if w.write_all(line.as_bytes()).await.is_err() || w.flush().await.is_err() {
                self.inbox.lock().await.remove(&id);
                clear_global_client(self.id).await;
                return Err(McpError::Closed);
            }
        }
        match timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => {
                // Reader task exited; oneshot dropped.
                self.inbox.lock().await.remove(&id);
                clear_global_client(self.id).await;
                Err(McpError::Closed)
            }
            Err(_) => {
                self.inbox.lock().await.remove(&id);
                Err(McpError::Timeout)
            }
        }
    }
}

async fn spawn_and_init(
    command: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
) -> Result<McpClient, McpError> {
    let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let mut cmd = Command::new(&command);
    cmd.args(&args)
        .current_dir(&cwd)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| McpError::Unavailable(format!("spawn {}: {e}", command.display())))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| McpError::Unavailable("no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| McpError::Unavailable("no stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| McpError::Unavailable("no stderr".into()))?;

    // Bounded stderr drain — keeps the subprocess from blocking on a
    // full pipe and lets us surface server-side error chatter on init
    // failure. Pattern mirrored from backends/claude_cli.rs.
    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            let mut reader = BufReader::new(stderr);
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut b = buf.lock().await;
                        b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                        if b.len() > 16 * 1024 {
                            let cut = b.len() - 8 * 1024;
                            *b = b.split_off(cut);
                        }
                    }
                }
            }
        });
    }

    let inbox: Inbox = Arc::new(Mutex::new(HashMap::new()));
    {
        let inbox = inbox.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let v: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let Some(id) = v.get("id").and_then(|x| x.as_u64()) else {
                    continue; // notification — ignore
                };
                let mut map = inbox.lock().await;
                let Some(tx) = map.remove(&id) else {
                    continue;
                };
                let payload = if let Some(err) = v.get("error") {
                    let code = err.get("code").and_then(|x| x.as_i64()).unwrap_or(-1);
                    let message = err
                        .get("message")
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    Err(McpError::ToolFailed { code, message })
                } else if let Some(result) = v.get("result") {
                    Ok(result.clone())
                } else {
                    Err(McpError::Protocol(
                        "response without result or error".into(),
                    ))
                };
                let _ = tx.send(payload);
            }
            // stdout closed: drop every pending sender so waiters unblock
            // promptly with `Closed` instead of `Timeout`.
            let mut map = inbox.lock().await;
            map.clear();
            drop(map);
            clear_global_client(client_id).await;
        });
    }

    let client = McpClient {
        id: client_id,
        next_id: AtomicU64::new(1),
        inbox,
        writer: Mutex::new(stdin),
        _child: Mutex::new(child),
    };

    // initialize handshake
    let init_result = timeout(
        INIT_TIMEOUT,
        client.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "aida-chat",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        ),
    )
    .await
    .map_err(|_| {
        McpError::Unavailable(format!(
            "initialize timed out. stderr: {}",
            blocking_stderr_snapshot(&stderr_buf)
        ))
    })?;
    init_result.map_err(|e| {
        McpError::Unavailable(format!(
            "initialize: {e}. stderr: {}",
            blocking_stderr_snapshot(&stderr_buf)
        ))
    })?;

    // initialized notification (no response expected)
    let notify = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    let line = format!("{}\n", notify);
    {
        let mut w = client.writer.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Unavailable(format!("write initialized notification: {e}")))?;
        w.flush()
            .await
            .map_err(|e| McpError::Unavailable(format!("flush initialized notification: {e}")))?;
    }

    Ok(client)
}

fn blocking_stderr_snapshot(buf: &Arc<Mutex<String>>) -> String {
    buf.try_lock()
        .map(|b| b.trim().to_string())
        .unwrap_or_default()
}

async fn clear_global_client(client_id: u64) {
    let mut slot = CLIENT.lock().await;
    if slot.client.as_ref().is_some_and(|c| c.id == client_id) {
        slot.client = None;
    }
}

#[cfg(test)]
async fn clear_global_for_test() {
    let mut slot = CLIENT.lock().await;
    slot.client = None;
    slot.unavailable = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::time::sleep;

    #[tokio::test]
    async fn global_respawns_after_child_death() {
        clear_global_for_test().await;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "aida-chat-mcp-respawn-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("fake-mcp.sh");
        let log = dir.join("spawns.log");
        fs::write(
            &script,
            r#"#!/bin/sh
log="$1"
printf 'spawn\n' >> "$log"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
      ;;
    *)
      ;;
  esac
done
"#,
        )
        .expect("script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");

        let args = vec![log.display().to_string()];
        let first = McpClient::global(&script, &args, &dir)
            .await
            .expect("first spawn");
        first.kill_for_test().await;

        let second = timeout(Duration::from_secs(5), async {
            loop {
                let candidate = McpClient::global(&script, &args, &dir)
                    .await
                    .expect("respawn");
                if !Arc::ptr_eq(&first, &candidate) {
                    break candidate;
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("respawn before timeout");

        assert!(!Arc::ptr_eq(&first, &second));
        let spawns = fs::read_to_string(&log).expect("spawn log");
        assert_eq!(spawns.lines().count(), 2);
        second.kill_for_test().await;
        clear_global_for_test().await;
        let _ = fs::remove_dir_all(&dir);
    }
}
