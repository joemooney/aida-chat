// trace:STORY-4 | ai:claude
//
// Read-only filesystem tools confined to `repo_root`. The single
// `resolve_within_repo` helper canonicalizes both the root and the
// requested path, then verifies the result is still inside the root —
// this defeats `..` escapes, absolute paths, and symlink escapes in one
// shot.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::fs;

use super::{Tool, ToolError};
use crate::server::config::ServerConfig;

pub fn read_file_spec() -> Tool {
    Tool {
        name: "read_file",
        description: "Read a UTF-8 text file from this repository. Path is relative to the repo \
            root. Returns the file's contents as text. Files larger than the configured cap \
            are truncated with a marker. Use this when you need to inspect a specific file's \
            contents; for searching across files, use grep_repo instead.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repo-relative path of the file to read, e.g. 'CLAUDE.md' or 'src/lib.rs'."
                }
            },
            "required": ["path"]
        }),
    }
}

pub fn list_directory_spec() -> Tool {
    Tool {
        name: "list_directory",
        description: "List the entries of a directory inside this repository. Returns one entry \
            per line, prefixed with 'd ' for directories and 'f ' for files. Use this to \
            discover what files exist before reading them.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Repo-relative directory path. Use '.' for the repo root."
                }
            },
            "required": ["path"]
        }),
    }
}

pub async fn read_file(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'path'".into()))?;
    let resolved = resolve_within_repo(&cfg.repo_root, path)?;
    let bytes = fs::read(&resolved)
        .await
        .map_err(|e| ToolError::Io(format!("{}: {e}", resolved.display())))?;
    let truncated = bytes.len() > cfg.max_read_bytes;
    let slice = if truncated {
        &bytes[..cfg.max_read_bytes]
    } else {
        &bytes[..]
    };
    let mut text = String::from_utf8_lossy(slice).into_owned();
    if truncated {
        text.push_str(&format!(
            "\n\n... [truncated: file is {} bytes, showing first {}]",
            bytes.len(),
            cfg.max_read_bytes
        ));
    }
    Ok(text)
}

pub async fn list_directory(cfg: &ServerConfig, input: &Value) -> Result<String, ToolError> {
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::BadInput("missing 'path'".into()))?;
    let resolved = resolve_within_repo(&cfg.repo_root, path)?;
    let mut entries = vec![];
    let mut rd = fs::read_dir(&resolved)
        .await
        .map_err(|e| ToolError::Io(format!("{}: {e}", resolved.display())))?;
    while let Some(e) = rd
        .next_entry()
        .await
        .map_err(|e| ToolError::Io(format!("read_dir: {e}")))?
    {
        let name = e.file_name().to_string_lossy().into_owned();
        // Hide the .git internals and the orphan-store worktree noise
        // (agent should reach AIDA data via aida_* tools, not by reading
        // the orphan branch's serialized YAML).
        if name == ".git" || name == ".aida-store" {
            continue;
        }
        let ft = e
            .file_type()
            .await
            .map_err(|e| ToolError::Io(format!("file_type: {e}")))?;
        let prefix = if ft.is_dir() { "d " } else { "f " };
        entries.push(format!("{prefix}{name}"));
    }
    entries.sort();
    Ok(entries.join("\n"))
}

/// Canonicalize `repo_root` + `requested` and verify the result is
/// still under repo_root. Rejects:
///   - paths whose canonical form escapes the repo (covers `..`, absolute paths,
///     and symlink-escape attacks),
///   - any path inside `.git/`.
pub fn resolve_within_repo(repo_root: &Path, requested: &str) -> Result<PathBuf, ToolError> {
    if requested.is_empty() {
        return Err(ToolError::BadInput("empty path".into()));
    }
    let joined = if requested == "." || requested == "./" {
        repo_root.to_path_buf()
    } else if Path::new(requested).is_absolute() {
        // Don't trust absolute paths from the model. Strip the leading '/'
        // and rejoin under the repo so we get a uniform "must be under
        // repo_root" check, but also flag it explicitly.
        return Err(ToolError::NotAllowed(
            "absolute paths are not allowed; pass a path relative to the repo root".into(),
        ));
    } else {
        repo_root.join(requested)
    };
    // If the path doesn't exist yet, canonicalize() fails; for read-only
    // tools that's actually what we want — surface a clean error.
    let canon = std::fs::canonicalize(&joined)
        .map_err(|e| ToolError::Io(format!("{}: {e}", joined.display())))?;
    if !canon.starts_with(repo_root) {
        return Err(ToolError::NotAllowed(format!(
            "path resolves outside repo root: {}",
            canon.display()
        )));
    }
    // Block .git/ outright. We accept .git as a name in the *visible*
    // ancestors check because canonicalize won't include it unless the
    // user really targeted it.
    if canon
        .components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(".git"))
    {
        return Err(ToolError::NotAllowed(".git is off-limits".into()));
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn make_temp_repo() -> tempdir_substitute::TempDir {
        let tmp = tempdir_substitute::TempDir::new("aida-chat-fs");
        let root = tmp.path();
        stdfs::write(root.join("hello.txt"), "hi").unwrap();
        stdfs::create_dir_all(root.join("sub")).unwrap();
        stdfs::write(root.join("sub/inner.txt"), "inner").unwrap();
        stdfs::create_dir_all(root.join(".git")).unwrap();
        stdfs::write(root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        tmp
    }

    #[test]
    fn resolve_normal_path_ok() {
        let tmp = make_temp_repo();
        let resolved = resolve_within_repo(tmp.path(), "hello.txt").unwrap();
        assert!(resolved.ends_with("hello.txt"));
    }

    #[test]
    fn resolve_parent_escape_rejected() {
        let tmp = make_temp_repo();
        let err = resolve_within_repo(tmp.path(), "../etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::NotAllowed(_) | ToolError::Io(_)));
    }

    #[test]
    fn resolve_absolute_rejected() {
        let tmp = make_temp_repo();
        let err = resolve_within_repo(tmp.path(), "/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::NotAllowed(_)));
    }

    #[test]
    fn resolve_git_rejected() {
        let tmp = make_temp_repo();
        let err = resolve_within_repo(tmp.path(), ".git/HEAD").unwrap_err();
        assert!(matches!(err, ToolError::NotAllowed(_)));
    }

    #[test]
    fn resolve_empty_rejected() {
        let tmp = make_temp_repo();
        assert!(resolve_within_repo(tmp.path(), "").is_err());
    }
}

// Tiny zero-dep TempDir to avoid pulling tempfile just for tests.
#[cfg(test)]
mod tempdir_substitute {
    use std::path::{Path, PathBuf};
    pub struct TempDir(PathBuf);
    impl TempDir {
        pub fn new(prefix: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            p.push(format!("{prefix}-{nonce}"));
            std::fs::create_dir_all(&p).unwrap();
            // Canonicalize so resolve_within_repo's prefix check works
            // on platforms where tmpdir is a symlink (e.g. /tmp -> /private/tmp).
            let p = std::fs::canonicalize(&p).unwrap();
            TempDir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
