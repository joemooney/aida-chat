// trace:STORY-5 | ai:claude
//
// Per-user conversation sessions. `SessionStore` is the trait the API
// layer talks to; `InMemorySessions` is the only impl for now. A future
// Postgres/sled-backed impl can drop in without changes upstream.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::messages::{ChatTurn, Role};
use crate::server::config::ServerConfig;

/// Native message representation used by the agent loop. Mirrors the
/// shape the Anthropic Messages API expects when we replay a multi-turn
/// conversation.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    User { text: String },
    /// One full assistant turn — may contain text and/or tool_use blocks.
    Assistant { content: Vec<AssistantBlock> },
    /// Tool results, sent back as a synthetic user-role turn.
    ToolResults { results: Vec<ToolResult> },
}

#[derive(Debug, Clone)]
pub enum AssistantBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_active: Instant,
    /// Native conversation history replayed to Anthropic on each turn.
    pub history: Vec<AgentMessage>,
    /// Visible transcript shown to the browser. Stays loosely aligned
    /// with `history`: one ChatTurn::User per user message, one
    /// ChatTurn::Assistant per finished assistant *turn* (which may
    /// have involved several tool round-trips internally).
    pub transcript: Vec<ChatTurn>,
}

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    async fn create(&self) -> SessionState;
    async fn get(&self, id: &str) -> Option<SessionState>;
    /// Append a user message to both history and the visible transcript.
    async fn append_user(&self, id: &str, text: &str) -> Result<(), String>;
    /// Append the agent's work for one user turn: zero or more
    /// Assistant/ToolResults entries (the raw history additions) plus
    /// one ChatTurn for the visible transcript.
    async fn commit_assistant_turn(
        &self,
        id: &str,
        history_appendix: Vec<AgentMessage>,
        transcript_turn: ChatTurn,
    ) -> Result<(), String>;
    /// Evict sessions that have been idle longer than the configured TTL.
    async fn evict_idle(&self);
}

pub struct InMemorySessions {
    inner: Mutex<HashMap<String, SessionState>>,
    cfg: Arc<ServerConfig>,
}

impl InMemorySessions {
    pub fn new(cfg: Arc<ServerConfig>) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            cfg,
        }
    }
}

#[async_trait::async_trait]
impl SessionStore for InMemorySessions {
    async fn create(&self) -> SessionState {
        let id = Uuid::new_v4().to_string();
        let state = SessionState {
            id: id.clone(),
            created_at: chrono::Utc::now(),
            last_active: Instant::now(),
            history: vec![],
            transcript: vec![],
        };
        self.inner.lock().await.insert(id.clone(), state.clone());
        state
    }

    async fn get(&self, id: &str) -> Option<SessionState> {
        let mut map = self.inner.lock().await;
        if let Some(s) = map.get_mut(id) {
            s.last_active = Instant::now();
            return Some(s.clone());
        }
        None
    }

    async fn append_user(&self, id: &str, text: &str) -> Result<(), String> {
        let mut map = self.inner.lock().await;
        let s = map.get_mut(id).ok_or_else(|| "no such session".to_string())?;
        s.history.push(AgentMessage::User { text: text.to_string() });
        s.transcript.push(ChatTurn {
            role: Role::User,
            text: text.to_string(),
            tool_calls: vec![],
            chart_artifacts: vec![],
        });
        s.last_active = Instant::now();
        Ok(())
    }

    async fn commit_assistant_turn(
        &self,
        id: &str,
        history_appendix: Vec<AgentMessage>,
        transcript_turn: ChatTurn,
    ) -> Result<(), String> {
        let mut map = self.inner.lock().await;
        let s = map.get_mut(id).ok_or_else(|| "no such session".to_string())?;
        s.history.extend(history_appendix);
        s.transcript.push(transcript_turn);
        s.last_active = Instant::now();
        Ok(())
    }

    async fn evict_idle(&self) {
        let ttl = self.cfg.session_ttl;
        let now = Instant::now();
        let mut map = self.inner.lock().await;
        map.retain(|_, s| now.duration_since(s.last_active) < ttl);
    }
}
