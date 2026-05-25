// trace:STORY-25 | ai:codex
//
// Focused trace-drift verification. This is deliberately not a chat-agent
// turn: the operator asks for one SPEC-ID, we gather bounded code context
// around each trace comment, and a small Anthropic classifier judges whether
// that trace site still matches the current requirement text.

use std::future::Future;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{aida, fs, traces, Tool, ToolError};
use crate::messages::{TraceFinding, VerifyDriftResponse};
use crate::server::config::ServerConfig;

const TRACE_SITE_LIMIT: usize = 10;
const CONTEXT_LINES: u64 = 20;
const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSite {
    pub path: String,
    pub line: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceVerdict {
    pub aligned: bool,
    pub severity: String,
    pub reason: String,
}

pub fn verify_trace_drift_spec() -> Tool {
    Tool {
        name: "verify_trace_drift",
        description: "For a SPEC-ID, inspect every trace comment that references it and classify \
            whether the surrounding code still implements the requirement. V1 checks at most 10 \
            trace sites per invocation and reports when the result is truncated.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "spec_id": {
                    "type": "string",
                    "description": "SPEC-ID such as STORY-3 or EPIC-16"
                }
            },
            "required": ["spec_id"]
        }),
    }
}

pub async fn verify_trace_drift(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let spec_id = input
        .get("spec_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'spec_id'".into()))?;
    let response = verify_trace_drift_for_spec(cfg, spec_id).await?;
    serde_json::to_string_pretty(&response)
        .map_err(|e| ToolError::Execution(format!("encode drift response: {e}")))
}

pub async fn verify_trace_drift_for_spec(
    cfg: &ServerConfig,
    spec_id: &str,
) -> Result<VerifyDriftResponse, ToolError> {
    validate_spec_id(spec_id)?;
    let spec_body = aida::aida_show(cfg, &json!({ "id": spec_id })).await?;
    let trace_output = traces::find_traces(cfg, &json!({ "spec_id": spec_id })).await?;
    let sites = parse_trace_sites(&trace_output);
    let (limited, truncated) = limit_trace_sites(sites);

    if limited.is_empty() {
        return Ok(VerifyDriftResponse {
            ok: true,
            findings: vec![],
            truncated: false,
            message: Some(format!("No trace comments found for {spec_id}")),
            error: None,
        });
    }

    aggregate_trace_findings(limited, truncated, |site| async {
        let snippet = read_trace_context(cfg, &site).await?;
        let verdict = classify_trace_site(cfg, spec_id, &spec_body, &site, &snippet).await?;
        Ok(finding_from_verdict(site, verdict))
    })
    .await
}

pub async fn aggregate_trace_findings<F, Fut>(
    sites: Vec<TraceSite>,
    truncated: bool,
    mut classify: F,
) -> Result<VerifyDriftResponse, ToolError>
where
    F: FnMut(TraceSite) -> Fut,
    Fut: Future<Output = Result<TraceFinding, ToolError>>,
{
    let mut findings = Vec::with_capacity(sites.len());
    for site in sites {
        findings.push(classify(site).await?);
    }
    let message = if truncated {
        Some(format!(
            "Checked first {TRACE_SITE_LIMIT} trace sites; additional sites were skipped."
        ))
    } else {
        Some(format!("Checked {} trace site(s).", findings.len()))
    };
    Ok(VerifyDriftResponse {
        ok: true,
        findings,
        truncated,
        message,
        error: None,
    })
}

pub fn parse_trace_sites(trace_output: &str) -> Vec<TraceSite> {
    trace_output
        .lines()
        .filter_map(|line| {
            if line.starts_with("(no trace comments") {
                return None;
            }
            let mut parts = line.splitn(3, ':');
            let path = parts.next()?.trim();
            let line_no = parts.next()?.trim().parse::<u64>().ok()?;
            if path.is_empty() {
                return None;
            }
            Some(TraceSite {
                path: path.trim_start_matches("./").to_string(),
                line: line_no,
            })
        })
        .collect()
}

pub fn limit_trace_sites(sites: Vec<TraceSite>) -> (Vec<TraceSite>, bool) {
    let truncated = sites.len() > TRACE_SITE_LIMIT;
    (
        sites.into_iter().take(TRACE_SITE_LIMIT).collect(),
        truncated,
    )
}

pub fn validate_spec_id(spec_id: &str) -> Result<(), ToolError> {
    let valid = !spec_id.is_empty()
        && spec_id.len() < 64
        && spec_id.contains('-')
        && spec_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(ToolError::BadInput(
            "spec_id does not look like a SPEC-ID".into(),
        ))
    }
}

async fn read_trace_context(cfg: &ServerConfig, site: &TraceSite) -> Result<String, ToolError> {
    let resolved = fs::resolve_within_repo(&cfg.repo_root, &site.path)?;
    let text = tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| ToolError::Io(format!("{}: {e}", resolved.display())))?;
    let start = site.line.saturating_sub(CONTEXT_LINES).max(1);
    let end = site.line.saturating_add(CONTEXT_LINES);
    let mut out = String::new();
    for (idx, line) in text.lines().enumerate() {
        let n = idx as u64 + 1;
        if n < start {
            continue;
        }
        if n > end {
            break;
        }
        out.push_str(&format!("{n:>5} | {line}\n"));
    }
    Ok(out)
}

async fn classify_trace_site(
    cfg: &ServerConfig,
    spec_id: &str,
    spec_body: &str,
    site: &TraceSite,
    snippet: &str,
) -> Result<TraceVerdict, ToolError> {
    let api_key = cfg.anthropic_api_key.as_deref().ok_or_else(|| {
        ToolError::Execution("ANTHROPIC_API_KEY is required for trace drift verification".into())
    })?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| ToolError::Execution(format!("reqwest client: {e}")))?;
    let body = json!({
        "model": cfg.model,
        "max_tokens": 500,
        "temperature": 0,
        "system": "You are a precise code-to-requirement drift classifier. Reply only with one minified JSON object matching {\"aligned\":bool,\"severity\":\"ok\"|\"minor\"|\"major\",\"reason\":\"short explanation\"}. No markdown. No prose. Keep reason under 160 characters. Use severity ok only when aligned is true.",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "text",
                "text": format!(
                    "Requirement {spec_id}:\n\n{spec_body}\n\nTrace site {}:{}:\n\n{}\n\nDoes this code still implement the requirement described by {spec_id}?",
                    site.path, site.line, snippet
                )
            }]
        }]
    });
    let resp = client
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_API_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::Execution(format!("anthropic drift request: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ToolError::Execution(format!(
            "anthropic drift classifier returned {status}: {body}"
        )));
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| ToolError::Execution(format!("decode anthropic drift response: {e}")))?;
    let text = value
        .get("content")
        .and_then(|v| v.as_array())
        .and_then(|items| items.iter().find_map(|item| item.get("text")?.as_str()))
        .ok_or_else(|| ToolError::Execution("anthropic drift response had no text".into()))?;
    parse_llm_verdict(text)
}

pub fn parse_llm_verdict(text: &str) -> Result<TraceVerdict, ToolError> {
    let trimmed = text.trim();
    let json_text = if trimmed.starts_with('{') {
        trimmed
    } else {
        let start = trimmed.find('{').ok_or_else(|| {
            ToolError::Execution(format!("classifier returned no JSON: {trimmed}"))
        })?;
        let end = trimmed.rfind('}').ok_or_else(|| {
            ToolError::Execution(format!("classifier returned incomplete JSON: {trimmed}"))
        })?;
        &trimmed[start..=end]
    };
    #[derive(Deserialize)]
    struct RawVerdict {
        aligned: bool,
        severity: String,
        reason: String,
    }
    let raw: RawVerdict = serde_json::from_str(json_text)
        .map_err(|e| ToolError::Execution(format!("parse classifier JSON: {e}: {json_text}")))?;
    let severity = raw.severity.trim().to_ascii_lowercase();
    if !matches!(severity.as_str(), "ok" | "minor" | "major") {
        return Err(ToolError::Execution(format!(
            "classifier returned invalid severity: {}",
            raw.severity
        )));
    }
    Ok(TraceVerdict {
        aligned: raw.aligned,
        severity,
        reason: raw.reason.trim().to_string(),
    })
}

pub fn finding_from_verdict(site: TraceSite, verdict: TraceVerdict) -> TraceFinding {
    TraceFinding {
        path: site.path,
        line: site.line,
        aligned: verdict.aligned,
        severity: verdict.severity,
        reason: verdict.reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_spec_id() {
        assert!(validate_spec_id("STORY-25").is_ok());
        assert!(matches!(
            validate_spec_id("story25"),
            Err(ToolError::BadInput(_))
        ));
        assert!(matches!(
            validate_spec_id("STORY 25"),
            Err(ToolError::BadInput(_))
        ));
    }

    #[test]
    fn parses_trace_sites_from_find_traces_output() {
        let out = "./src/app.rs:42:// trace:STORY-25 | ai:codex\nsrc/lib.rs:7:// trace:STORY-25 STORY-3 | ai:claude\n";
        let sites = parse_trace_sites(out);
        assert_eq!(
            sites,
            vec![
                TraceSite {
                    path: "src/app.rs".into(),
                    line: 42
                },
                TraceSite {
                    path: "src/lib.rs".into(),
                    line: 7
                }
            ]
        );
    }

    #[test]
    fn truncates_after_ten_sites() {
        let sites = (1..=12)
            .map(|line| TraceSite {
                path: "src/app.rs".into(),
                line,
            })
            .collect();
        let (limited, truncated) = limit_trace_sites(sites);
        assert!(truncated);
        assert_eq!(limited.len(), 10);
        assert_eq!(limited[9].line, 10);
    }

    #[tokio::test]
    async fn aggregates_mocked_classifier_findings() {
        let sites = vec![
            TraceSite {
                path: "src/a.rs".into(),
                line: 1,
            },
            TraceSite {
                path: "src/b.rs".into(),
                line: 2,
            },
        ];
        let response = aggregate_trace_findings(sites, false, |site| async move {
            Ok(finding_from_verdict(
                site,
                TraceVerdict {
                    aligned: false,
                    severity: "minor".into(),
                    reason: "mock drift".into(),
                },
            ))
        })
        .await
        .unwrap();
        assert!(response.ok);
        assert!(!response.truncated);
        assert_eq!(response.findings.len(), 2);
        assert_eq!(response.findings[0].reason, "mock drift");
    }

    #[test]
    fn parses_classifier_json_with_surrounding_text() {
        let verdict = parse_llm_verdict(
            "```json\n{\"aligned\":false,\"severity\":\"major\",\"reason\":\"handler changed\"}\n```",
        )
        .unwrap();
        assert!(!verdict.aligned);
        assert_eq!(verdict.severity, "major");
        assert_eq!(verdict.reason, "handler changed");
    }
}
