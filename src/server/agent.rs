// trace:STORY-3 STORY-15 | ai:claude
//
// Agent dispatch: picks a concrete backend per config and forwards the
// user turn to it. Both backends emit the same `AgentEvent` stream so
// the SSE layer in api.rs is identical regardless of backend.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::messages::{ChartArtifact, ToolCall};
use crate::server::backends;
use crate::server::config::{Backend, ServerConfig};
use crate::server::sessions::SessionStore;

/// Events emitted by an agent backend while it handles one user turn.
/// Mapped 1:1 to SSE events the browser sees.
pub enum AgentEvent {
    /// One incremental chunk of assistant text.
    TextDelta(String),
    /// A tool just finished (success or error).
    ToolCall(ToolCall),
    /// trace:EPIC-29 | ai:claude
    /// A `chart_*` tool produced a rendered SVG artifact. Flows
    /// out-of-band from the tool's text result so the model doesn't
    /// see the raw SVG bytes, only the summary. The SSE side
    /// serializes this as event `chart` with the JSON-encoded
    /// `ChartArtifact` as the data field.
    ChartArtifact(ChartArtifact),
    /// Turn finished cleanly.
    Done,
    /// Fatal error during the turn; the channel will close right after.
    Error(String),
}

pub async fn run_turn(
    cfg: Arc<ServerConfig>,
    sessions: Arc<dyn SessionStore>,
    session_id: String,
    user_text: String,
    tx: mpsc::Sender<AgentEvent>,
) {
    match cfg.backend {
        Backend::Anthropic => {
            backends::anthropic::run_turn(cfg, sessions, session_id, user_text, tx).await
        }
        Backend::ClaudeCli => {
            backends::claude_cli::run_turn(cfg, sessions, session_id, user_text, tx).await
        }
    }
}
