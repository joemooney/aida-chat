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
use crate::messages::ChatHistory;

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
                        .map(|turn| view! { <TurnView turn=turn/> })
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
fn TurnView(turn: ChatTurn) -> impl IntoView {
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
            let html = render_markdown(&text);
            view! { <div class="text markdown" inner_html=html/> }.into_any()
        }
    };
    view! {
        <div class=format!("turn {role_class}")>
            <div class="role">{role_label}</div>
            {tools_view}
            {body}
        </div>
    }
}

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
