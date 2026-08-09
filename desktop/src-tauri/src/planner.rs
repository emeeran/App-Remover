//! Plan composition + approval (SPEC §3.4, decisions D11/D14).
//!
//! Composition turns a sealed residue graph into an ordered list of operations
//! with safety verdicts. Verdict logic is pure (NFR-4) and unit-tested; live
//! reverse-dependency data feeds the uninstall verdict.

use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};

use crate::backends;
use crate::db::{now_millis, Db};
use crate::domain::*;
use crate::error::{AppError, AppResult};
use crate::policy;

/// Compose a dry-run plan over a sealed scan and persist it.
pub fn compose(
    app: &CanonicalApplication,
    graph: &ResidueGraph,
    mode: RemovalMode,
    scope: RemovalScope,
    db: &Db,
) -> AppResult<RemovalPlan> {
    let pkg_name = app
        .package_instance_refs
        .first()
        .map(|r| r.package_name.clone())
        .unwrap_or_else(|| app.name.clone());

    let mut operations = Vec::new();
    let mut order = 0u32;

    // 1. Uninstall the package itself — verdict from reverse dependencies.
    let rdep_names = backends::reverse_deps(app).unwrap_or_default();
    let impact_ids: Vec<String> = rdep_names
        .iter()
        .map(|n| backends::canonical_id(PackageBackend::Deb, n))
        .collect();
    let has_protected_rdep = rdep_names.iter().any(|n| policy::is_protected(n));
    let (verdict, rationale) = verdict_uninstall(&rdep_names, has_protected_rdep);
    operations.push(RemovalOperation {
        operation_id: uuid::Uuid::new_v4().to_string(),
        order,
        action: Action::UninstallPackage,
        target_kind: "package".into(),
        target_ref: pkg_name,
        verdict,
        impact: impact_ids,
        rationale,
        accepted: false,
    });
    order += 1;

    // 2. Delete leftover (heuristic) artifacts the package manager won't remove.
    for a in &graph.artifacts {
        if !matches!(a.discovered_by, DiscoverySource::Heuristic) {
            continue;
        }
        let (v, why) = verdict_delete(a.usage_kind);
        operations.push(RemovalOperation {
            operation_id: uuid::Uuid::new_v4().to_string(),
            order,
            action: Action::DeleteFile,
            target_kind: "artifact".into(),
            target_ref: a.artifact_id.clone(),
            verdict: v,
            impact: Vec::new(),
            rationale: why,
            accepted: false,
        });
        order += 1;
    }

    // Snapshot projection (D11): only leftover artifacts get backed up.
    let projected: u64 = graph
        .artifacts
        .iter()
        .filter(|a| matches!(a.discovered_by, DiscoverySource::Heuristic))
        .map(|a| a.size_bytes.unwrap_or(0))
        .sum();
    let exceeds_cost_cap = projected > policy::snapshot_cost_cap();

    let plan = RemovalPlan {
        plan_id: uuid::Uuid::new_v4().to_string(),
        application_id: app.canonical_app_id.clone(),
        scan_version: graph.scan_version,
        mode,
        scope,
        status: PlanStatus::Draft,
        blocked_reasons: blocked_reasons_from(&operations),
        operations,
        projected_snapshot_bytes: projected,
        exceeds_cost_cap,
        composed_at: now_millis(),
    };

    let conn = db.0.lock().unwrap();
    persist_plan(&conn, &plan)?;
    Ok(plan)
}

/// Verdict for uninstalling a package (pure). Protected dependent → Blocked;
/// any dependent → Risky; else Safe.
pub fn verdict_uninstall(rdep_names: &[String], has_protected_rdep: bool) -> (SafetyVerdict, String) {
    if has_protected_rdep {
        return (
            SafetyVerdict::Blocked,
            "a protected/essential package depends on this".into(),
        );
    }
    if !rdep_names.is_empty() {
        return (
            SafetyVerdict::Risky,
            format!("{} package(s) depend on this", rdep_names.len()),
        );
    }
    (SafetyVerdict::Safe, "no reverse dependencies".into())
}

/// Verdict for deleting a leftover artifact (pure). Exclusive → Safe; Shared
/// or System → Blocked (NFR-4: never operate on shared/system).
pub fn verdict_delete(usage: UsageKind) -> (SafetyVerdict, String) {
    match usage {
        UsageKind::Exclusive => (SafetyVerdict::Safe, "exclusive to this app".into()),
        UsageKind::Shared => (SafetyVerdict::Blocked, "shared with other apps".into()),
        UsageKind::System => (SafetyVerdict::Blocked, "system-owned".into()),
    }
}

fn blocked_reasons_from(operations: &[RemovalOperation]) -> Vec<String> {
    operations
        .iter()
        .filter(|o| matches!(o.verdict, SafetyVerdict::Blocked))
        .map(|o| format!("{}: {}", o.target_ref, o.rationale))
        .collect()
}

/// Approve a draft plan (F3): refuse blocked ops; require risky ops to be
/// explicitly accepted. Idempotent if already approved.
pub fn approve(plan_id: &str, accepted_risk_op_ids: &[String], db: &Db) -> AppResult<RemovalPlan> {
    let conn = db.0.lock().unwrap();
    let mut plan = load_plan(&conn, plan_id)?;
    if matches!(plan.status, PlanStatus::Approved) {
        return Ok(plan);
    }
    if matches!(plan.status, PlanStatus::Superseded) {
        return Err(AppError::IllegalTransition("plan superseded".into()));
    }

    let blocked: Vec<String> = plan
        .operations
        .iter()
        .filter(|o| matches!(o.verdict, SafetyVerdict::Blocked))
        .map(|o| o.operation_id.clone())
        .collect();
    if !blocked.is_empty() {
        return Err(AppError::PlanHasBlockedOp(blocked));
    }

    let unaccepted: Vec<String> = plan
        .operations
        .iter()
        .filter(|o| {
            matches!(o.verdict, SafetyVerdict::Risky) && !accepted_risk_op_ids.contains(&o.operation_id)
        })
        .map(|o| o.operation_id.clone())
        .collect();
    if !unaccepted.is_empty() {
        return Err(AppError::RiskNotAccepted(unaccepted));
    }

    for op in plan.operations.iter_mut() {
        if matches!(op.verdict, SafetyVerdict::Risky) && accepted_risk_op_ids.contains(&op.operation_id) {
            op.accepted = true;
        }
    }
    plan.status = PlanStatus::Approved;
    persist_plan(&conn, &plan)?;
    Ok(plan)
}

// --------------------------- persistence ---------------------------

fn jstr<T: Serialize>(v: &T) -> AppResult<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default())
}
fn eparse<T: DeserializeOwned>(s: &str) -> AppResult<T> {
    Ok(serde_json::from_value(serde_json::Value::String(s.to_owned()))?)
}

pub fn persist_plan(conn: &Connection, plan: &RemovalPlan) -> AppResult<()> {
    // Delete children BEFORE upserting the parent: plan_operations FK is
    // ON DELETE RESTRICT, so an INSERT OR REPLACE on `plans` would fail while
    // operations still reference it.
    conn.execute(
        "DELETE FROM plan_operations WHERE plan_id=?1",
        params![plan.plan_id],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO plans
         (plan_id, canonical_app_id, scan_version, mode, scope, status,
          projected_snapshot_bytes, exceeds_cost_cap, composed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            plan.plan_id,
            plan.application_id,
            plan.scan_version,
            jstr(&plan.mode)?,
            jstr(&plan.scope)?,
            jstr(&plan.status)?,
            plan.projected_snapshot_bytes as i64,
            plan.exceeds_cost_cap as i64,
            plan.composed_at,
        ],
    )?;
    for op in &plan.operations {
        conn.execute(
            "INSERT INTO plan_operations
             (operation_id, plan_id, \"order\", action, target_kind, target_ref,
              verdict, impact, rationale, accepted)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                op.operation_id,
                plan.plan_id,
                op.order,
                jstr(&op.action)?,
                op.target_kind,
                op.target_ref,
                jstr(&op.verdict)?,
                serde_json::to_string(&op.impact)?,
                op.rationale,
                op.accepted as i64,
            ],
        )?;
    }
    Ok(())
}

pub fn load_plan(conn: &Connection, plan_id: &str) -> AppResult<RemovalPlan> {
    let row = conn
        .query_row(
            "SELECT canonical_app_id, scan_version, mode, scope, status,
                    projected_snapshot_bytes, exceeds_cost_cap, composed_at
             FROM plans WHERE plan_id=?1",
            params![plan_id],
            |r| {
                Ok(PlanRow {
                    application_id: r.get(0)?,
                    scan_version: r.get(1)?,
                    mode: r.get(2)?,
                    scope: r.get(3)?,
                    status: r.get(4)?,
                    projected: r.get(5)?,
                    exceeds: r.get(6)?,
                    composed: r.get(7)?,
                })
            },
        )
        .map_err(|_| AppError::NotFound(format!("plan {plan_id}")))?;

    let mut stmt = conn.prepare(
        "SELECT operation_id, \"order\", action, target_kind, target_ref, verdict,
                impact, rationale, accepted
         FROM plan_operations WHERE plan_id=?1 ORDER BY \"order\"",
    )?;
    let mut rows = stmt.query_map(params![plan_id], |r| {
        Ok(OpRow {
            operation_id: r.get(0)?,
            order: r.get(1)?,
            action: r.get(2)?,
            target_kind: r.get(3)?,
            target_ref: r.get(4)?,
            verdict: r.get(5)?,
            impact: r.get(6)?,
            rationale: r.get(7)?,
            accepted: r.get(8)?,
        })
    })?;
    let mut operations = Vec::new();
    for row in rows.by_ref() {
        let r = row?;
        operations.push(RemovalOperation {
            operation_id: r.operation_id,
            order: r.order,
            action: eparse(&r.action).unwrap_or(Action::DeleteFile),
            target_kind: r.target_kind,
            target_ref: r.target_ref,
            verdict: eparse(&r.verdict).unwrap_or(SafetyVerdict::ManualReview),
            impact: serde_json::from_str(&r.impact).unwrap_or_default(),
            rationale: r.rationale,
            accepted: r.accepted != 0,
        });
    }

    let blocked_reasons = blocked_reasons_from(&operations);
    Ok(RemovalPlan {
        plan_id: plan_id.to_string(),
        application_id: row.application_id,
        scan_version: row.scan_version,
        mode: eparse(&row.mode)?,
        scope: eparse(&row.scope)?,
        status: eparse(&row.status)?,
        operations,
        blocked_reasons,
        projected_snapshot_bytes: row.projected as u64,
        exceeds_cost_cap: row.exceeds != 0,
        composed_at: row.composed,
    })
}

struct PlanRow {
    application_id: String,
    scan_version: u32,
    mode: String,
    scope: String,
    status: String,
    projected: i64,
    exceeds: i64,
    composed: i64,
}
struct OpRow {
    operation_id: String,
    order: u32,
    action: String,
    target_kind: String,
    target_ref: String,
    verdict: String,
    impact: String,
    rationale: String,
    accepted: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn fresh_db() -> Db {
        // Unique file per call — tests run in parallel and must not share a db.
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("ar-plan-{n}.db"));
        let _ = std::fs::remove_file(&p);
        let db = Db::open(&p).expect("open");
        let conn = db.0.lock().unwrap();
        conn.execute(
            "INSERT INTO applications (canonical_app_id, name, first_seen_at) VALUES (?1,?2,?3)",
            params!["app1", "App One", 0],
        )
        .unwrap();
        drop(conn);
        db
    }

    #[test]
    fn verdict_uninstall_levels() {
        assert_eq!(verdict_uninstall(&[], false).0, SafetyVerdict::Safe);
        assert_eq!(
            verdict_uninstall(&["dep".into()], false).0,
            SafetyVerdict::Risky
        );
        assert_eq!(
            verdict_uninstall(&["dep".into()], true).0,
            SafetyVerdict::Blocked
        );
    }

    #[test]
    fn verdict_delete_levels() {
        assert_eq!(verdict_delete(UsageKind::Exclusive).0, SafetyVerdict::Safe);
        assert_eq!(verdict_delete(UsageKind::Shared).0, SafetyVerdict::Blocked);
        assert_eq!(verdict_delete(UsageKind::System).0, SafetyVerdict::Blocked);
    }

    fn op(id: &str, verdict: SafetyVerdict) -> RemovalOperation {
        RemovalOperation {
            operation_id: id.into(),
            order: 0,
            action: Action::DeleteFile,
            target_kind: "artifact".into(),
            target_ref: id.into(),
            verdict,
            impact: Vec::new(),
            rationale: "t".into(),
            accepted: false,
        }
    }

    fn draft_plan(id: &str, ops: Vec<RemovalOperation>) -> RemovalPlan {
        RemovalPlan {
            plan_id: id.into(),
            application_id: "app1".into(),
            scan_version: 1,
            mode: RemovalMode::Remove,
            scope: RemovalScope::SystemWide,
            status: PlanStatus::Draft,
            blocked_reasons: blocked_reasons_from(&ops),
            operations: ops,
            projected_snapshot_bytes: 0,
            exceeds_cost_cap: false,
            composed_at: 0,
        }
    }

    #[test]
    fn plan_round_trips() {
        let db = fresh_db();
        let conn = db.0.lock().unwrap();
        let plan = draft_plan(
            "p1",
            vec![op("o1", SafetyVerdict::Safe), op("o2", SafetyVerdict::Risky)],
        );
        persist_plan(&conn, &plan).unwrap();
        let loaded = load_plan(&conn, "p1").unwrap();
        assert_eq!(loaded.operations.len(), 2);
        assert_eq!(loaded.operations[0].verdict, SafetyVerdict::Safe);
        assert_eq!(loaded.operations[1].verdict, SafetyVerdict::Risky);
    }

    #[test]
    fn approve_rejects_blocked() {
        let db = fresh_db();
        {
            let conn = db.0.lock().unwrap();
            persist_plan(&conn, &draft_plan("pb", vec![op("o1", SafetyVerdict::Blocked)])).unwrap();
        }
        let err = approve("pb", &[], &db).unwrap_err();
        assert!(matches!(err, AppError::PlanHasBlockedOp(_)));
    }

    #[test]
    fn approve_requires_risky_acceptance() {
        let db = fresh_db();
        {
            let conn = db.0.lock().unwrap();
            persist_plan(&conn, &draft_plan("pr", vec![op("o1", SafetyVerdict::Risky)])).unwrap();
        }
        let err = approve("pr", &[], &db).unwrap_err();
        assert!(matches!(err, AppError::RiskNotAccepted(_)));
        // accepting it succeeds
        let approved = approve("pr", &["o1".to_string()], &db).unwrap();
        assert_eq!(approved.status, PlanStatus::Approved);
        assert!(approved.operations[0].accepted);
    }

    #[test]
    fn approve_safe_plan() {
        let db = fresh_db();
        {
            let conn = db.0.lock().unwrap();
            persist_plan(&conn, &draft_plan("ps", vec![op("o1", SafetyVerdict::Safe)])).unwrap();
        }
        let approved = approve("ps", &[], &db).unwrap();
        assert_eq!(approved.status, PlanStatus::Approved);
    }
}
