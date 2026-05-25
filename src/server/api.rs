// trace:STORY-5 | ai:claude
//
// HTTP API: session create/history + SSE streaming endpoint that the
// browser's EventSource hits. EventSource only does GET, so /api/chat
// is GET with the user message as a query parameter (URL-encoded).

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Extension, Json as AxumJson, Path, Query};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use leptos::prelude::LeptosOptions;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::messages::{
    ChatHistory, ChatTurn, CommentRequest, CommentResponse, CreateSessionResponse, Role,
    ServerInfo, SpecRequest, SpecResponse, UltraplanRequest, UltraplanResponse,
};
use crate::server::agent::{self, AgentEvent};
use crate::server::backends::claude_cli;
use crate::server::config::{Backend, ServerConfig};
use crate::server::sessions::SessionStore;
use crate::server::tools::{aida, ToolError};

#[derive(Clone)]
pub struct ApiState {
    pub sessions: Arc<dyn SessionStore>,
    pub cfg: Arc<ServerConfig>,
}

/// API routes mounted at /api. Returns a Router that inherits LeptosOptions
/// state from the parent and accesses ApiState via Extension, so it can be
/// nested into the main Leptos router without state-type conflicts.
pub fn router(sessions: Arc<dyn SessionStore>, cfg: Arc<ServerConfig>) -> Router<LeptosOptions> {
    let state = ApiState { sessions, cfg };
    Router::<LeptosOptions>::new()
        .route("/info", get(get_info))
        .route("/sessions", post(create_session))
        .route("/sessions/{id}/history", get(get_history))
        .route("/sessions/{id}/comment", post(add_comment))
        .route("/sessions/{id}/spec", post(add_spec))
        // trace:STORY-24 | ai:agy
        .route("/sessions/{id}/ultraplan", post(seed_ultraplan))
        .route("/chat", get(chat_stream))
        .layer(Extension(state))
}

async fn get_info(Extension(state): Extension<ApiState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        backend: state.cfg.backend.as_str().into(),
    })
}

async fn create_session(Extension(state): Extension<ApiState>) -> Json<CreateSessionResponse> {
    let s = state.sessions.create().await;
    // Pre-warm the claude subprocess in the background. The user will
    // be typing their first message for a few seconds; by the time it
    // arrives, the process should already be past its init phase.
    if state.cfg.backend == Backend::ClaudeCli {
        let cfg = state.cfg.clone();
        let sid = s.id.clone();
        tokio::spawn(async move {
            claude_cli::prewarm(cfg, sid).await;
        });
    }
    Json(CreateSessionResponse { session_id: s.id })
}

async fn get_history(
    Extension(state): Extension<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<ChatHistory>, StatusCode> {
    match state.sessions.get(&id).await {
        Some(s) => Ok(Json(ChatHistory {
            session_id: id,
            turns: s.transcript.clone(),
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Deserialize)]
struct ChatQuery {
    session_id: String,
    q: String,
}

// trace:STORY-21 | ai:codex
async fn add_comment(
    Extension(state): Extension<ApiState>,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<CommentRequest>,
) -> (StatusCode, Json<CommentResponse>) {
    if state.sessions.get(&id).await.is_none() {
        return comment_error(StatusCode::NOT_FOUND, "unknown session_id".into());
    }

    let spec_id = body.spec_id;
    let text = body.text;
    match aida::aida_comment_add(
        &state.cfg,
        &json!({
            "spec_id": spec_id.clone(),
            "text": text,
        }),
    )
    .await
    {
        Ok(_) => comment_ok(format!("Comment added to {spec_id}")),
        Err(ToolError::BadInput(e)) => comment_error(StatusCode::BAD_REQUEST, e),
        Err(e) => comment_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn comment_ok(message: String) -> (StatusCode, Json<CommentResponse>) {
    (
        StatusCode::OK,
        Json(CommentResponse {
            ok: true,
            message: Some(message),
            error: None,
        }),
    )
}

fn comment_error(status: StatusCode, error: String) -> (StatusCode, Json<CommentResponse>) {
    (
        status,
        Json(CommentResponse {
            ok: false,
            message: None,
            error: Some(error),
        }),
    )
}

// trace:STORY-22 | ai:codex
async fn add_spec(
    Extension(state): Extension<ApiState>,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<SpecRequest>,
) -> (StatusCode, Json<SpecResponse>) {
    if state.sessions.get(&id).await.is_none() {
        return spec_error(StatusCode::NOT_FOUND, "unknown session_id".into());
    }

    match aida::aida_add(
        &state.cfg,
        &json!({
            "type": body.r#type,
            "title": body.title,
            "description": body.description,
        }),
    )
    .await
    {
        Ok(spec_id) => spec_ok(spec_id),
        Err(ToolError::BadInput(e)) => spec_error(StatusCode::BAD_REQUEST, e),
        Err(e) => spec_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn spec_ok(spec_id: String) -> (StatusCode, Json<SpecResponse>) {
    (
        StatusCode::OK,
        Json(SpecResponse {
            ok: true,
            spec_id: Some(spec_id.clone()),
            message: Some(format!("Created {spec_id}")),
            error: None,
        }),
    )
}

fn spec_error(status: StatusCode, error: String) -> (StatusCode, Json<SpecResponse>) {
    (
        status,
        Json(SpecResponse {
            ok: false,
            spec_id: None,
            message: None,
            error: Some(error),
        }),
    )
}

// trace:STORY-24 | ai:agy
async fn seed_ultraplan(
    Extension(state): Extension<ApiState>,
    Path(id): Path<String>,
    AxumJson(body): AxumJson<UltraplanRequest>,
) -> (StatusCode, Json<UltraplanResponse>) {
    if state.sessions.get(&id).await.is_none() {
        return ultraplan_error(StatusCode::NOT_FOUND, "unknown session_id".into());
    }

    let spec_id = body.spec_id;
    match aida::aida_ultraplan(
        &state.cfg,
        &json!({
            "spec_id": spec_id.clone(),
        }),
    )
    .await
    {
        Ok(prompt) => ultraplan_ok(prompt, format!("Plan prompt generated")),
        Err(ToolError::BadInput(e)) => ultraplan_error(StatusCode::BAD_REQUEST, e),
        Err(e) => ultraplan_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// trace:STORY-24 | ai:agy
fn ultraplan_ok(prompt: String, message: String) -> (StatusCode, Json<UltraplanResponse>) {
    (
        StatusCode::OK,
        Json(UltraplanResponse {
            ok: true,
            prompt: Some(prompt),
            message: Some(message),
            error: None,
        }),
    )
}

// trace:STORY-24 | ai:agy
fn ultraplan_error(status: StatusCode, error: String) -> (StatusCode, Json<UltraplanResponse>) {
    (
        status,
        Json(UltraplanResponse {
            ok: false,
            prompt: None,
            message: None,
            error: Some(error),
        }),
    )
}

async fn chat_stream(
    Extension(state): Extension<ApiState>,
    Query(params): Query<ChatQuery>,
) -> axum::response::Response {
    // Validate session exists before opening the stream so we can return
    // a proper HTTP error rather than a stream that immediately closes.
    if state.sessions.get(&params.session_id).await.is_none() {
        return (StatusCode::NOT_FOUND, "unknown session_id").into_response();
    }

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let cfg = state.cfg.clone();
    let sessions = state.sessions.clone();
    let session_id = params.session_id.clone();
    let user_text = params.q.clone();

    tokio::spawn(async move {
        agent::run_turn(cfg, sessions, session_id, user_text, tx).await;
    });

    let stream = ReceiverStream::new(rx).map(event_to_sse);
    Sse::new(stream).keep_alive(KeepAlive::new()).into_response()
}

fn event_to_sse(ev: AgentEvent) -> Result<Event, Infallible> {
    let evt = match ev {
        AgentEvent::TextDelta(s) => Event::default().event("text").data(s),
        AgentEvent::ToolCall(tc) => {
            let json = serde_json::to_string(&tc).unwrap_or_else(|_| "{}".into());
            Event::default().event("tool").data(json)
        }
        AgentEvent::Done => Event::default().event("done").data("ok"),
        AgentEvent::Error(msg) => Event::default().event("err").data(msg),
    };
    Ok(evt)
}

// `ChatHistory` is what the browser sees on a page reload. Re-derived
// from the session's transcript, which is built up server-side as the
// conversation progresses.
#[allow(dead_code)]
fn synthesize_empty_history(id: String) -> ChatHistory {
    ChatHistory {
        session_id: id,
        turns: vec![ChatTurn {
            role: Role::Assistant,
            text: "Hi! Ask about this repo or its requirements.".into(),
            tool_calls: vec![],
        }],
    }
}
