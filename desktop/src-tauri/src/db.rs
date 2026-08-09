//! SQLite connection management. The schema is embedded and applied
//! idempotently on open (migration-on-startup). The connection sits behind a
//! `Mutex` in `tauri::State`, mirroring the Node reference's synchronous
//! better-sqlite3 model.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::AppResult;

pub struct Db(pub Mutex<Connection>);

impl Db {
    /// Open (creating the file + parent dir) and migrate the database.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(include_str!("schema.sql"))?;
        Ok(Self(Mutex::new(conn)))
    }
}

/// Epoch milliseconds right now (timestamp helper for inserts).
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates_all_tables() {
        let mut path = std::env::temp_dir();
        path.push(format!("ar-m1-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let db = Db::open(&path).expect("open + migrate");
        let conn = db.0.lock().unwrap();

        let expected = [
            "applications",
            "install_sources",
            "package_instances",
            "scans",
            "artifacts",
            "plans",
            "plan_operations",
            "jobs",
            "executed_steps",
            "snapshots",
            "snapshot_entries",
            "audit_records",
            "event_log",
        ];
        for table in expected {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |r| r.get(0),
                )
                .expect("query sqlite_master");
            assert_eq!(n, 1, "table `{table}` missing after migration");
        }
    }
}
