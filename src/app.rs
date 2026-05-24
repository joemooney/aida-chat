// trace:STORY-2 | ai:claude
//
// Single-page chat shell. One route ("/"), one component. The component
// owns the chat state on the client side, talks to the backend over
// fetch + EventSource, and persists the session id in localStorage so a
// page reload keeps the conversation.

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::messages::{ChatTurn, Role, ToolCallSummary};
#[cfg(feature = "hydrate")]
use crate::messages::{ChatHistory, CommentResponse};

// trace:STORY-21 | ai:claude
//
// SPEC-ID helpers used by the comment-capture affordance. Mirror the
// brief's regex `\b(EPIC|STORY|TASK|BUG|FR|ADR|SPIKE)-\d+\b` without
// pulling in a `regex` dependency (~500 KB of wasm).
//
// - `extract_first_spec_id` finds the first cited SPEC-ID in an
//   assistant message so the form can pre-populate the field.
// - `is_valid_spec_id` is exact-match validation for the field on submit.
//
// Backend re-validates definitively; these are UX guards.

const SPEC_ID_PREFIXES: &[&str] = &["EPIC", "STORY", "TASK", "BUG", "FR", "ADR", "SPIKE"];

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True iff `s` is exactly one valid SPEC-ID with a known prefix
/// (PREFIX-digits, single segment per the brief's regex).
pub fn is_valid_spec_id(s: &str) -> bool {
    let b = s.as_bytes();
    for &p in SPEC_ID_PREFIXES {
        let pb = p.as_bytes();
        if b.len() < pb.len() + 2 {
            continue;
        }
        if &b[..pb.len()] != pb {
            continue;
        }
        if b[pb.len()] != b'-' {
            continue;
        }
        return b[pb.len() + 1..].iter().all(|c| c.is_ascii_digit());
    }
    false
}

/// First substring matching `\b(EPIC|STORY|TASK|BUG|FR|ADR|SPIKE)-\d+\b`
/// in `text`. Returns `None` when no cited SPEC-ID is found.
pub fn extract_first_spec_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        let boundary_before = i == 0 || !is_word_byte(bytes[i - 1]);
        if boundary_before {
            for &p in SPEC_ID_PREFIXES {
                let pb = p.as_bytes();
                if len < i + pb.len() + 2 {
                    continue;
                }
                if &bytes[i..i + pb.len()] != pb {
                    continue;
                }
                if bytes[i + pb.len()] != b'-' {
                    continue;
                }
                let digit_start = i + pb.len() + 1;
                let mut j = digit_start;
                while j < len && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j == digit_start {
                    continue; // no digits — not a valid SPEC-ID
                }
                let boundary_after = j == len || !is_word_byte(bytes[j]);
                if boundary_after {
                    return Some(text[i..j].to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// Render an assistant message's markdown body to safe-ish HTML.
/// We strip raw HTML events from the markdown stream so the model
/// can't inject `<script>` (or any other tag) by writing it verbatim
/// into its reply. Standard markdown — tables, code, lists, links —
/// renders normally.
fn render_markdown(src: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, opts)
        .filter(|e| !matches!(e, Event::Html(_) | Event::InlineHtml(_)));
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

pub fn shell(options: LeptosOptions) -> impl IntoView {
    use leptos::hydration::{AutoReload, HydrationScripts};
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options=options.clone()/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/aida_chat.css"/>
        <Title text="AIDA Chat"/>
        <Router>
            <Routes fallback=|| view! { <p>"404"</p> }>
                <Route path=path!("/") view=ChatPage/>
            </Routes>
        </Router>
    }
}

#[component]
fn ChatPage() -> impl IntoView {
    let (turns, set_turns) = signal::<Vec<ChatTurn>>(vec![]);
    let (draft, set_draft) = signal(String::new());
    let (streaming, set_streaming) = signal(false);
    let (live_text, set_live_text) = signal(String::new());
    let (live_tools, set_live_tools) = signal::<Vec<ToolCallSummary>>(vec![]);
    let (error, set_error) = signal::<Option<String>>(None);
    let (session_id, set_session_id) = signal::<Option<String>>(None);
    #[allow(unused_variables)] // set_backend is only used in the hydrate build
    let (backend, set_backend) = signal::<Option<String>>(None);

    // On mount (client only): pull session_id from localStorage if present
    // and rehydrate history from the server. If none, create one.
    #[cfg(feature = "hydrate")]
    {
        Effect::new(move |prev: Option<()>| {
            if prev.is_some() {
                return;
            }
            // Fetch the active backend in the background so the header
            // can show which one we're talking to.
            leptos::task::spawn_local(async move {
                if let Ok(b) = fetch_backend().await {
                    set_backend.set(Some(b));
                }
            });
            let stored = local_storage_get("aida_chat_session_id");
            match stored {
                Some(id) if !id.is_empty() => {
                    set_session_id.set(Some(id.clone()));
                    leptos::task::spawn_local(async move {
                        match fetch_history(&id).await {
                            Ok(h) => set_turns.set(h.turns),
                            Err(e) => set_error.set(Some(e)),
                        }
                    });
                }
                _ => {
                    leptos::task::spawn_local(async move {
                        match create_session().await {
                            Ok(id) => {
                                local_storage_set("aida_chat_session_id", &id);
                                set_session_id.set(Some(id));
                            }
                            Err(e) => set_error.set(Some(e)),
                        }
                    });
                }
            }
        });
    }

    let new_chat = move |_| {
        set_turns.set(vec![]);
        set_live_text.set(String::new());
        set_live_tools.set(vec![]);
        set_error.set(None);
        set_session_id.set(None);
        #[cfg(feature = "hydrate")]
        {
            leptos::task::spawn_local(async move {
                match create_session().await {
                    Ok(id) => {
                        local_storage_set("aida_chat_session_id", &id);
                        set_session_id.set(Some(id));
                    }
                    Err(e) => set_error.set(Some(e)),
                }
            });
        }
    };

    let send = move |_| {
        if streaming.get_untracked() {
            return;
        }
        let text = draft.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(_sid) = session_id.get_untracked() else {
            set_error.set(Some("Session not ready yet".into()));
            return;
        };
        set_error.set(None);
        set_draft.set(String::new());
        set_turns.update(|t| {
            t.push(ChatTurn {
                role: Role::User,
                text: text.clone(),
                tool_calls: vec![],
            })
        });
        set_streaming.set(true);
        set_live_text.set(String::new());
        set_live_tools.set(vec![]);

        #[cfg(feature = "hydrate")]
        {
            stream_chat(
                _sid,
                text,
                set_live_text,
                set_live_tools,
                move |final_text, tool_calls| {
                    set_turns.update(|t| {
                        t.push(ChatTurn {
                            role: Role::Assistant,
                            text: final_text,
                            tool_calls,
                        })
                    });
                    set_live_text.set(String::new());
                    set_live_tools.set(vec![]);
                    set_streaming.set(false);
                },
                move |err| {
                    set_error.set(Some(err));
                    set_streaming.set(false);
                },
            );
        }
    };

    view! {
        <div class="chat-app">
            <header class="chat-header">
                <h1>"AIDA Chat"</h1>
                <div class="header-right">
                    <Show when=move || backend.get().is_some()>
                        <span class="backend-badge" title="active agent backend">
                            {move || backend.get().unwrap_or_default()}
                        </span>
                    </Show>
                    <button class="new-chat-btn" on:click=new_chat disabled=move || streaming.get()>
                        "New chat"
                    </button>
                </div>
            </header>
            <main class="chat-messages">
                {move || {
                    turns.get()
                        .into_iter()
                        .map(|turn| view! { <TurnView turn=turn session_id=session_id/> })
                        .collect_view()
                }}
                <Show when=move || streaming.get()>
                    <div class="turn assistant streaming">
                        <div class="role">"assistant"</div>
                        <div class="tools">
                            {move || {
                                live_tools.get()
                                    .into_iter()
                                    .map(|tc| view! { <ToolBadge call=tc/> })
                                    .collect_view()
                            }}
                        </div>
                        <div class="text markdown" inner_html=move || render_markdown(&live_text.get())/>
                        <span class="cursor">"▌"</span>
                    </div>
                </Show>
                <Show when=move || error.get().is_some()>
                    <div class="error">{move || error.get().unwrap_or_default()}</div>
                </Show>
            </main>
            <footer class="chat-input">
                <textarea
                    placeholder="Ask about this repo or the AIDA requirements…"
                    prop:value=move || draft.get()
                    on:input=move |ev| set_draft.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" && !ev.shift_key() {
                            ev.prevent_default();
                            send(());
                        }
                    }
                    disabled=move || streaming.get() || session_id.get().is_none()
                />
                <button
                    class="send-btn"
                    on:click=move |_| send(())
                    disabled=move || streaming.get() || session_id.get().is_none()
                >
                    "Send"
                </button>
            </footer>
        </div>
    }
}

#[component]
fn TurnView(turn: ChatTurn, session_id: ReadSignal<Option<String>>) -> impl IntoView {
    let role_class = match turn.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let role_label = match turn.role {
        Role::User => "you",
        Role::Assistant => "assistant",
    };
    let has_tools = !turn.tool_calls.is_empty();
    let tools_view = if has_tools {
        let badges = turn
            .tool_calls
            .clone()
            .into_iter()
            .map(|tc| view! { <ToolBadge call=tc/> })
            .collect_view();
        Some(view! { <div class="tools">{badges}</div> })
    } else {
        None
    };
    let text = turn.text.clone();
    let body = match turn.role {
        // User messages stay as plain pre-wrapped text so the user's
        // exact whitespace/punctuation is preserved.
        Role::User => view! { <div class="text">{text}</div> }.into_any(),
        // Assistant replies render markdown to HTML so tables, code
        // blocks, lists, headings, etc. look right.
        Role::Assistant => {
            let html = render_markdown(&turn.text);
            view! { <div class="text markdown" inner_html=html/> }.into_any()
        }
    };
    let capture_view = match turn.role {
        // trace:STORY-21 | ai:claude
        // Comment-capture affordance — only Assistant messages get the
        // "Save as comment" trigger.
        Role::Assistant => Some(view! { <CommentCapture body_text=turn.text session_id=session_id/> }),
        Role::User => None,
    };
    view! {
        <div class=format!("turn {role_class}")>
            <div class="role">{role_label}</div>
            {tools_view}
            {body}
            {capture_view}
        </div>
    }
}

// trace:STORY-21 | ai:claude
//
// Inline comment-capture form. Lives under each assistant message; click
// "Save as comment" to expand, then POST to /api/sessions/:id/comment.
// State is component-local — a new assistant turn arriving will rebuild
// this component and reset the form, which is acceptable for STORY-21's
// "capture an already-rendered message" flow.
#[component]
fn CommentCapture(
    /// Assistant message body — used to pre-populate the text field.
    body_text: String,
    session_id: ReadSignal<Option<String>>,
) -> impl IntoView {
    let initial_spec_id = extract_first_spec_id(&body_text).unwrap_or_default();
    // StoredValue is Copy, so the inline `on:click` handlers (which run
    // multiple times across re-renders of <Show>) can capture these by
    // move without consuming the underlying String.
    let body_text_sv = StoredValue::new(body_text);
    let initial_spec_id_sv = StoredValue::new(initial_spec_id);

    let (is_open, set_is_open) = signal(false);
    let (spec_id, set_spec_id) = signal(initial_spec_id_sv.get_value());
    let (text, set_text) = signal(body_text_sv.get_value());
    let (submitting, set_submitting) = signal(false);
    let (error_msg, set_error_msg) = signal::<Option<String>>(None);
    let (badge_msg, set_badge_msg) = signal::<Option<String>>(None);

    view! {
        <Show when=move || !is_open.get() && badge_msg.get().is_none()>
            <div class="actions">
                <button
                    class="action-btn"
                    on:click=move |_| {
                        set_spec_id.set(initial_spec_id_sv.get_value());
                        set_text.set(body_text_sv.get_value());
                        set_error_msg.set(None);
                        set_is_open.set(true);
                    }
                    disabled=move || session_id.get().is_none()
                    title="Save this message as a comment on a SPEC-ID"
                >
                    "Save as comment"
                </button>
            </div>
        </Show>
        <Show when=move || badge_msg.get().is_some()>
            <div class="capture-badge">
                {move || badge_msg.get().unwrap_or_default()}
            </div>
        </Show>
        <Show when=move || is_open.get()>
            <div class="comment-form">
                <label class="comment-row">
                    <span class="comment-label">"SPEC-ID"</span>
                    <input
                        class="spec-id-input"
                        prop:value=move || spec_id.get()
                        on:input=move |ev| set_spec_id.set(event_target_value(&ev))
                        placeholder="e.g. EPIC-16"
                    />
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Comment"</span>
                    <textarea
                        class="comment-text"
                        prop:value=move || text.get()
                        on:input=move |ev| set_text.set(event_target_value(&ev))
                        rows="5"
                    />
                </label>
                <Show when=move || error_msg.get().is_some()>
                    <div class="comment-error">
                        {move || error_msg.get().unwrap_or_default()}
                    </div>
                </Show>
                <div class="comment-actions">
                    <button
                        class="save-btn"
                        on:click=move |_| save_comment(
                            session_id,
                            spec_id,
                            text,
                            submitting,
                            set_submitting,
                            set_is_open,
                            set_error_msg,
                            set_badge_msg,
                        )
                        disabled=move || submitting.get()
                    >
                        {move || if submitting.get() { "Saving…" } else { "Save" }}
                    </button>
                    <button
                        class="cancel-btn"
                        on:click=move |_| {
                            set_is_open.set(false);
                            set_error_msg.set(None);
                        }
                        disabled=move || submitting.get()
                    >
                        "Cancel"
                    </button>
                </div>
            </div>
        </Show>
    }
}

// trace:STORY-21 | ai:claude
//
// Save-button handler factored out so the click closure stays small and
// `Fn` even inside the `<Show>` re-render loop. All arguments are Copy
// (signals + the StoredValue-backed widgets) so this can be called any
// number of times.
#[allow(clippy::too_many_arguments)]
fn save_comment(
    session_id: ReadSignal<Option<String>>,
    spec_id: ReadSignal<String>,
    text: ReadSignal<String>,
    submitting: ReadSignal<bool>,
    set_submitting: WriteSignal<bool>,
    set_is_open: WriteSignal<bool>,
    set_error_msg: WriteSignal<Option<String>>,
    set_badge_msg: WriteSignal<Option<String>>,
) {
    if submitting.get_untracked() {
        return;
    }
    let sid_field = spec_id.get_untracked().trim().to_string();
    let text_field = text.get_untracked();
    if !is_valid_spec_id(&sid_field) {
        set_error_msg.set(Some(format!(
            "Not a valid SPEC-ID. Expected one of {} followed by `-` and digits.",
            SPEC_ID_PREFIXES.join("/")
        )));
        return;
    }
    if text_field.trim().is_empty() {
        set_error_msg.set(Some("Comment text is empty.".into()));
        return;
    }
    let Some(sid) = session_id.get_untracked() else {
        set_error_msg.set(Some("Session not ready yet.".into()));
        return;
    };
    set_error_msg.set(None);
    set_submitting.set(true);
    #[cfg(feature = "hydrate")]
    {
        let sid_field_owned = sid_field.clone();
        leptos::task::spawn_local(async move {
            match post_comment(&sid, &sid_field_owned, &text_field).await {
                Ok(message) => {
                    set_submitting.set(false);
                    set_is_open.set(false);
                    let badge_text = if message.trim().is_empty() {
                        format!("Comment added to {sid_field_owned}")
                    } else {
                        message
                    };
                    set_badge_msg.set(Some(badge_text));
                    schedule_clear(set_badge_msg);
                }
                Err(err) => {
                    set_submitting.set(false);
                    set_error_msg.set(Some(err));
                }
            }
        });
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = (sid, sid_field, text_field, set_badge_msg, set_is_open);
        set_submitting.set(false);
    }
}

/// Clear the success badge after a few seconds. Wasm-only — SSR doesn't
/// run JS so the badge would just stay visible (harmless).
#[cfg(feature = "hydrate")]
fn schedule_clear(set_badge_msg: WriteSignal<Option<String>>) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let cb = Closure::<dyn FnMut()>::new(move || {
        set_badge_msg.set(None);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        cb.as_ref().unchecked_ref(),
        3000,
    );
    cb.forget();
}

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
fn schedule_clear(_set_badge_msg: WriteSignal<Option<String>>) {}

#[component]
fn ToolBadge(call: ToolCallSummary) -> impl IntoView {
    let status = if call.ok { "ok" } else { "err" };
    let title = call.input_preview.clone();
    view! {
        <span class=format!("tool-badge {status}") title=title>
            <span class="tool-name">{call.name.clone()}</span>
        </span>
    }
}

// ---------------------------------------------------------------------------
// Client-only helpers (only compiled into the wasm bundle)
// ---------------------------------------------------------------------------

// trace:STORY-21 | ai:claude
#[cfg(test)]
mod spec_id_tests {
    use super::{extract_first_spec_id, is_valid_spec_id};

    #[test]
    fn extracts_first_cited_spec_id() {
        let text = "I checked EPIC-16 and it depends on STORY-3.";
        assert_eq!(extract_first_spec_id(text).as_deref(), Some("EPIC-16"));
    }

    #[test]
    fn handles_every_known_prefix() {
        for (input, expected) in [
            ("see EPIC-1", "EPIC-1"),
            ("see STORY-42 for context", "STORY-42"),
            ("now TASK-7 lands", "TASK-7"),
            ("BUG-103 was the regression", "BUG-103"),
            ("FR-0042 is functional", "FR-0042"),
            ("ADR-9 chose REST over RPC", "ADR-9"),
            ("SPIKE-2 ran for 2 days", "SPIKE-2"),
        ] {
            assert_eq!(
                extract_first_spec_id(input).as_deref(),
                Some(expected),
                "input was {input:?}"
            );
        }
    }

    #[test]
    fn returns_none_when_no_spec_id_cited() {
        assert_eq!(extract_first_spec_id(""), None);
        assert_eq!(extract_first_spec_id("just plain English here"), None);
        assert_eq!(
            extract_first_spec_id("Lowercase epic-16 should not match"),
            None
        );
        assert_eq!(extract_first_spec_id("EPIC-"), None); // no digits
        assert_eq!(extract_first_spec_id("HOOK-12"), None); // unknown prefix
    }

    #[test]
    fn respects_word_boundaries() {
        // No false match inside another word.
        assert_eq!(extract_first_spec_id("RECEPIC-16"), None);
        assert_eq!(extract_first_spec_id("EPIC-16abc"), None);
        // Boundary at start of string.
        assert_eq!(extract_first_spec_id("EPIC-1"), Some("EPIC-1".into()));
        // Boundary at end of string.
        assert_eq!(
            extract_first_spec_id("(EPIC-1)").as_deref(),
            Some("EPIC-1")
        );
        // Adjacent punctuation OK.
        assert_eq!(
            extract_first_spec_id("Reference: STORY-42, please."),
            Some("STORY-42".into())
        );
    }

    #[test]
    fn extracts_when_id_is_inside_markdown_link() {
        let s = "See [STORY-21](https://example.com/STORY-21) for details.";
        assert_eq!(extract_first_spec_id(s).as_deref(), Some("STORY-21"));
    }

    #[test]
    fn is_valid_spec_id_accepts_known_prefixes() {
        for s in ["EPIC-1", "STORY-42", "TASK-7", "BUG-103", "FR-0042", "ADR-9", "SPIKE-2"] {
            assert!(is_valid_spec_id(s), "{s:?} should be valid");
        }
    }

    #[test]
    fn is_valid_spec_id_rejects_bad_shapes() {
        for s in [
            "",
            "EPIC",
            "EPIC-",
            "epic-16",
            "EPIC -16",
            "EPIC-16 ",
            "HOOK-1",
            "EPIC-16-1", // multi-segment not in brief regex
            "EPIC-16abc",
            "(EPIC-16)",
        ] {
            assert!(!is_valid_spec_id(s), "{s:?} should be invalid");
        }
    }
}

#[cfg(feature = "hydrate")]
fn local_storage_get(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
}

#[cfg(feature = "hydrate")]
fn local_storage_set(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, value);
    }
}

#[cfg(feature = "hydrate")]
async fn fetch_backend() -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let opts = RequestInit::new();
    opts.set_method("GET");
    let req = Request::new_with_str_and_init("/api/info", &opts)
        .map_err(|e| format!("request init: {e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "not a response")?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("text await: {e:?}"))?;
    let s = text.as_string().ok_or("text not string")?;
    let parsed: crate::messages::ServerInfo =
        serde_json::from_str(&s).map_err(|e| format!("decode: {e}"))?;
    Ok(parsed.backend)
}

#[cfg(feature = "hydrate")]
async fn create_session() -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let opts = RequestInit::new();
    opts.set_method("POST");
    let req = Request::new_with_str_and_init("/api/sessions", &opts)
        .map_err(|e| format!("request init: {e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "not a response")?;
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("text await: {e:?}"))?;
    let s = text.as_string().ok_or("text not string")?;
    let parsed: crate::messages::CreateSessionResponse =
        serde_json::from_str(&s).map_err(|e| format!("decode: {e}"))?;
    Ok(parsed.session_id)
}

#[cfg(feature = "hydrate")]
async fn fetch_history(session_id: &str) -> Result<ChatHistory, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let opts = RequestInit::new();
    opts.set_method("GET");
    let url = format!("/api/sessions/{session_id}/history");
    let req = Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| format!("request init: {e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "not a response")?;
    if resp.status() == 404 {
        // session no longer exists on the server (probably evicted): clear it
        local_storage_set("aida_chat_session_id", "");
        return Err("session expired".into());
    }
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
        .await
        .map_err(|e| format!("text await: {e:?}"))?;
    let s = text.as_string().ok_or("text not string")?;
    serde_json::from_str(&s).map_err(|e| format!("decode: {e}"))
}

// trace:STORY-21 | ai:claude
//
// POST /api/sessions/{session_id}/comment with {spec_id, text}. Returns
// the success message (or stub message) on Ok, and a user-facing error
// string on Err. Matches the contract:
//   200 {"ok": true, "message": "..."}
//   400|500 {"ok": false, "error": "..."}
#[cfg(feature = "hydrate")]
async fn post_comment(session_id: &str, spec_id: &str, text: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

    let body = serde_json::to_string(&crate::messages::CommentRequest {
        spec_id: spec_id.to_string(),
        text: text.to_string(),
    })
    .map_err(|e| format!("encode: {e}"))?;
    let headers = Headers::new().map_err(|e| format!("headers: {e:?}"))?;
    headers
        .set("content-type", "application/json")
        .map_err(|e| format!("set header: {e:?}"))?;

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_headers(&headers);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&body));

    let url = format!("/api/sessions/{session_id}/comment");
    let req = Request::new_with_str_and_init(&url, &opts)
        .map_err(|e| format!("request init: {e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|e| format!("fetch: {e:?}"))?;
    let resp: Response = resp_value.dyn_into().map_err(|_| "not a response")?;
    let status = resp.status();
    let text_promise = resp.text().map_err(|e| format!("text: {e:?}"))?;
    let body_text = JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("text await: {e:?}"))?
        .as_string()
        .unwrap_or_default();
    // Try the typed envelope first; fall back to the raw status + body on a
    // backend that returns a non-conforming shape (e.g. a plain 500).
    if let Ok(parsed) = serde_json::from_str::<CommentResponse>(&body_text) {
        if parsed.ok {
            return Ok(parsed.message.unwrap_or_default());
        }
        return Err(parsed.error.unwrap_or_else(|| format!("HTTP {status}")));
    }
    if (200..300).contains(&status) {
        Ok(body_text)
    } else {
        Err(format!("HTTP {status}: {}", body_text.trim()))
    }
}

#[cfg(feature = "hydrate")]
fn stream_chat(
    session_id: String,
    user_text: String,
    set_live_text: WriteSignal<String>,
    set_live_tools: WriteSignal<Vec<ToolCallSummary>>,
    on_done: impl Fn(String, Vec<ToolCallSummary>) + 'static,
    on_error: impl Fn(String) + 'static,
) {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    use web_sys::{EventSource, MessageEvent};

    let encoded_session = js_sys::encode_uri_component(&session_id);
    let encoded_text = js_sys::encode_uri_component(&user_text);
    let url = format!(
        "/api/chat?session_id={}&q={}",
        encoded_session.as_string().unwrap_or_default(),
        encoded_text.as_string().unwrap_or_default()
    );

    let es = match EventSource::new(&url) {
        Ok(es) => es,
        Err(e) => {
            on_error(format!("EventSource failed: {e:?}"));
            return;
        }
    };
    let es_holder: Rc<RefCell<Option<EventSource>>> = Rc::new(RefCell::new(Some(es.clone())));
    let accumulated: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let tools: Rc<RefCell<Vec<ToolCallSummary>>> = Rc::new(RefCell::new(vec![]));
    let on_done = Rc::new(on_done);
    let on_error = Rc::new(on_error);

    // text event: streaming text delta
    {
        let accumulated = accumulated.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            if let Some(s) = ev.data().as_string() {
                let mut acc = accumulated.borrow_mut();
                acc.push_str(&s);
                set_live_text.set(acc.clone());
            }
        });
        es.add_event_listener_with_callback("text", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // tool event: tool call started/finished (one event per finished call)
    {
        let tools = tools.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            if let Some(s) = ev.data().as_string() {
                if let Ok(tc) = serde_json::from_str::<ToolCallSummary>(&s) {
                    let mut t = tools.borrow_mut();
                    t.push(tc);
                    set_live_tools.set(t.clone());
                }
            }
        });
        es.add_event_listener_with_callback("tool", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // done event: agent finished cleanly
    {
        let es_holder = es_holder.clone();
        let accumulated = accumulated.clone();
        let tools = tools.clone();
        let on_done = on_done.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |_ev: MessageEvent| {
            if let Some(es) = es_holder.borrow_mut().take() {
                es.close();
            }
            let text = accumulated.borrow().clone();
            let tc = tools.borrow().clone();
            on_done(text, tc);
        });
        es.add_event_listener_with_callback("done", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // err event: backend reported an error
    {
        let es_holder = es_holder.clone();
        let on_error = on_error.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            if let Some(es) = es_holder.borrow_mut().take() {
                es.close();
            }
            let msg = ev.data().as_string().unwrap_or_else(|| "stream error".into());
            on_error(msg);
        });
        es.add_event_listener_with_callback("err", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // Default onerror (transport-level): only treat as fatal if the
    // EventSource hasn't already been closed by a "done" / "err" event.
    {
        let es_holder = es_holder.clone();
        let on_error = on_error.clone();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if let Some(es) = es_holder.borrow_mut().take() {
                es.close();
                on_error("stream disconnected".into());
            }
        });
        es.set_onerror(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}
