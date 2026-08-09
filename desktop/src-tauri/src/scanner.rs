//! Scan orchestration: compute a residue graph, persist it, read it back.

use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};

use crate::db::{now_millis, Db};
use crate::domain::*;
use crate::error::{AppError, AppResult};
use crate::{backends, residue};

pub fn create_scan(canonical_app_id: String, db: &Db) -> AppResult<ResidueGraph> {
    let app = backends::inventory()?
        .into_iter()
        .find(|a| a.canonical_app_id == canonical_app_id)
        .ok_or_else(|| AppError::NotFound(format!("application {canonical_app_id}")))?;

    if app.is_protected {
        return Err(AppError::ProtectedApp(app.name.clone()));
    }

    let mut artifacts = backends::residue(&app)?;
    artifacts.sort_by(|a, b| a.target.cmp(&b.target));
    artifacts.dedup_by(|a, b| a.target == b.target);
    let runtime_warnings = residue::detect_runtime_warnings(&app.name);
    let counts = residue::count_usage(&artifacts);

    let conn = db.0.lock().unwrap();
    // Persist the app + package instances first (scans/artifacts FK → applications).
    upsert_application(&conn, &app)?;
    upsert_package_instances(&conn, &app)?;
    let scan_version = next_scan_version(&conn, &canonical_app_id)?;
    let sealed_at = now_millis();
    persist_scan(&conn, &canonical_app_id, scan_version, sealed_at, &artifacts)?;
    drop(app);

    Ok(ResidueGraph {
        application_id: canonical_app_id,
        scan_version,
        sealed_at,
        artifacts,
        counts,
        runtime_warnings,
    })
}

pub fn get_scan(canonical_app_id: String, scan_version: u32, db: &Db) -> AppResult<ResidueGraph> {
    let conn = db.0.lock().unwrap();
    let sealed_at: i64 = conn
        .query_row(
            "SELECT sealed_at FROM scans WHERE canonical_app_id=?1 AND scan_version=?2",
            params![canonical_app_id, scan_version],
            |r| r.get(0),
        )
        .map_err(|_| AppError::NotFound(format!("scan {canonical_app_id} v{scan_version}")))?;
    let artifacts = load_artifacts(&conn, &canonical_app_id, scan_version)?;
    Ok(ResidueGraph {
        application_id: canonical_app_id,
        scan_version,
        sealed_at,
        counts: residue::count_usage(&artifacts),
        artifacts,
        runtime_warnings: Vec::new(),
    })
}

fn next_scan_version(conn: &Connection, app_id: &str) -> AppResult<u32> {
    let max: i64 = conn.query_row(
        "SELECT COALESCE(MAX(scan_version),0) FROM scans WHERE canonical_app_id=?1",
        params![app_id],
        |r| r.get(0),
    )?;
    Ok(max as u32 + 1)
}

fn upsert_application(conn: &Connection, app: &CanonicalApplication) -> AppResult<()> {
    let (de_path, de_appid) = app
        .desktop_entry
        .as_ref()
        .map(|d| (Some(d.path.clone()), d.app_id.clone()))
        .unwrap_or((None, None));
    conn.execute(
        "INSERT INTO applications
         (canonical_app_id, name, desktop_entry_path, desktop_app_id, is_protected,
          disambiguated, first_seen_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(canonical_app_id) DO UPDATE SET
           name=excluded.name, desktop_entry_path=excluded.desktop_entry_path,
           desktop_app_id=excluded.desktop_app_id, is_protected=excluded.is_protected,
           disambiguated=excluded.disambiguated",
        params![
            app.canonical_app_id,
            app.name,
            de_path,
            de_appid,
            app.is_protected as i64,
            app.instances_disambiguated as i64,
            now_millis(),
        ],
    )?;
    Ok(())
}

fn upsert_package_instances(conn: &Connection, app: &CanonicalApplication) -> AppResult<()> {
    for r in &app.package_instance_refs {
        let id = format!("{}:{}", app.canonical_app_id, r.package_name);
        let backend = serde_json::to_value(&r.backend)?
            .as_str()
            .unwrap_or("")
            .to_string();
        let scope = r.scope.as_ref().and_then(|s| {
            serde_json::to_value(s)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
        });
        conn.execute(
            "INSERT OR REPLACE INTO package_instances
             (package_instance_id, canonical_app_id, backend, package_name, version, scope)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![id, app.canonical_app_id, backend, r.package_name, r.version, scope],
        )?;
    }
    Ok(())
}

/// Serialize an enum to its kebab-case string form (e.g. `Binary` → "binary").
fn jstr<T: Serialize>(v: &T) -> AppResult<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default())
}

/// Parse an enum back from its kebab-case string.
fn eparse<T: DeserializeOwned>(s: &str) -> AppResult<T> {
    Ok(serde_json::from_value(serde_json::Value::String(s.to_owned()))?)
}

fn persist_scan(
    conn: &Connection,
    app_id: &str,
    version: u32,
    sealed_at: i64,
    artifacts: &[Artifact],
) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO scans (canonical_app_id, scan_version, sealed_at) VALUES (?1,?2,?3)",
        params![app_id, version, sealed_at],
    )?;
    conn.execute(
        "DELETE FROM artifacts WHERE canonical_app_id=?1 AND scan_version=?2",
        params![app_id, version],
    )?;
    for a in artifacts {
        let scope: Option<String> = match &a.scope {
            Some(s) => Some(jstr(s)?),
            None => None,
        };
        conn.execute(
            "INSERT INTO artifacts
             (artifact_id, canonical_app_id, scan_version, category, target, owner_set,
              usage_kind, size_bytes, deletable, confidence, discovered_by, scope)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                a.artifact_id,
                app_id,
                version,
                jstr(&a.category)?,
                a.target,
                serde_json::to_string(&a.owner_set)?,
                jstr(&a.usage_kind)?,
                a.size_bytes.map(|v| v as i64),
                a.deletable as i64,
                jstr(&a.confidence)?,
                jstr(&a.discovered_by)?,
                scope,
            ],
        )?;
    }
    Ok(())
}

pub fn load_artifacts(conn: &Connection, app_id: &str, version: u32) -> AppResult<Vec<Artifact>> {
    let mut stmt = conn.prepare(
        "SELECT artifact_id, category, target, owner_set, usage_kind, size_bytes, deletable,
                confidence, discovered_by, scope
         FROM artifacts WHERE canonical_app_id=?1 AND scan_version=?2 ORDER BY target",
    )?;
    let rows = stmt.query_map(params![app_id, version], |r| {
        Ok(ArtifactRow {
            artifact_id: r.get(0)?,
            category: r.get(1)?,
            target: r.get(2)?,
            owner_set: r.get(3)?,
            usage_kind: r.get(4)?,
            size_bytes: r.get(5)?,
            deletable: r.get(6)?,
            confidence: r.get(7)?,
            discovered_by: r.get(8)?,
            scope: r.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let r = row?;
        out.push(Artifact {
            artifact_id: r.artifact_id,
            category: eparse(&r.category).unwrap_or(ArtifactCategory::Data),
            target: r.target,
            owner_set: serde_json::from_str(&r.owner_set).unwrap_or_default(),
            usage_kind: eparse(&r.usage_kind).unwrap_or(UsageKind::Exclusive),
            size_bytes: r.size_bytes.map(|v| v as u64),
            deletable: r.deletable != 0,
            confidence: eparse(&r.confidence).unwrap_or(ConfidenceLevel::Medium),
            discovered_by: eparse(&r.discovered_by).unwrap_or(DiscoverySource::Manifest),
            scope: r.scope.and_then(|s| eparse::<ScopeTag>(&s).ok()),
        });
    }
    Ok(out)
}

struct ArtifactRow {
    artifact_id: String,
    category: String,
    target: String,
    owner_set: String,
    usage_kind: String,
    size_bytes: Option<i64>,
    deletable: i64,
    confidence: String,
    discovered_by: String,
    scope: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn fresh_db() -> Db {
        let mut p = std::env::temp_dir();
        p.push(format!("ar-scan-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        Db::open(&p).expect("open")
    }

    fn sample_artifacts() -> Vec<Artifact> {
        vec![
            Artifact {
                artifact_id: "a1".into(),
                category: ArtifactCategory::Binary,
                target: "/usr/bin/curl".into(),
                owner_set: vec!["app1".into()],
                usage_kind: UsageKind::Exclusive,
                size_bytes: Some(2048),
                deletable: true,
                confidence: ConfidenceLevel::High,
                discovered_by: DiscoverySource::Manifest,
                scope: Some(ScopeTag::System),
            },
            Artifact {
                artifact_id: "a2".into(),
                category: ArtifactCategory::Config,
                target: "/home/u/.config/curl".into(),
                owner_set: vec!["app1".into()],
                usage_kind: UsageKind::Exclusive,
                size_bytes: None,
                deletable: true,
                confidence: ConfidenceLevel::Medium,
                discovered_by: DiscoverySource::Heuristic,
                scope: Some(ScopeTag::User),
            },
        ]
    }

    #[test]
    fn scan_persists_and_round_trips() {
        let db = fresh_db();
        let conn = db.0.lock().unwrap();
        // artifacts/scans FK → applications; seed the parent row first.
        conn.execute(
            "INSERT INTO applications (canonical_app_id, name, first_seen_at) VALUES (?1,?2,?3)",
            params!["app1", "App One", 0],
        )
        .unwrap();
        let v = next_scan_version(&conn, "app1").unwrap();
        assert_eq!(v, 1);
        persist_scan(&conn, "app1", 1, 1234, &sample_artifacts()).unwrap();
        // second create increments version
        let v2 = next_scan_version(&conn, "app1").unwrap();
        assert_eq!(v2, 2);
        let loaded = load_artifacts(&conn, "app1", 1).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].target, "/home/u/.config/curl"); // ordered by target
        assert_eq!(loaded[1].target, "/usr/bin/curl");
        assert_eq!(loaded[1].category, ArtifactCategory::Binary);
        assert_eq!(loaded[1].scope, Some(ScopeTag::System));
        assert_eq!(loaded[0].scope, Some(ScopeTag::User));
        assert_eq!(loaded[1].size_bytes, Some(2048));
        assert!(loaded[1].deletable);
    }
}
