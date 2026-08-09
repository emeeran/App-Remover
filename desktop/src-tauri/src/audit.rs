//! Append-only, hash-chained audit log (SPEC §3.7, NFR-9).
//!
//! Each record's `hash = SHA256(prev_hash || canonical(record))`; `prev_hash`
//! is the prior record's hash (NULL for the genesis record). No UPDATE/DELETE
//! is ever issued against `audit_records`.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::db::now_millis;
use crate::domain::{AuditRecord, ExecutedStep};
use crate::dto::HistoryItem;
use crate::error::{AppError, AppResult};

fn last_hash(conn: &Connection) -> AppResult<Option<String>> {
    let v: Option<String> = conn
        .query_row(
            "SELECT hash FROM audit_records
             ORDER BY created_at DESC, audit_record_id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v)
}

/// Append a terminal audit record for a job (outcome "completed" or
/// "rolled_back"). Computes and persists the hash-chain link.
pub fn append(
    conn: &Connection,
    job_id: &str,
    plan_snapshot: &serde_json::Value,
    steps: &[ExecutedStep],
    snapshot_id: &str,
    outcome: &str,
    undoable: bool,
) -> AppResult<AuditRecord> {
    let id = uuid::Uuid::new_v4().to_string();
    let created = now_millis();
    let prev = last_hash(conn)?;
    let plan_json = serde_json::to_string(plan_snapshot)?;
    let steps_json = serde_json::to_string(steps)?;
    let canonical = format!(
        "{id}|{job_id}|{plan_json}|{steps_json}|{snapshot_id}|{outcome}|{undoable}|{created}"
    );

    let mut h = Sha256::new();
    if let Some(p) = &prev {
        h.update(p.as_bytes());
    }
    h.update(canonical.as_bytes());
    let hash = hex::encode(h.finalize());

    conn.execute(
        "INSERT INTO audit_records
         (audit_record_id, job_id, plan_snapshot, steps, snapshot_id, outcome, undoable,
          created_at, hash, prev_hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            job_id,
            plan_json,
            steps_json,
            snapshot_id,
            outcome,
            undoable as i64,
            created,
            hash,
            prev,
        ],
    )?;

    Ok(AuditRecord {
        audit_record_id: id,
        job_id: job_id.to_string(),
        plan_snapshot: plan_snapshot.clone(),
        steps: steps.to_vec(),
        snapshot_id: snapshot_id.to_string(),
        outcome: outcome.to_string(),
        undoable,
        created_at: created,
        hash,
        prev_hash: prev,
    })
}

/// Load one audit record (parse the JSON plan/steps back into types).
pub fn load(conn: &Connection, id: &str) -> AppResult<AuditRecord> {
    let row = conn
        .query_row(
            "SELECT audit_record_id, job_id, plan_snapshot, steps, snapshot_id, outcome,
                    undoable, created_at, hash, prev_hash
             FROM audit_records WHERE audit_record_id=?1",
            params![id],
            |r| {
                Ok(AuditRow {
                    audit_record_id: r.get(0)?,
                    job_id: r.get(1)?,
                    plan_snapshot: r.get(2)?,
                    steps: r.get(3)?,
                    snapshot_id: r.get(4)?,
                    outcome: r.get(5)?,
                    undoable: r.get(6)?,
                    created_at: r.get(7)?,
                    hash: r.get(8)?,
                    prev_hash: r.get(9)?,
                })
            },
        )
        .map_err(|_| AppError::NotFound(format!("audit record {id}")))?;
    let plan: serde_json::Value = serde_json::from_str(&row.plan_snapshot)?;
    let steps: Vec<ExecutedStep> = serde_json::from_str(&row.steps)?;
    Ok(AuditRecord {
        audit_record_id: row.audit_record_id,
        job_id: row.job_id,
        plan_snapshot: plan,
        steps,
        snapshot_id: row.snapshot_id,
        outcome: row.outcome,
        undoable: row.undoable != 0,
        created_at: row.created_at,
        hash: row.hash,
        prev_hash: row.prev_hash,
    })
}

/// Most recent audit records (history view), with the affected app's name.
pub fn recent(conn: &Connection, limit: i64) -> AppResult<Vec<HistoryItem>> {
    let mut stmt = conn.prepare(
        "SELECT a.audit_record_id, a.job_id, a.outcome, a.undoable, a.created_at, app.name
         FROM audit_records a
         LEFT JOIN jobs j ON a.job_id = j.job_id
         LEFT JOIN plans p ON j.plan_id = p.plan_id
         LEFT JOIN applications app ON p.canonical_app_id = app.canonical_app_id
         ORDER BY a.created_at DESC, a.audit_record_id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |r| {
        Ok(HistoryItem {
            audit_record_id: r.get(0)?,
            job_id: r.get(1)?,
            outcome: r.get(2)?,
            undoable: r.get::<_, i64>(3)? != 0,
            created_at: r.get(4)?,
            app_name: r.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(AppError::from)
}

struct AuditRow {
    audit_record_id: String,
    job_id: String,
    plan_snapshot: String,
    steps: String,
    snapshot_id: String,
    outcome: String,
    undoable: i64,
    created_at: i64,
    hash: String,
    prev_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let mut p = std::env::temp_dir();
        p.push(format!("ar-audit-{}.db", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_file(&p);
        let conn = Connection::open(&p).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute(
            "INSERT INTO applications (canonical_app_id,name,first_seen_at) VALUES ('a','A',0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO plans (plan_id,canonical_app_id,scan_version,mode,scope,status,projected_snapshot_bytes,exceeds_cost_cap,composed_at) VALUES ('p','a',1,'remove','system-wide','approved',0,0,0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (job_id,plan_id,status,created_at) VALUES ('j','p','completed',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snapshots (snapshot_id,job_id,captured_at,checksum_algo,manifest_checksum,total_bytes,excludes_cache,blob_root) VALUES ('s','j',1,'sha256','x',0,1,'/tmp')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn chains_and_is_deterministic() {
        let conn = fresh();
        let r1 = append(&conn, "j", &serde_json::json!({"plan":1}), &[], "s", "completed", true).unwrap();
        assert!(r1.prev_hash.is_none());
        let r2 = append(&conn, "j", &serde_json::json!({"plan":2}), &[], "s", "rolled_back", false).unwrap();
        assert_eq!(r2.prev_hash.as_deref(), Some(r1.hash.as_str()));
        // hash incorporates content → different records differ
        assert_ne!(r1.hash, r2.hash);
    }
}
