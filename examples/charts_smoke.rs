// trace:EPIC-29 | ai:codex

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use std::sync::Arc;

    use aida_chat::server::config::{Backend, ServerConfig};
    use aida_chat::server::tools::charts::{
        chart_feature, chart_sprint, chart_status, extract_chart_artifact,
    };
    use serde_json::json;

    let repo_root = std::env::var("AIDA_CHAT_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let repo_root = std::fs::canonicalize(repo_root)?;
    let out_dir = std::env::var("AIDA_CHART_SMOKE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join("target/charts_smoke"));
    std::fs::create_dir_all(&out_dir)?;

    let cfg = Arc::new(ServerConfig {
        backend: Backend::Anthropic,
        anthropic_api_key: Some("smoke".into()),
        model: "smoke".into(),
        repo_root,
        max_tool_iterations: 1,
        max_output_tokens: 1024,
        max_read_bytes: 256 * 1024,
        session_ttl: std::time::Duration::from_secs(60),
        mcp_command: PathBuf::from("aida"),
        mcp_args: vec!["mcp-serve".into()],
    });

    for (name, output) in [
        ("status", chart_status(&cfg, &json!({})).await?),
        ("sprint", chart_sprint(&cfg, &json!({})).await?),
        ("feature", chart_feature(&cfg, &json!({})).await?),
    ] {
        let artifact = extract_chart_artifact(&output)
            .ok_or_else(|| format!("{name} did not return a chart artifact"))?;
        let path = out_dir.join(format!("{name}.svg"));
        std::fs::write(&path, artifact.svg)?;
        println!(
            "{name}: {} ({}) -> {}",
            artifact.title,
            artifact.summary,
            path.display()
        );
    }

    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    eprintln!("charts_smoke requires --features ssr");
    std::process::exit(1);
}
