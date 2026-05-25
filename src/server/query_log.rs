// trace:EPIC-26 | ai:codex
//
// Per-project query log for fast-response routing analysis. This is local
// aida-chat runtime state, deliberately not an AIDA core entity.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRow {
    pub id: i64,
    pub session_id: String,
    pub ts: i64,
    pub query: String,
    pub latency_ms: Option<i64>,
    pub served_from: Option<String>,
    pub starred: bool,
}

pub fn db_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".aida-chat").join("queries.db")
}

pub fn start_query(repo_root: &Path, session_id: &str, query: &str) -> Result<i64, String> {
    let conn = open(repo_root)?;
    let ts = now_epoch_secs();
    conn.execute(
        "insert into queries (session_id, ts, query, latency_ms, served_from) values (?1, ?2, ?3, null, null)",
        params![session_id, ts, query],
    )
    .map_err(|e| format!("insert query log: {e}"))?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_query(
    repo_root: &Path,
    id: i64,
    latency_ms: i64,
    served_from: ServedFrom,
) -> Result<(), String> {
    let conn = open(repo_root)?;
    conn.execute(
        "update queries set latency_ms = ?1, served_from = ?2 where id = ?3",
        params![latency_ms, served_from.as_str(), id],
    )
    .map_err(|e| format!("update query log: {e}"))?;
    Ok(())
}

pub fn get_query(repo_root: &Path, id: i64) -> Result<Option<QueryRow>, String> {
    let conn = open(repo_root)?;
    conn.query_row(
        "select id, session_id, ts, query, latency_ms, served_from, starred from queries where id = ?1",
        params![id],
        |row| {
            Ok(QueryRow {
                id: row.get(0)?,
                session_id: row.get(1)?,
                ts: row.get(2)?,
                query: row.get(3)?,
                latency_ms: row.get(4)?,
                served_from: row.get(5)?,
                starred: row.get::<_, i64>(6)? != 0,
            })
        },
    )
    .optional()
    .map_err(|e| format!("read query log: {e}"))
}

fn open(repo_root: &Path) -> Result<Connection, String> {
    let path = db_path(repo_root);
    let dir = path
        .parent()
        .ok_or_else(|| format!("query db path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let conn = Connection::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let current: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .map_err(|e| format!("read query log schema version: {e}"))?;
    if current == 0 {
        conn.execute_batch(
            "create table if not exists queries (
                id integer primary key,
                session_id text not null,
                ts integer not null,
                query text not null,
                latency_ms integer,
                served_from text check (served_from in ('llm', 'canned', 'skill')),
                starred integer not null default 0
            );
            create index if not exists idx_queries_session_ts on queries(session_id, ts);
            create index if not exists idx_queries_served_from on queries(served_from);
            pragma user_version = 1;",
        )
        .map_err(|e| format!("migrate query log v1: {e}"))?;
    } else if current > SCHEMA_VERSION {
        return Err(format!(
            "query log schema version {current} is newer than supported {SCHEMA_VERSION}"
        ));
    }
    Ok(())
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, Clone, Copy)]
pub enum ServedFrom {
    Llm,
    Canned,
    Skill,
}

impl ServedFrom {
    pub fn as_str(self) -> &'static str {
        match self {
            ServedFrom::Llm => "llm",
            ServedFrom::Canned => "canned",
            ServedFrom::Skill => "skill",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_schema_roundtrip() {
        let root = temp_root("query-log-roundtrip");
        let id = start_query(&root, "s1", "hello").unwrap();
        finish_query(&root, id, 12, ServedFrom::Canned).unwrap();

        let row = get_query(&root, id).unwrap().unwrap();
        assert_eq!(row.session_id, "s1");
        assert_eq!(row.query, "hello");
        assert_eq!(row.latency_ms, Some(12));
        assert_eq!(row.served_from.as_deref(), Some("canned"));
        assert!(!row.starred);
    }

    fn temp_root(name: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "aida-chat-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
