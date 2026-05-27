// trace:STORY-23 | ai:codex
//
// Memory write tool. This is the one deliberate exception to the
// repo_root confinement used by the read-only file tools: writes are
// confined to ~/.claude/projects/<slug>/memory/, where <slug> is derived
// from cfg.repo_root and never from model/user input.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;

const MAX_NAME_CHARS: usize = 80;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_BODY_BYTES: usize = 16 * 1024;

pub fn write_memory_spec() -> Tool {
    Tool {
        name: "write_memory",
        description: "Save a pattern, principle, or correction worth preserving across sessions. \
            Files land in ~/.claude/projects/<slug>/memory/ and are picked up by BOTH Claude Code \
            and aida-chat next session (shared, not isolated). Use sparingly — only when the user \
            signals 'remember this' OR the discussion produced a generalizable principle.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "kebab-slug, max 80 chars, no path separators; .md is added automatically"
                },
                "description": {
                    "type": "string",
                    "description": "one-line description, max 200 chars"
                },
                "type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"]
                },
                "body": {
                    "type": "string",
                    "description": "markdown body, max 16 KiB"
                }
            },
            "required": ["name", "description", "type", "body"]
        }),
    }
}

pub async fn write_memory(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let name = input
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'name'".into()))?;
    let description = input
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'description'".into()))?;
    let memory_type = input
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'type'".into()))?;
    let body = input
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'body'".into()))?;

    validate_memory_input(name, description, memory_type, body)?;

    let memory_dir = memory_dir_for_repo(&cfg.repo_root)?;
    std::fs::create_dir_all(&memory_dir)
        .map_err(|e| ToolError::Io(format!("mkdir {}: {e}", memory_dir.display())))?;
    let final_path =
        resolve_within_memory_dir(&memory_dir, &memory_dir.join(format!("{name}.md")))?;
    if final_path.symlink_metadata().is_ok() {
        return Err(ToolError::BadInput(format!(
            "memory '{name}' already exists — pick a different name"
        )));
    }

    let doc = render_memory_file(name, description, memory_type, body);
    atomic_write(&final_path, &doc)?;
    update_memory_index(&memory_dir, name, description)?;

    Ok(final_path.display().to_string())
}

pub fn project_slug(repo_root: &Path) -> Result<String, ToolError> {
    let canon = std::fs::canonicalize(repo_root)
        .map_err(|e| ToolError::Io(format!("canonicalize {}: {e}", repo_root.display())))?;
    Ok(canon.display().to_string().replace('/', "-"))
}

fn memory_dir_for_repo(repo_root: &Path) -> Result<PathBuf, ToolError> {
    let home = std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::Execution("HOME is not set".into()))?;
    Ok(PathBuf::from(home)
        .join(".claude")
        .join("projects")
        .join(project_slug(repo_root)?)
        .join("memory"))
}

fn validate_memory_input(
    name: &str,
    description: &str,
    memory_type: &str,
    body: &str,
) -> Result<(), ToolError> {
    validate_memory_name(name)?;
    if description.trim().is_empty() {
        return Err(ToolError::BadInput("description may not be empty".into()));
    }
    if description.chars().any(|c| c == '\n' || c == '\r') {
        return Err(ToolError::BadInput(
            "description must be a single line".into(),
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(ToolError::BadInput(
            "description may not exceed 200 characters".into(),
        ));
    }
    if !matches!(memory_type, "user" | "feedback" | "project" | "reference") {
        return Err(ToolError::BadInput(format!(
            "type must be one of user, feedback, project, reference: {memory_type}"
        )));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ToolError::BadInput("body may not exceed 16 KiB".into()));
    }
    Ok(())
}

fn validate_memory_name(name: &str) -> Result<(), ToolError> {
    if name.is_empty() {
        return Err(ToolError::BadInput("name may not be empty".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ToolError::BadInput(
            "name may not exceed 80 characters".into(),
        ));
    }
    if name.ends_with(".md") {
        return Err(ToolError::BadInput(
            "name must not include the .md extension".into(),
        ));
    }
    if name.starts_with('.') || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(ToolError::BadInput(
            "name may not contain path separators, '..', or a leading '.'".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ToolError::BadInput(
            "name may only contain ASCII letters, numbers, '_' and '-'".into(),
        ));
    }
    Ok(())
}

pub fn resolve_within_memory_dir(
    memory_dir: &Path,
    final_path: &Path,
) -> Result<PathBuf, ToolError> {
    let memory_dir = std::fs::canonicalize(memory_dir)
        .map_err(|e| ToolError::Io(format!("canonicalize {}: {e}", memory_dir.display())))?;
    if final_path.is_absolute() && !final_path.starts_with(&memory_dir) {
        return Err(ToolError::NotAllowed(format!(
            "memory path resolves outside memory dir: {}",
            final_path.display()
        )));
    }
    let candidate = if final_path.is_absolute() {
        final_path.to_path_buf()
    } else {
        memory_dir.join(final_path)
    };
    let candidate_exists = candidate.symlink_metadata().is_ok();
    let resolved = match std::fs::canonicalize(&candidate) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !candidate_exists => {
            let parent = candidate
                .parent()
                .ok_or_else(|| ToolError::NotAllowed("memory path has no parent".into()))?;
            let parent = std::fs::canonicalize(parent)
                .map_err(|e| ToolError::Io(format!("canonicalize {}: {e}", parent.display())))?;
            if parent != memory_dir {
                return Err(ToolError::NotAllowed(format!(
                    "memory path resolves outside memory dir: {}",
                    candidate.display()
                )));
            }
            candidate
        }
        Err(e) => return Err(ToolError::Io(format!("{}: {e}", candidate.display()))),
    };
    if !resolved.starts_with(&memory_dir) {
        return Err(ToolError::NotAllowed(format!(
            "memory path resolves outside memory dir: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn render_memory_file(name: &str, description: &str, memory_type: &str, body: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nmetadata:\n  type: {memory_type}\n---\n\n{body}\n"
    )
}

fn update_memory_index(memory_dir: &Path, name: &str, description: &str) -> Result<(), ToolError> {
    let index_path = memory_dir.join("MEMORY.md");
    let entry = format!("- [{name}]({name}.md) — {description}");
    let old = match std::fs::read_to_string(&index_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ToolError::Io(format!("{}: {e}", index_path.display()))),
    };
    let needle = format!("[{name}]({name}.md)");
    let mut replaced = false;
    let mut lines: Vec<String> = old
        .lines()
        .map(|line| {
            if line.contains(&needle) {
                replaced = true;
                entry.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if !replaced {
        lines.push(entry);
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    atomic_write(&index_path, &out)
}

fn atomic_write(path: &Path, content: &str) -> Result<(), ToolError> {
    let parent = path
        .parent()
        .ok_or_else(|| ToolError::Io(format!("{} has no parent", path.display())))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ToolError::Io(format!("system clock: {e}")))?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ToolError::Io(format!("{} has no file name", path.display())))?;
    let tmp = parent.join(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
    std::fs::write(&tmp, content).map_err(|e| ToolError::Io(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ToolError::Io(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn slug_replaces_slashes() {
        let p = Path::new("/home/joe/ai/aida-chat");
        assert_eq!(
            project_slug(p).unwrap(),
            "-home-joe-ai-aida-chat"
        );
    }

    #[test]
    fn filename_validators() {
        for bad in ["foo/bar", "..", ".foo", "", "foo.md", "foo\\bar"] {
            assert!(validate_memory_name(bad).is_err(), "{bad} should fail");
        }
        assert!(validate_memory_name(&"x".repeat(81)).is_err());
        for good in ["foo", "foo-bar", "foo_bar", "Foo123"] {
            assert!(validate_memory_name(good).is_ok(), "{good} should pass");
        }
    }

    #[test]
    fn confinement_rejects_escapes_and_symlink() {
        let tmp = TempDir::new("aida-chat-memory");
        let memory_dir = tmp.path().join("memory");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::create_dir_all(&outside).unwrap();

        assert!(resolve_within_memory_dir(&memory_dir, Path::new("../escape.md")).is_err());
        assert!(resolve_within_memory_dir(&memory_dir, Path::new("/etc/passwd")).is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("target.md"), memory_dir.join("link.md"))
                .unwrap();
            assert!(resolve_within_memory_dir(&memory_dir, &memory_dir.join("link.md")).is_err());
        }
    }

    #[test]
    fn frontmatter_write_roundtrip() {
        let tmp = TempDir::new("aida-chat-memory-frontmatter");
        let dir = tmp.path();
        let path = dir.join("principle.md");
        let doc = render_memory_file("principle", "short description", "project", "## Body\nText");
        atomic_write(&path, &doc).unwrap();
        let got = fs::read_to_string(path).unwrap();
        assert!(got.starts_with("---\nname: principle\n"));
        assert!(got.contains("description: short description\n"));
        assert!(got.contains("metadata:\n  type: project\n"));
        assert!(got.ends_with("## Body\nText\n"));
    }

    #[test]
    fn memory_index_replaces_not_duplicates() {
        let tmp = TempDir::new("aida-chat-memory-index");
        let dir = tmp.path();
        update_memory_index(dir, "one", "first").unwrap();
        update_memory_index(dir, "one", "second").unwrap();
        let got = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert_eq!(got.matches("[one](one.md)").count(), 1);
        assert!(got.contains("- [one](one.md) — second"));
    }

    #[test]
    fn duplicate_file_rejected() {
        let tmp = TempDir::new("aida-chat-memory-duplicate");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let home = tmp.path().join("home");
        let slug = repo.display().to_string().replace('/', "-");
        let memory_dir = home
            .join(".claude")
            .join("projects")
            .join(slug)
            .join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("exists.md"), "already").unwrap();
        with_home(&home, || {
            let cfg = fixture_cfg(repo);
            let input = json!({
                "name": "exists",
                "description": "existing",
                "type": "project",
                "body": "body"
            });
            let err = tokio_test_block_on(write_memory(&cfg, &input)).unwrap_err();
            assert!(matches!(err, ToolError::BadInput(_)));
        });
    }

    fn fixture_cfg(repo_root: PathBuf) -> ServerConfig {
        ServerConfig {
            backend: crate::server::config::Backend::Anthropic,
            anthropic_api_key: None,
            model: "test".into(),
            repo_root,
            max_tool_iterations: 1,
            max_output_tokens: 1,
            max_read_bytes: 1024,
            session_ttl: std::time::Duration::from_secs(60),
            mcp_command: PathBuf::from("aida"),
            mcp_args: vec!["mcp-serve".into()],
        }
    }

    fn tokio_test_block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn with_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let out = f();
        if let Some(old) = old {
            std::env::set_var("HOME", old);
        } else {
            std::env::remove_var("HOME");
        }
        out
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!("{prefix}-{nonce}"));
            fs::create_dir_all(&p).unwrap();
            TempDir(fs::canonicalize(p).unwrap())
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
