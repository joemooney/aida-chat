use aida_chat::server::config::{Backend, ServerConfig};
use aida_chat::server::tools::traces::find_traces;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let cfg = ServerConfig {
        backend: Backend::Anthropic,
        anthropic_api_key: Some("x".into()),
        model: "x".into(),
        repo_root: std::env::current_dir().unwrap(),
        max_tool_iterations: 1,
        max_output_tokens: 1,
        max_read_bytes: 1,
        session_ttl: Duration::from_secs(1),
        mcp_command: PathBuf::from("aida"),
        mcp_args: vec!["mcp-serve".into()],
    };
    for id in &["EPIC-1", "EPIC-16", "STORY-3", "STORY-15"] {
        let out = find_traces(&cfg, &json!({"spec_id": id})).await.unwrap();
        println!("--- {id}:\n{out}\n");
    }
}
