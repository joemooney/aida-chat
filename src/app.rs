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

#[cfg(feature = "hydrate")]
use crate::messages::{ChatHistory, CommentResponse, MemoryResponse, SpecResponse};
use crate::messages::{ChartArtifact, ChatTurn, Role, ToolCall};

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

// trace:STORY-22 | ai:claude
//
// Title heuristic for the new-SPEC capture form: take the first
// sentence (`.`, `!`, `?`, or newline-terminated), trimmed; if it
// exceeds `max_len` chars, truncate at the nearest preceding word
// boundary and append `…`. Empty input gives an empty title — the
// user fills it in manually.
pub fn extract_first_sentence_title(body: &str, max_len: usize) -> String {
    let trimmed = body.trim_start();
    let end_byte = trimmed
        .char_indices()
        .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    let first = trimmed[..end_byte].trim();
    if first.chars().count() <= max_len {
        return first.to_string();
    }
    // Find the byte index at exactly `max_len` chars in.
    let mut cap_byte = first.len();
    for (i, (byte_idx, _)) in first.char_indices().enumerate() {
        if i == max_len {
            cap_byte = byte_idx;
            break;
        }
    }
    let prefix = &first[..cap_byte];
    // Prefer a word boundary — but only if there's whitespace within the
    // last 20 chars, so we don't collapse "OneVeryLongUnbrokenString" to "".
    if let Some(last_space) = prefix.rfind(char::is_whitespace) {
        let after_space = &prefix[..last_space];
        if after_space.chars().count() + 20 >= max_len {
            return format!("{after_space}…");
        }
    }
    format!("{prefix}…")
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
    let (live_tools, set_live_tools) = signal::<Vec<ToolCall>>(vec![]);
    // trace:EPIC-29 | ai:claude — chart artifacts streamed during this turn.
    let (live_charts, set_live_charts) = signal::<Vec<ChartArtifact>>(vec![]);
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
        set_live_charts.set(vec![]);
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
                chart_artifacts: vec![],
            })
        });
        set_streaming.set(true);
        set_live_text.set(String::new());
        set_live_tools.set(vec![]);
        set_live_charts.set(vec![]);

        #[cfg(feature = "hydrate")]
        {
            stream_chat(
                _sid,
                text,
                set_live_text,
                set_live_tools,
                set_live_charts,
                move |final_text, tool_calls, chart_artifacts| {
                    set_turns.update(|t| {
                        t.push(ChatTurn {
                            role: Role::Assistant,
                            text: final_text,
                            tool_calls,
                            chart_artifacts,
                        })
                    });
                    set_live_text.set(String::new());
                    set_live_tools.set(vec![]);
                    set_live_charts.set(vec![]);
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
                        // trace:STORY-14 | ai:claude — same ToolStrip drives live + historical.
                        <ToolStrip tool_calls=live_tools.into()/>
                        <div class="text markdown" inner_html=move || render_markdown(&live_text.get())/>
                        <span class="cursor">"▌"</span>
                        // trace:EPIC-29 | ai:claude — live chart artifacts.
                        <div class="chart-gallery">
                            {move || {
                                live_charts.get()
                                    .into_iter()
                                    .map(|art| view! { <ChartArtifactView art=art/> })
                                    .collect_view()
                            }}
                        </div>
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
    // trace:STORY-14 | ai:claude
    // ToolStrip wants a Signal so live and historical paths share one
    // component. For historical turns the data is immutable, so wrap
    // it in a StoredValue-backed derived signal.
    let has_tools = !turn.tool_calls.is_empty();
    let tools_view = if has_tools {
        let stored = StoredValue::new(turn.tool_calls.clone());
        let tool_calls_signal: Signal<Vec<ToolCall>> =
            Signal::derive(move || stored.get_value());
        Some(view! { <ToolStrip tool_calls=tool_calls_signal/> })
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
        // trace:STORY-21 STORY-22 STORY-23 | ai:claude
        // Three capture affordances per assistant message: comment on an
        // existing SPEC, create a new SPEC, save as a memory file. Kept
        // parallel (no shared abstraction) on purpose — see the post-PR
        // factoring note at the bottom of this file.
        Role::Assistant => {
            let body_for_comment = turn.text.clone();
            let body_for_spec = turn.text.clone();
            let body_for_memory = turn.text;
            Some(view! {
                <CommentCapture body_text=body_for_comment session_id=session_id/>
                <SpecCapture body_text=body_for_spec session_id=session_id/>
                <MemoryCapture body_text=body_for_memory session_id=session_id/>
            })
        }
        Role::User => None,
    };
    // trace:EPIC-29 | ai:claude — render any chart artifacts emitted
    // during the turn (persisted on ChatTurn, replays on /history).
    let chart_view = if !turn.chart_artifacts.is_empty() {
        let artifacts = turn.chart_artifacts.clone();
        Some(view! {
            <div class="chart-gallery">
                {artifacts.into_iter().map(|art| view! { <ChartArtifactView art=art/> }).collect_view()}
            </div>
        })
    } else {
        None
    };
    view! {
        <div class=format!("turn {role_class}")>
            <div class="role">{role_label}</div>
            {tools_view}
            {body}
            {chart_view}
            {capture_view}
        </div>
    }
}

// =========================================================================
// trace:STORY-21 STORY-22 STORY-23 | ai:claude
//
// Three parallel capture components: CommentCapture, SpecCapture,
// MemoryCapture. See the factoring-decision block in the source — kept
// parallel until a fourth instance + stable variability matrix.
// =========================================================================

// trace:EPIC-29 | ai:claude
//
// Renders a single chart SVG inline. The SVG is server-rendered and
// trusted (we control the generator end-to-end). Caption — when
// present — appears below the chart in muted small caps.
#[component]
fn ChartArtifactView(art: ChartArtifact) -> impl IntoView {
    let svg_html = art.svg;
    let kind = art.kind.clone();
    let caption = art.caption.clone();
    view! {
        <div class=format!("chart-artifact chart-{}", kind)>
            <div class="chart-svg" inner_html=svg_html></div>
            {caption.map(|c| view! { <div class="chart-caption">{c}</div> })}
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

// trace:STORY-22 | ai:claude
//
// Inline new-SPEC capture form. Parallel to CommentCapture: same
// StoredValue trick for re-prime, same `<Show>` gating, same badge
// fade. Deliberately not factored against CommentCapture yet — wait
// for STORY-23 to choose the right shared abstraction.
#[component]
fn SpecCapture(
    /// Assistant message body — used to pre-populate title + description.
    body_text: String,
    session_id: ReadSignal<Option<String>>,
) -> impl IntoView {
    let initial_title = extract_first_sentence_title(&body_text, 80);
    let body_sv = StoredValue::new(body_text);
    let initial_title_sv = StoredValue::new(initial_title);

    let (is_open, set_is_open) = signal(false);
    let (spec_type, set_spec_type) = signal("task".to_string());
    let (title, set_title) = signal(initial_title_sv.get_value());
    let (description, set_description) = signal(body_sv.get_value());
    let (submitting, set_submitting) = signal(false);
    let (error_msg, set_error_msg) = signal::<Option<String>>(None);
    let (badge_msg, set_badge_msg) = signal::<Option<String>>(None);

    view! {
        <Show when=move || !is_open.get() && badge_msg.get().is_none()>
            <div class="actions">
                <button
                    class="action-btn"
                    on:click=move |_| {
                        set_spec_type.set("task".to_string());
                        set_title.set(initial_title_sv.get_value());
                        set_description.set(body_sv.get_value());
                        set_error_msg.set(None);
                        set_is_open.set(true);
                    }
                    disabled=move || session_id.get().is_none()
                    title="Create a new AIDA requirement from this message"
                >
                    "Create as new SPEC"
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
                    <span class="comment-label">"Type"</span>
                    <select
                        class="spec-type-select"
                        on:change=move |ev| set_spec_type.set(event_target_value(&ev))
                        prop:value=move || spec_type.get()
                    >
                        <option value="task">"task"</option>
                        <option value="bug">"bug"</option>
                        <option value="story">"story"</option>
                        <option value="epic">"epic"</option>
                        <option value="spike">"spike"</option>
                    </select>
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Title"</span>
                    <input
                        class="spec-title-input"
                        prop:value=move || title.get()
                        on:input=move |ev| set_title.set(event_target_value(&ev))
                        maxlength="200"
                        placeholder="Short, descriptive title"
                    />
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Description"</span>
                    <textarea
                        class="comment-text"
                        prop:value=move || description.get()
                        on:input=move |ev| set_description.set(event_target_value(&ev))
                        rows="6"
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
                        on:click=move |_| save_spec(
                            session_id,
                            spec_type,
                            title,
                            description,
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

// trace:STORY-22 | ai:claude
//
// Save handler for SpecCapture. Parallel to save_comment — kept
// separate intentionally.
#[allow(clippy::too_many_arguments)]
fn save_spec(
    session_id: ReadSignal<Option<String>>,
    spec_type: ReadSignal<String>,
    title: ReadSignal<String>,
    description: ReadSignal<String>,
    submitting: ReadSignal<bool>,
    set_submitting: WriteSignal<bool>,
    set_is_open: WriteSignal<bool>,
    set_error_msg: WriteSignal<Option<String>>,
    set_badge_msg: WriteSignal<Option<String>>,
) {
    if submitting.get_untracked() {
        return;
    }
    let type_field = spec_type.get_untracked();
    let title_field = title.get_untracked().trim().to_string();
    let desc_field = description.get_untracked();
    if title_field.is_empty() {
        set_error_msg.set(Some("Title is required.".into()));
        return;
    }
    if desc_field.trim().is_empty() {
        set_error_msg.set(Some("Description is required.".into()));
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
        leptos::task::spawn_local(async move {
            match post_spec(&sid, &type_field, &title_field, &desc_field).await {
                Ok((spec_id, message)) => {
                    set_submitting.set(false);
                    set_is_open.set(false);
                    let badge_text = if !spec_id.trim().is_empty() {
                        format!("Created {}", spec_id.trim())
                    } else if !message.trim().is_empty() {
                        message
                    } else {
                        "Created (backend returned no SPEC-ID)".into()
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
        let _ = (
            sid,
            type_field,
            title_field,
            desc_field,
            set_badge_msg,
            set_is_open,
        );
        set_submitting.set(false);
    }
}

// trace:STORY-23 | ai:claude
//
// Inline memory-write capture form. Third instance of the capture-loop
// UX (after CommentCapture + SpecCapture). Kept parallel to the others
// for now — see the factoring note at the bottom of this file for why.
//
// The four `MemoryType` choices match the auto-memory `<types>` taxonomy
// used by Claude Code globally (user / feedback / project / reference).
// `feedback` is the default because "save this assistant message as a
// memory" is overwhelmingly a "this is a correction or principle" intent.
const MEMORY_TYPES: &[&str] = &["user", "feedback", "project", "reference"];
const MEMORY_NAME_MAX: usize = 80;
const MEMORY_DESCRIPTION_MAX: usize = 150;

/// True iff `s` matches `^[a-z][a-z0-9_-]{0,79}$` — the brief's slug
/// regex for memory file names. Hand-rolled to avoid a `regex` dep.
pub fn is_valid_memory_name(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > MEMORY_NAME_MAX {
        return false;
    }
    if !b[0].is_ascii_lowercase() {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn is_valid_memory_type(s: &str) -> bool {
    MEMORY_TYPES.contains(&s)
}

#[component]
fn MemoryCapture(
    /// Assistant message body — pre-populates the memory body.
    body_text: String,
    session_id: ReadSignal<Option<String>>,
) -> impl IntoView {
    let body_sv = StoredValue::new(body_text);

    let (is_open, set_is_open) = signal(false);
    let (name, set_name) = signal(String::new());
    let (description, set_description) = signal(String::new());
    let (mem_type, set_mem_type) = signal("feedback".to_string());
    let (body, set_body) = signal(body_sv.get_value());
    let (submitting, set_submitting) = signal(false);
    let (error_msg, set_error_msg) = signal::<Option<String>>(None);
    let (badge_msg, set_badge_msg) = signal::<Option<String>>(None);

    view! {
        <Show when=move || !is_open.get() && badge_msg.get().is_none()>
            <div class="actions">
                <button
                    class="action-btn"
                    on:click=move |_| {
                        set_name.set(String::new());
                        set_description.set(String::new());
                        set_mem_type.set("feedback".to_string());
                        set_body.set(body_sv.get_value());
                        set_error_msg.set(None);
                        set_is_open.set(true);
                    }
                    disabled=move || session_id.get().is_none()
                    title="Save this message as a markdown memory under ~/.claude/projects/<slug>/memory/"
                >
                    "Save as memory"
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
                    <span class="comment-label">"Name (slug)"</span>
                    <input
                        class="memory-name-input"
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                        placeholder="e.g. login-edge-case"
                        maxlength="80"
                    />
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Description"</span>
                    <input
                        class="memory-description-input"
                        prop:value=move || description.get()
                        on:input=move |ev| set_description.set(event_target_value(&ev))
                        placeholder="One-line summary of what this memory captures"
                        maxlength="150"
                    />
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Type"</span>
                    <select
                        class="spec-type-select"
                        on:change=move |ev| set_mem_type.set(event_target_value(&ev))
                        prop:value=move || mem_type.get()
                    >
                        <option value="user">"user"</option>
                        <option value="feedback">"feedback"</option>
                        <option value="project">"project"</option>
                        <option value="reference">"reference"</option>
                    </select>
                </label>
                <label class="comment-row">
                    <span class="comment-label">"Body"</span>
                    <textarea
                        class="comment-text"
                        prop:value=move || body.get()
                        on:input=move |ev| set_body.set(event_target_value(&ev))
                        rows="6"
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
                        on:click=move |_| save_memory(
                            session_id,
                            name,
                            description,
                            mem_type,
                            body,
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

// trace:STORY-23 | ai:claude
//
// Save handler for MemoryCapture. Mirrors save_spec/save_comment.
#[allow(clippy::too_many_arguments)]
fn save_memory(
    session_id: ReadSignal<Option<String>>,
    name: ReadSignal<String>,
    description: ReadSignal<String>,
    mem_type: ReadSignal<String>,
    body: ReadSignal<String>,
    submitting: ReadSignal<bool>,
    set_submitting: WriteSignal<bool>,
    set_is_open: WriteSignal<bool>,
    set_error_msg: WriteSignal<Option<String>>,
    set_badge_msg: WriteSignal<Option<String>>,
) {
    if submitting.get_untracked() {
        return;
    }
    let name_field = name.get_untracked().trim().to_string();
    let description_field = description.get_untracked().trim().to_string();
    let type_field = mem_type.get_untracked();
    let body_field = body.get_untracked();

    if !is_valid_memory_name(&name_field) {
        set_error_msg.set(Some(
            "Name must be lowercase kebab-slug: start with a letter, then \
             letters/digits/hyphens/underscores (max 80 chars)."
                .into(),
        ));
        return;
    }
    if description_field.is_empty() {
        set_error_msg.set(Some("Description is required.".into()));
        return;
    }
    if description_field.chars().count() > MEMORY_DESCRIPTION_MAX {
        set_error_msg.set(Some(format!(
            "Description must be {MEMORY_DESCRIPTION_MAX} chars or fewer."
        )));
        return;
    }
    if !is_valid_memory_type(&type_field) {
        set_error_msg.set(Some(format!(
            "Type must be one of {}.",
            MEMORY_TYPES.join("/")
        )));
        return;
    }
    if body_field.trim().is_empty() {
        set_error_msg.set(Some("Body is empty.".into()));
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
        let name_for_badge = name_field.clone();
        leptos::task::spawn_local(async move {
            match post_memory(&sid, &name_field, &description_field, &type_field, &body_field).await
            {
                Ok((path, message)) => {
                    set_submitting.set(false);
                    set_is_open.set(false);
                    let badge_text = if !path.trim().is_empty() {
                        format!("Memory saved: {}", path.trim())
                    } else if !message.trim().is_empty() {
                        message
                    } else {
                        format!("Memory saved: {name_for_badge}.md")
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
        let _ = (
            sid,
            name_field,
            description_field,
            type_field,
            body_field,
            set_badge_msg,
            set_is_open,
        );
        set_submitting.set(false);
    }
}

// =========================================================================
// trace:STORY-14 | ai:claude
//
// Tool-call inspector UI. Clicking a tool badge expands an inline
// <ToolCallPanel> below the message that shows the full tool input
// (pretty-printed JSON), the full output, the duration, and the
// success/failure status. The chat shell stays put; nothing is
// modal/overlay.
//
// `ChatTurn.tool_calls` is `Vec<ToolCall>` from the SSE event + history
// endpoint directly (Codex's STORY-14 backend PR landed first). The
// frontend consumes it without any per-tool conversion.
// =========================================================================

/// Format a duration in milliseconds: under a second → integer ms
/// (`42 ms`), at-or-above a second → one-decimal s (`1.2 s`).
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        let secs = ms as f64 / 1000.0;
        format!("{secs:.1} s")
    }
}

/// Reusable tool-call strip: renders a row of clickable badges, plus
/// (below) inline `<ToolCallPanel>`s for whichever badges the operator
/// has expanded. Same component drives both the live SSE stream and
/// the historical-turn render path.
///
/// The `expanded` set is component-local, so each TurnView (and the
/// live streaming turn) gets its own independent panel state. Multiple
/// panels can be open simultaneously — operator may want to compare
/// `aida_list` output to `find_traces` output side by side.
#[component]
fn ToolStrip(
    /// Reactive source of tool calls. Live render passes a streaming
    /// `live_tools.into()`; historical render derives from the stored
    /// turn vector.
    tool_calls: Signal<Vec<ToolCall>>,
) -> impl IntoView {
    use std::collections::HashSet;
    let (expanded, set_expanded) = signal::<HashSet<usize>>(HashSet::new());

    view! {
        <div class="tools">
            {move || {
                tool_calls.get().into_iter().enumerate().map(|(i, tc)| {
                    let name = tc.name.clone();
                    let preview = tc.input.to_string();
                    let ok = tc.ok;
                    view! {
                        <button
                            type="button"
                            class=move || {
                                let active = if expanded.get().contains(&i) { " active" } else { "" };
                                let status = if ok { "ok" } else { "err" };
                                format!("tool-badge {status}{active}")
                            }
                            title=preview
                            on:click=move |_| {
                                set_expanded.update(|s| {
                                    if !s.insert(i) {
                                        s.remove(&i);
                                    }
                                });
                            }
                        >
                            <span class="tool-name">{name}</span>
                        </button>
                    }
                }).collect_view()
            }}
        </div>
        <div class="tool-panels">
            {move || {
                tool_calls.get().into_iter().enumerate().map(|(i, tc)| {
                    let call_sv = StoredValue::new(tc.clone());
                    view! {
                        <Show when=move || expanded.get().contains(&i)>
                            <ToolCallPanel call=call_sv.get_value()/>
                        </Show>
                    }
                }).collect_view()
            }}
        </div>
    }
}

/// Read-only inspector for a single tool call. No truncation — the
/// `<pre>` blocks scroll if they exceed the panel's max-height.
#[component]
fn ToolCallPanel(call: ToolCall) -> impl IntoView {
    let duration_str = format_duration_ms(call.duration_ms);
    let status_class = if call.ok { "ok" } else { "err" };
    let status_text = if call.ok { "ok" } else { "failed" };
    let input_pretty = serde_json::to_string_pretty(&call.input)
        .unwrap_or_else(|_| call.input.to_string());
    let output = call.output;
    let output_empty = output.trim().is_empty();

    view! {
        <div class="tool-call-panel">
            <div class="tool-panel-head">
                <span class="tool-panel-name">{call.name}</span>
                <span class=format!("tool-panel-status {status_class}")>{status_text}</span>
                <span class="tool-panel-duration">{duration_str}</span>
            </div>
            <div class="tool-panel-section">
                <div class="tool-panel-label">"Input"</div>
                <pre class="tool-panel-pre">{input_pretty}</pre>
            </div>
            <div class="tool-panel-section">
                <div class="tool-panel-label">"Output"</div>
                {
                    if output_empty {
                        view! { <pre class="tool-panel-pre muted">"(no output)"</pre> }.into_any()
                    } else {
                        view! { <pre class="tool-panel-pre">{output}</pre> }.into_any()
                    }
                }
            </div>
        </div>
    }
}

/// Sample tool-call fixture for the inspector. Only intended for
/// manual demo / screenshot purposes; not wired into production
/// rendering. Drops at integration with Codex's backend PR.
#[cfg(any(test, debug_assertions))]
#[allow(dead_code)]
pub fn dev_fixture_tool_calls() -> Vec<ToolCall> {
    vec![
        ToolCall {
            name: "aida_list".into(),
            input: serde_json::json!({"status": "in-progress"}),
            output: "Found 3 requirements:\n- [EPIC-16] Evolve aida-chat into first-proof-of-concept aida-consumer (Status: Approved)\n- [STORY-14] Tool-call inspector (Status: In Progress)\n- [TASK-501] Wire the killer demo (Status: Draft)".into(),
            duration_ms: 42,
            ok: true,
        },
        ToolCall {
            name: "find_traces".into(),
            input: serde_json::json!({"spec_id": "EPIC-16"}),
            output: "./src/server/mcp/client.rs:1:// trace:EPIC-16 | ai:claude\n./src/server/mcp/mod.rs:1:// trace:EPIC-16 | ai:claude\n./src/server/mcp/protocol.rs:1:// trace:EPIC-16 | ai:claude\n./src/server/tools/aida.rs:1:// trace:STORY-4 EPIC-16 | ai:claude\n./src/server/tools/traces.rs:1:// trace:EPIC-16 | ai:claude".into(),
            duration_ms: 1234,
            ok: true,
        },
        ToolCall {
            name: "read_file".into(),
            input: serde_json::json!({"path": "nonexistent.rs"}),
            output: "error: file not found within repo root".into(),
            duration_ms: 8,
            ok: false,
        },
    ]
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

    // trace:STORY-22 | ai:claude
    use super::extract_first_sentence_title;

    #[test]
    fn title_heuristic_takes_first_sentence() {
        assert_eq!(
            extract_first_sentence_title("Fix the login bug. There's more context after.", 80),
            "Fix the login bug"
        );
        assert_eq!(
            extract_first_sentence_title("How do I add auth?\nLong follow-up here.", 80),
            "How do I add auth"
        );
        assert_eq!(
            extract_first_sentence_title("Done!\nMore details.", 80),
            "Done"
        );
        // Newline before any terminator → first line.
        assert_eq!(
            extract_first_sentence_title("first line\nsecond line", 80),
            "first line"
        );
    }

    #[test]
    fn title_heuristic_returns_whole_body_when_no_terminator() {
        assert_eq!(
            extract_first_sentence_title("Just a fragment with no terminator", 80),
            "Just a fragment with no terminator"
        );
        assert_eq!(extract_first_sentence_title("", 80), "");
        assert_eq!(extract_first_sentence_title("   ", 80), "");
    }

    #[test]
    fn title_heuristic_caps_at_max_len_with_word_boundary() {
        // Sentence longer than max_len → truncate at a word boundary with `…`.
        let body = "This is a fairly long sentence that should be truncated near the cap";
        let out = extract_first_sentence_title(body, 30);
        assert!(out.ends_with('…'), "expected ellipsis on {out:?}");
        // Character count of the visible prefix (excluding `…`) should be ≤ 30.
        let visible = &out[..out.len() - "…".len()];
        assert!(
            visible.chars().count() <= 30,
            "{out:?} has {} visible chars",
            visible.chars().count()
        );
        // Truncated at a word boundary (no trailing partial word).
        assert!(
            !visible.ends_with(|c: char| c.is_ascii_alphanumeric())
                || visible.split_whitespace().count() >= 2,
            "should break on whitespace: {out:?}"
        );
    }

    #[test]
    fn title_heuristic_falls_back_to_hard_cut_for_unbroken_strings() {
        // No whitespace → can't break on a word boundary, so hard-cut.
        let body = "OneVeryLongUnbrokenStringThatExceedsTheLimit";
        let out = extract_first_sentence_title(body, 10);
        assert!(out.ends_with('…'), "{out:?}");
        let visible = &out[..out.len() - "…".len()];
        assert_eq!(visible.chars().count(), 10);
    }

    #[test]
    fn title_heuristic_strips_leading_whitespace_and_trims() {
        assert_eq!(
            extract_first_sentence_title("   Hello world.   ", 80),
            "Hello world"
        );
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

// trace:STORY-23 | ai:claude
#[cfg(test)]
mod memory_name_tests {
    use super::is_valid_memory_name;

    #[test]
    fn accepts_valid_kebab_slugs() {
        for s in [
            "a",
            "foo",
            "foo-bar",
            "foo_bar",
            "abc123",
            "login-edge-case",
            "x-y-z-0-1-2",
            "a_b_c",
        ] {
            assert!(is_valid_memory_name(s), "{s:?} should be valid");
        }
    }

    #[test]
    fn rejects_uppercase_and_bad_first_char() {
        for s in [
            "",         // empty
            "Foo",      // uppercase first
            "fooBar",   // uppercase in middle
            "1foo",     // digit start
            "-foo",     // hyphen start
            "_foo",     // underscore start
            " foo",     // whitespace start
        ] {
            assert!(!is_valid_memory_name(s), "{s:?} should be invalid");
        }
    }

    #[test]
    fn rejects_invalid_chars() {
        for s in [
            "foo/bar",  // slash
            "foo.bar",  // dot
            "..foo",    // dot-leading
            "foo bar",  // space
            "foo@bar",  // at-sign
            "foo:bar",  // colon
        ] {
            assert!(!is_valid_memory_name(s), "{s:?} should be invalid");
        }
    }

    #[test]
    fn enforces_80_char_cap() {
        let exactly_80: String = std::iter::once('a').chain(std::iter::repeat_n('b', 79)).collect();
        assert_eq!(exactly_80.len(), 80);
        assert!(is_valid_memory_name(&exactly_80));
        let over_80: String = std::iter::once('a').chain(std::iter::repeat_n('b', 80)).collect();
        assert_eq!(over_80.len(), 81);
        assert!(!is_valid_memory_name(&over_80));
    }
}

// trace:STORY-14 | ai:claude
#[cfg(test)]
mod tool_inspector_tests {
    use super::*;

    #[test]
    fn format_duration_under_one_second() {
        assert_eq!(format_duration_ms(0), "0 ms");
        assert_eq!(format_duration_ms(1), "1 ms");
        assert_eq!(format_duration_ms(42), "42 ms");
        assert_eq!(format_duration_ms(999), "999 ms");
    }

    #[test]
    fn format_duration_one_second_boundary() {
        // Brief: <1000 → ms; >=1000 → "1.2 s" (one decimal).
        assert_eq!(format_duration_ms(1000), "1.0 s");
        assert_eq!(format_duration_ms(1500), "1.5 s");
    }

    #[test]
    fn format_duration_long() {
        assert_eq!(format_duration_ms(60_000), "60.0 s");
        assert_eq!(format_duration_ms(123_456), "123.5 s");
    }

    #[test]
    fn json_pretty_print_smoke() {
        // The panel uses serde_json::to_string_pretty directly; this
        // pins the expected shape so a serde_json upgrade can't silently
        // collapse the pretty output to one line.
        let v = serde_json::json!({"path": "src/lib.rs", "limit": 200});
        let pretty = serde_json::to_string_pretty(&v).unwrap();
        assert!(pretty.contains('\n'), "expected newlines in {pretty:?}");
        assert!(pretty.contains("  "), "expected indentation in {pretty:?}");
        assert!(pretty.contains(r#""path""#));
        assert!(pretty.contains(r#""src/lib.rs""#));
    }

    #[test]
    fn dev_fixture_renders_three_distinct_calls() {
        let calls = dev_fixture_tool_calls();
        assert_eq!(calls.len(), 3);
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"aida_list"));
        assert!(names.contains(&"find_traces"));
        assert!(names.contains(&"read_file"));
        // At least one should be failed (read_file in the fixture) so
        // the panel's `failed` status branch gets exercised visually.
        assert!(calls.iter().any(|c| !c.ok));
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

// trace:STORY-22 | ai:claude
//
// POST /api/sessions/{session_id}/spec with {type, title, description}.
// Returns (spec_id, message) on Ok; user-facing error string on Err.
// Matches the contract:
//   200 {"ok": true, "spec_id": "BUG-378", "message": "..."}
//   400|500 {"ok": false, "error": "..."}
#[cfg(feature = "hydrate")]
async fn post_spec(
    session_id: &str,
    spec_type: &str,
    title: &str,
    description: &str,
) -> Result<(String, String), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

    let body = serde_json::to_string(&crate::messages::SpecRequest {
        r#type: spec_type.to_string(),
        title: title.to_string(),
        description: description.to_string(),
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

    let url = format!("/api/sessions/{session_id}/spec");
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
    if let Ok(parsed) = serde_json::from_str::<SpecResponse>(&body_text) {
        if parsed.ok {
            return Ok((
                parsed.spec_id.unwrap_or_default(),
                parsed.message.unwrap_or_default(),
            ));
        }
        return Err(parsed.error.unwrap_or_else(|| format!("HTTP {status}")));
    }
    if (200..300).contains(&status) {
        Ok((String::new(), body_text))
    } else {
        Err(format!("HTTP {status}: {}", body_text.trim()))
    }
}

// trace:STORY-23 | ai:claude
//
// POST /api/sessions/{session_id}/memory with {name, description,
// type, body}. Returns (path, message) on Ok; user-facing error
// string on Err. Matches the contract:
//   200 {"ok": true, "path": "/…/memory/foo.md", "message": "..."}
//   400|500 {"ok": false, "error": "..."}
#[cfg(feature = "hydrate")]
async fn post_memory(
    session_id: &str,
    name: &str,
    description: &str,
    mem_type: &str,
    body: &str,
) -> Result<(String, String), String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response};

    let payload = serde_json::to_string(&crate::messages::MemoryRequest {
        name: name.to_string(),
        description: description.to_string(),
        r#type: mem_type.to_string(),
        body: body.to_string(),
    })
    .map_err(|e| format!("encode: {e}"))?;
    let headers = Headers::new().map_err(|e| format!("headers: {e:?}"))?;
    headers
        .set("content-type", "application/json")
        .map_err(|e| format!("set header: {e:?}"))?;

    let opts = RequestInit::new();
    opts.set_method("POST");
    opts.set_headers(&headers);
    opts.set_body(&wasm_bindgen::JsValue::from_str(&payload));

    let url = format!("/api/sessions/{session_id}/memory");
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
    if let Ok(parsed) = serde_json::from_str::<MemoryResponse>(&body_text) {
        if parsed.ok {
            return Ok((
                parsed.path.unwrap_or_default(),
                parsed.message.unwrap_or_default(),
            ));
        }
        return Err(parsed.error.unwrap_or_else(|| format!("HTTP {status}")));
    }
    if (200..300).contains(&status) {
        Ok((String::new(), body_text))
    } else {
        Err(format!("HTTP {status}: {}", body_text.trim()))
    }
}

#[cfg(feature = "hydrate")]
fn stream_chat(
    session_id: String,
    user_text: String,
    set_live_text: WriteSignal<String>,
    set_live_tools: WriteSignal<Vec<ToolCall>>,
    // trace:EPIC-29 | ai:claude
    set_live_charts: WriteSignal<Vec<ChartArtifact>>,
    on_done: impl Fn(String, Vec<ToolCall>, Vec<ChartArtifact>) + 'static,
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
    let tools: Rc<RefCell<Vec<ToolCall>>> = Rc::new(RefCell::new(vec![]));
    let charts: Rc<RefCell<Vec<ChartArtifact>>> = Rc::new(RefCell::new(vec![]));
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
                if let Ok(tc) = serde_json::from_str::<ToolCall>(&s) {
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

    // trace:EPIC-29 | ai:claude
    // chart event: a chart_* tool produced an SVG artifact.
    {
        let charts = charts.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
            if let Some(s) = ev.data().as_string() {
                if let Ok(art) = serde_json::from_str::<ChartArtifact>(&s) {
                    let mut c = charts.borrow_mut();
                    c.push(art);
                    set_live_charts.set(c.clone());
                }
            }
        });
        es.add_event_listener_with_callback("chart", cb.as_ref().unchecked_ref())
            .ok();
        cb.forget();
    }

    // done event: agent finished cleanly
    {
        let es_holder = es_holder.clone();
        let accumulated = accumulated.clone();
        let tools = tools.clone();
        let charts = charts.clone();
        let on_done = on_done.clone();
        let cb = Closure::<dyn FnMut(MessageEvent)>::new(move |_ev: MessageEvent| {
            if let Some(es) = es_holder.borrow_mut().take() {
                es.close();
            }
            let text = accumulated.borrow().clone();
            let tc = tools.borrow().clone();
            let arts = charts.borrow().clone();
            on_done(text, tc, arts);
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
