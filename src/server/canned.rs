// trace:EPIC-26 | ai:codex
//
// Minimal canned-answer library. V1 intentionally supports only exact and
// case-insensitive substring matching; semantic matching belongs to a later
// EPIC-26 slice.

use std::path::Path;

use serde::Deserialize;

use crate::server::config::ServerConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct CannedLibrary {
    #[serde(default)]
    pub answers: Vec<CannedAnswer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CannedAnswer {
    #[serde(rename = "match")]
    pub matcher: String,
    pub strategy: MatchStrategy,
    pub answer: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchStrategy {
    Exact,
    Icontains,
}

impl CannedLibrary {
    pub fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join(".aida").join("canned-answers.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self { answers: vec![] }),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    pub fn match_query(&self, query: &str, cfg: &ServerConfig) -> Option<String> {
        self.answers
            .iter()
            .find(|answer| answer.matches(query))
            .map(|answer| interpolate(&answer.answer, cfg))
    }
}

impl CannedAnswer {
    fn matches(&self, query: &str) -> bool {
        let query = normalize(query);
        let needle = normalize(&self.matcher);
        match self.strategy {
            MatchStrategy::Exact => query == needle,
            MatchStrategy::Icontains => query.contains(&needle),
        }
    }
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

fn interpolate(answer: &str, cfg: &ServerConfig) -> String {
    answer
        .replace("{{backend}}", cfg.backend.as_str())
        .replace("{{repo_root}}", &cfg.repo_root.display().to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::server::config::{Backend, ServerConfig};

    #[test]
    fn exact_matches_case_insensitive_and_trimmed() {
        let answer = CannedAnswer {
            matcher: "what is aida-chat?".into(),
            strategy: MatchStrategy::Exact,
            answer: "ok".into(),
        };

        assert!(answer.matches("  WHAT IS AIDA-CHAT? "));
        assert!(!answer.matches("what is aida-chat? please"));
    }

    #[test]
    fn icontains_matches_case_insensitive_substrings() {
        let answer = CannedAnswer {
            matcher: "backend am i on".into(),
            strategy: MatchStrategy::Icontains,
            answer: "ok".into(),
        };

        assert!(answer.matches("What BACKEND am I on today?"));
        assert!(!answer.matches("what model is this"));
    }

    #[test]
    fn library_interpolates_backend() {
        let cfg = fixture_cfg();
        let library = CannedLibrary {
            answers: vec![CannedAnswer {
                matcher: "backend".into(),
                strategy: MatchStrategy::Exact,
                answer: "Backend: {{backend}}".into(),
            }],
        };

        assert_eq!(
            library.match_query(" backend ", &cfg).as_deref(),
            Some("Backend: anthropic-api")
        );
    }

    fn fixture_cfg() -> ServerConfig {
        ServerConfig {
            backend: Backend::Anthropic,
            anthropic_api_key: Some("test".into()),
            model: "test".into(),
            repo_root: PathBuf::from("/tmp/aida-chat-test"),
            max_tool_iterations: 1,
            max_output_tokens: 1,
            max_read_bytes: 1,
            session_ttl: std::time::Duration::from_secs(60),
            mcp_command: PathBuf::from("aida"),
            mcp_args: vec!["mcp-serve".into()],
        }
    }
}
