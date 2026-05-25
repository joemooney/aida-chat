// trace:STORY-2 | ai:claude
//
// Server entrypoint: wires Leptos's SSR routes onto an Axum router, plus
// our chat API endpoints. The chat API holds the per-user session store
// in axum::extract::State.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use aida_chat::app::{shell, App};
    use aida_chat::server::api::router as api_router;
    use aida_chat::server::config::ServerConfig;
    use aida_chat::server::sessions::{InMemorySessions, SessionStore};
    use axum::Router;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use std::sync::Arc;

    // Load .env if present (so ANTHROPIC_API_KEY can live in a local file).
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = match ServerConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
    };
    let cfg = Arc::new(cfg);
    let sessions = Arc::new(InMemorySessions::new(cfg.clone()));
    // Periodic eviction of idle sessions.
    {
        let s = sessions.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                iv.tick().await;
                s.evict_idle().await;
            }
        });
    }

    let conf = get_configuration(None).unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    let app = Router::new()
        .leptos_routes(&leptos_options, routes, {
            let opts = leptos_options.clone();
            move || shell(opts.clone())
        })
        .nest("/api", api_router(sessions.clone(), cfg.clone()))
        .fallback(leptos_axum::file_and_error_handler(move |opts| {
            shell(opts)
        }))
        .with_state(leptos_options);

    println!("AIDA Chat running at http://{addr}");
    println!("Backend:             {}", cfg.backend.as_str());
    println!("Repo root scoped to: {}", cfg.repo_root.display());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}
