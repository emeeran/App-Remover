//! Job state machine + execution (SPEC §4, NFR-5). created→running→completed,
//! or →failed→rolled_back (auto-rollback on any step failure). Execution goes
//! through the `Executor` trait so the runner is unit-testable without root.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};

use crate::audit;
use crate::db::{now_millis, Db};
use crate::domain::*;
use crate::error::{AppError, AppResult};
use crate::executor::Executor;
use crate::{planner, policy, scanner, snapshot};

/// Snapshot blob root: `${XDG_DATA_HOME:-~/.local/share}/app-remover/snapshots`.
fn blob_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
        .join("app-remover")
        .join("snapshots")
}

/// package_name, version, backend for an app (from package_instances).
fn package_info(conn: &Connection, app_id: &str) -> AppResult<(String, Option<String>, PackageBackend)> {
    let row = conn
        .query_row(
            "SELECT package_name, version, backend FROM package_instances
             WHERE canonical_app_id=?1 LIMIT 1",
            params![app_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| AppError::NotFound(format!("package instance for {app_id}")))?;
    let backend: PackageBackend =
        serde_json::from_value(serde_json::Value::String(row.2)).unwrap_or(PackageBackend::Deb);
    Ok((row.0, row.1, backend))
}

/// Create a job for an approved plan: guards, snapshot capture, persist.
pub fn create_job(plan_id: &str, proceed_despite_cost_cap: bool, db: &Db) -> AppResult<RemovalJob> {
    let conn = db.0.lock().unwrap();
    let plan = planner::load_plan(&conn, plan_id)?;
    if !matches!(plan.status, PlanStatus::Approved) {
        return Err(AppError::PlanNotApproved(plan_id.into()));
    }

    // Single-write lock: no other in-flight job for this app (FR-28).
    let conflict: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM jobs j JOIN plans p ON j.plan_id=p.plan_id
             WHERE p.canonical_app_id=?1 AND j.status IN ('created','running') LIMIT 1",
            params![plan.application_id],
            |r| r.get(0),
        )
        .optional()?;
    if conflict.is_some() {
        return Err(AppError::ConflictingJob(plan.application_id.clone()));
    }

    if plan.exceeds_cost_cap && !proceed_despite_cost_cap {
        return Err(AppError::SnapshotCostCapExceeded {
            projected: plan.projected_snapshot_bytes,
            cap: policy::snapshot_cost_cap(),
        });
    }

    let (pkg_name, version, _backend) = package_info(&conn, &plan.application_id)?;
    let artifacts = scanner::load_artifacts(&conn, &plan.application_id, plan.scan_version)?;
    let to_backup: Vec<Artifact> = artifacts
        .iter()
        .filter(|a| matches!(a.discovered_by, DiscoverySource::Heuristic))
        .cloned()
        .collect();

    let job_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO jobs (job_id, plan_id, status, created_at) VALUES (?1,?2,'created',?3)",
        params![job_id, plan_id, now_millis()],
    )?;

    let snap = snapshot::capture(&job_id, &pkg_name, version.as_deref(), &to_backup, &blob_root())?;
    snapshot::persist_snapshot(&conn, &snap)?;
    conn.execute(
        "UPDATE jobs SET snapshot_id=?1 WHERE job_id=?2",
        params![snap.snapshot_id, job_id],
    )?;

    load_job(&conn, &job_id)
}

/// Begin a created job: execute its operations; auto-rollback on failure; audit.
pub fn begin_job(job_id: &str, executor: &dyn Executor, db: &Db) -> AppResult<RemovalJob> {
    let conn = db.0.lock().unwrap();
    let mut job = load_job(&conn, job_id)?;
    if !matches!(job.status, JobStatus::Created) {
        return Err(AppError::IllegalTransition(format!(
            "job {job_id} is {:?}, not 'created'",
            job.status
        )));
    }
    let plan = planner::load_plan(&conn, &job.plan_id)?;
    if !matches!(plan.status, PlanStatus::Approved) {
        return Err(AppError::PlanNotApproved(job.plan_id.clone())); // F3 re-check
    }
    let (_pkg, _ver, backend) = package_info(&conn, &plan.application_id)?;
    let purge = matches!(plan.mode, RemovalMode::Purge);
    let artifacts = scanner::load_artifacts(&conn, &plan.application_id, plan.scan_version)?;
    let path_map: HashMap<String, String> = artifacts
        .iter()
        .map(|a| (a.artifact_id.clone(), a.target.clone()))
        .collect();

    job.status = JobStatus::Running;
    job.started_at = Some(now_millis());
    update_job(&conn, &job)?;

    let mut steps: Vec<ExecutedStep> = Vec::new();
    let mut failure: Option<ErrorDetail> = None;

    for op in &plan.operations {
        let mut step = ExecutedStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            operation_id: op.operation_id.clone(),
            order: op.order,
            status: StepStatus::Running,
            started_at: Some(now_millis()),
            finished_at: None,
            error: None,
        };
        match execute_op(op, &path_map, backend, purge, executor) {
            Ok(()) => {
                step.status = StepStatus::Succeeded;
            }
            Err(e) => {
                step.status = StepStatus::Failed;
                step.error = Some(ErrorDetail {
                    code: e.code().into(),
                    message: e.message(),
                });
                failure = Some(ErrorDetail {
                    code: e.code().into(),
                    message: format!("operation {} failed: {}", op.operation_id, e.message()),
                });
                steps.push(step);
                break;
            }
        }
        step.finished_at = Some(now_millis());
        steps.push(step);
    }

    let snapshot_id = job.snapshot_id.clone().unwrap_or_default();
    let (outcome, undoable) = if failure.is_none() {
        job.status = JobStatus::Completed;
        ("completed", true)
    } else {
        job.status = JobStatus::Failed;
        job.failure = failure.clone();
        update_job(&conn, &job)?;
        // Auto-rollback (NFR-5). SnapshotCorrupt → cannot safely undo.
        if let Err(e) = rollback(&conn, &snapshot_id, backend, executor) {
            job.failure = Some(ErrorDetail {
                code: e.code().into(),
                message: format!("rollback failed: {}", e.message()),
            });
        } else {
            job.status = JobStatus::RolledBack;
        }
        ("rolled_back", false)
    };
    job.finished_at = Some(now_millis());

    persist_steps(&conn, job_id, &steps)?;
    update_job(&conn, &job)?;

    let plan_json = serde_json::to_value(&plan)?;
    audit::append(&conn, job_id, &plan_json, &steps, &snapshot_id, outcome, undoable)?;

    load_job(&conn, job_id)
}

fn execute_op(
    op: &RemovalOperation,
    path_map: &HashMap<String, String>,
    backend: PackageBackend,
    purge: bool,
    executor: &dyn Executor,
) -> AppResult<()> {
    match op.action {
        Action::UninstallPackage => executor.uninstall_package(backend, &op.target_ref, purge),
        Action::DeleteFile => {
            let path = path_map
                .get(&op.target_ref)
                .ok_or_else(|| AppError::Internal(format!("no path for artifact {}", op.target_ref)))?;
            executor.delete_path(path)
        }
        // Stop-service / association / orphan pruning: no-op in M5.
        _ => Ok(()),
    }
}

fn rollback(conn: &Connection, snapshot_id: &str, backend: PackageBackend, executor: &dyn Executor) -> AppResult<()> {
    let snap = snapshot::load_snapshot(conn, snapshot_id)?;
    snapshot::verify(&snap)?; // NFR-15
    snapshot::restore(&snap, backend, executor)?;
    Ok(())
}

/// Undo a completed removal: load the audit record + its snapshot, verify before
/// restore (NFR-15), then best-effort restore — packages that fail to reinstall
/// are returned as `deferred` rather than aborting the whole undo.
pub fn undo(audit_record_id: &str, executor: &dyn Executor, db: &Db) -> AppResult<UndoResult> {
    let conn = db.0.lock().unwrap();
    let rec = audit::load(&conn, audit_record_id)?;
    if !rec.undoable {
        return Err(AppError::IllegalTransition(
            "audit record is not undoable".into(),
        ));
    }
    let snap = snapshot::load_snapshot(&conn, &rec.snapshot_id)?;
    snapshot::verify(&snap)?; // NFR-15 — refuses on tamper/corruption

    let mut restored_files = 0u32;
    let mut reinstalled = Vec::new();
    let mut deferred = Vec::new();
    for e in &snap.entries {
        match e.kind {
            SnapshotEntryKind::FileBackup => {
                if let Some(bp) = &e.blob_path {
                    if executor.restore_file(bp, &e.original_path).is_ok() {
                        restored_files += 1;
                    }
                }
            }
            SnapshotEntryKind::PackageRecord => {
                if let Some(pkg) = &e.package_name {
                    match executor.reinstall_package(PackageBackend::Deb, pkg) {
                        Ok(()) => reinstalled.push(pkg.clone()),
                        Err(_) => deferred.push(pkg.clone()),
                    }
                }
            }
            SnapshotEntryKind::UnitRecord => {}
        }
    }
    Ok(UndoResult {
        audit_record_id: audit_record_id.to_string(),
        restored_files,
        reinstalled_packages: reinstalled,
        deferred_packages: deferred,
    })
}

// --------------------------- persistence ---------------------------

pub fn load_job(conn: &Connection, job_id: &str) -> AppResult<RemovalJob> {
    let row = conn
        .query_row(
            "SELECT plan_id, snapshot_id, status, created_at, started_at, finished_at, failure
             FROM jobs WHERE job_id=?1",
            params![job_id],
            |r| {
                Ok(JobRow {
                    plan_id: r.get(0)?,
                    snapshot_id: r.get(1)?,
                    status: r.get(2)?,
                    created_at: r.get(3)?,
                    started_at: r.get(4)?,
                    finished_at: r.get(5)?,
                    failure: r.get(6)?,
                })
            },
        )
        .map_err(|_| AppError::NotFound(format!("job {job_id}")))?;

    let mut stmt = conn.prepare(
        "SELECT step_id, operation_id, \"order\", status, started_at, finished_at, error
         FROM executed_steps WHERE job_id=?1 ORDER BY \"order\"",
    )?;
    let mut rows = stmt.query_map(params![job_id], |r| {
        Ok(StepRow {
            step_id: r.get(0)?,
            operation_id: r.get(1)?,
            order: r.get(2)?,
            status: r.get(3)?,
            started_at: r.get(4)?,
            finished_at: r.get(5)?,
            error: r.get(6)?,
        })
    })?;
    let mut steps = Vec::new();
    for row in rows.by_ref() {
        let r = row?;
        steps.push(ExecutedStep {
            step_id: r.step_id,
            operation_id: r.operation_id,
            order: r.order,
            status: serde_json::from_value(serde_json::Value::String(r.status))
                .unwrap_or(StepStatus::Pending),
            started_at: r.started_at,
            finished_at: r.finished_at,
            error: r.error.and_then(|s| serde_json::from_str(&s).ok()),
        });
    }

    Ok(RemovalJob {
        job_id: job_id.to_string(),
        plan_id: row.plan_id,
        snapshot_id: row.snapshot_id,
        status: serde_json::from_value(serde_json::Value::String(row.status))
            .unwrap_or(JobStatus::Created),
        steps,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        failure: row.failure.and_then(|s| serde_json::from_str(&s).ok()),
    })
}

fn update_job(conn: &Connection, job: &RemovalJob) -> AppResult<()> {
    let failure = job.failure.as_ref().map(|f| serde_json::to_string(f)).transpose()?;
    conn.execute(
        "UPDATE jobs SET status=?1, started_at=?2, finished_at=?3, failure=?4 WHERE job_id=?5",
        params![
            jstr_enum(job.status),
            job.started_at,
            job.finished_at,
            failure,
            job.job_id,
        ],
    )?;
    Ok(())
}

fn persist_steps(conn: &Connection, job_id: &str, steps: &[ExecutedStep]) -> AppResult<()> {
    conn.execute("DELETE FROM executed_steps WHERE job_id=?1", params![job_id])?;
    for s in steps {
        let error = s.error.as_ref().map(|e| serde_json::to_string(e)).transpose()?;
        conn.execute(
            "INSERT INTO executed_steps
             (step_id, job_id, operation_id, \"order\", status, started_at, finished_at, error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                s.step_id,
                job_id,
                s.operation_id,
                s.order,
                jstr_enum(s.status),
                s.started_at,
                s.finished_at,
                error,
            ],
        )?;
    }
    Ok(())
}

fn jstr_enum<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(&v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

struct JobRow {
    plan_id: String,
    snapshot_id: Option<String>,
    status: String,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    failure: Option<String>,
}
struct StepRow {
    step_id: String,
    operation_id: String,
    order: u32,
    status: String,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::sync::Mutex;

    struct FakeExec {
        fail_uninstall: bool,
        calls: Mutex<Vec<&'static str>>,
    }
    impl Executor for FakeExec {
        fn uninstall_package(&self, _: PackageBackend, _: &str, _: bool) -> AppResult<()> {
            self.calls.lock().unwrap().push("uninstall");
            if self.fail_uninstall {
                Err(AppError::Internal("uninstall failed (simulated)".into()))
            } else {
                Ok(())
            }
        }
        fn reinstall_package(&self, _: PackageBackend, _: &str) -> AppResult<()> {
            self.calls.lock().unwrap().push("reinstall");
            Ok(())
        }
        fn delete_path(&self, _: &str) -> AppResult<()> {
            self.calls.lock().unwrap().push("delete");
            Ok(())
        }
        fn restore_file(&self, _: &str, _: &str) -> AppResult<()> {
            self.calls.lock().unwrap().push("restore");
            Ok(())
        }
    }

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn fresh() -> Db {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("ar-run-{n}.db"));
        let _ = std::fs::remove_file(&p);
        Db::open(&p).expect("open")
    }

    /// Seed a full approved-plan chain for app `app1` / package `foo`.
    fn seed(conn: &Connection) {
        conn.execute(
            "INSERT INTO applications (canonical_app_id,name,first_seen_at) VALUES ('app1','foo',0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO package_instances (package_instance_id,canonical_app_id,backend,package_name,version,scope)
             VALUES ('pi1','app1','deb','foo','1.0','system')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scans (canonical_app_id,scan_version,sealed_at) VALUES ('app1',1,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO artifacts (artifact_id,canonical_app_id,scan_version,category,target,owner_set,usage_kind,size_bytes,deletable,confidence,discovered_by,scope)
             VALUES ('aid1','app1',1,'config','/tmp/nonexistent','[\"app1\"]','exclusive',0,1,'high','heuristic','user')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO plans (plan_id,canonical_app_id,scan_version,mode,scope,status,projected_snapshot_bytes,exceeds_cost_cap,composed_at)
             VALUES ('p1','app1',1,'remove','system-wide','approved',0,0,1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO plan_operations (operation_id,plan_id,\"order\",action,target_kind,target_ref,verdict,impact,rationale,accepted)
             VALUES ('o1','p1',0,'uninstall-package','package','foo','safe','[]','ok',0),
                    ('o2','p1',1,'delete-file','artifact','aid1','safe','[]','ok',0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn happy_path_completes() {
        let db = fresh();
        {
            let c = db.0.lock().unwrap();
            seed(&c);
        }
        let job = create_job("p1", false, &db).unwrap();
        assert_eq!(job.status, JobStatus::Created);
        assert!(job.snapshot_id.is_some());

        let fake = FakeExec { fail_uninstall: false, calls: Mutex::new(vec![]) };
        let done = begin_job(&job.job_id, &fake, &db).unwrap();
        assert_eq!(done.status, JobStatus::Completed);
        assert_eq!(done.steps.len(), 2);
        assert!(done.steps.iter().all(|s| s.status == StepStatus::Succeeded));
        let calls = fake.calls.lock().unwrap();
        assert!(calls.contains(&"uninstall"));
        assert!(calls.contains(&"delete"));
    }

    #[test]
    fn undo_restores_completed_job() {
        let db = fresh();
        {
            let c = db.0.lock().unwrap();
            seed(&c);
        }
        let job = create_job("p1", false, &db).unwrap();
        let fake = FakeExec {
            fail_uninstall: false,
            calls: Mutex::new(vec![]),
        };
        let done = begin_job(&job.job_id, &fake, &db).unwrap();
        assert_eq!(done.status, JobStatus::Completed);
        let audit_id = {
            let c = db.0.lock().unwrap();
            c.query_row(
                "SELECT audit_record_id FROM audit_records WHERE job_id=?1",
                params![done.job_id],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        let res = undo(&audit_id, &fake, &db).unwrap();
        assert!(res.reinstalled_packages.contains(&"foo".to_string()));
        assert!(fake.calls.lock().unwrap().contains(&"reinstall"));
    }

    #[test]
    fn undo_refuses_non_undoable() {
        let db = fresh();
        let audit_id = {
            let c = db.0.lock().unwrap();
            seed(&c);
            c.execute(
                "INSERT INTO jobs (job_id,plan_id,status,created_at) VALUES ('jx','p1','rolled-back',1)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO snapshots (snapshot_id,job_id,captured_at,checksum_algo,manifest_checksum,total_bytes,excludes_cache,blob_root)
                 VALUES ('sx','jx',1,'sha256','x',0,1,'/tmp')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO audit_records (audit_record_id,job_id,plan_snapshot,steps,snapshot_id,outcome,undoable,created_at,hash,prev_hash)
                 VALUES ('ar1','jx','{}','[]','sx','rolled_back',0,1,'h',NULL)",
                [],
            )
            .unwrap();
            "ar1".to_string()
        };
        let fake = FakeExec {
            fail_uninstall: false,
            calls: Mutex::new(vec![]),
        };
        let err = undo(&audit_id, &fake, &db).unwrap_err();
        assert!(matches!(err, AppError::IllegalTransition(_)));
    }

    #[test]
    fn failure_triggers_rollback() {
        let db = fresh();
        {
            let c = db.0.lock().unwrap();
            seed(&c);
        }
        let job = create_job("p1", false, &db).unwrap();
        let fake = FakeExec { fail_uninstall: true, calls: Mutex::new(vec![]) };
        let done = begin_job(&job.job_id, &fake, &db).unwrap();
        assert_eq!(done.status, JobStatus::RolledBack);
        // uninstall attempted, failed; rollback reinstalled the package
        let calls = fake.calls.lock().unwrap();
        assert!(calls.contains(&"uninstall"));
        assert!(calls.contains(&"reinstall"));
        // failed step recorded
        assert!(done.steps.iter().any(|s| s.status == StepStatus::Failed));
    }

    #[test]
    fn unapproved_plan_rejected() {
        let db = fresh();
        {
            let c = db.0.lock().unwrap();
            seed(&c);
            c.execute("UPDATE plans SET status='draft' WHERE plan_id='p1'", []).unwrap();
        }
        let err = create_job("p1", false, &db).unwrap_err();
        assert!(matches!(err, AppError::PlanNotApproved(_)));
    }

    #[test]
    fn conflicting_job_rejected() {
        let db = fresh();
        {
            let c = db.0.lock().unwrap();
            seed(&c);
            c.execute("INSERT INTO jobs (job_id,plan_id,status,created_at) VALUES ('j0','p1','running',1)", []).unwrap();
        }
        let err = create_job("p1", false, &db).unwrap_err();
        assert!(matches!(err, AppError::ConflictingJob(_)));
    }

    #[test]
    fn begin_non_created_rejected() {
        let db = fresh();
        let job_id = {
            let c = db.0.lock().unwrap();
            seed(&c);
            c.execute("INSERT INTO jobs (job_id,plan_id,status,created_at) VALUES ('jx','p1','running',1)", []).unwrap();
            "jx".to_string()
        };
        let fake = FakeExec { fail_uninstall: false, calls: Mutex::new(vec![]) };
        let err = begin_job(&job_id, &fake, &db).unwrap_err();
        assert!(matches!(err, AppError::IllegalTransition(_)));
    }
}
