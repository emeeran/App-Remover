//! Pre-removal snapshots enabling undo (SPEC §3.6, NFR-15).
//!
//! Captures leftover config/data artifacts (files and directory trees) into a
//! blob root and records the package for reinstall. Integrity is anchored by a
//! sha256 manifest over all entries; `verify` recomputes it before any restore.

use std::path::Path;

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::db::now_millis;
use crate::domain::{Artifact, PackageBackend, Snapshot, SnapshotEntry, SnapshotEntryKind};
use crate::error::{AppError, AppResult};
use crate::executor::{copy_dir, Executor};

fn jstr<T: serde::Serialize>(v: &T) -> AppResult<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default())
}

/// sha256 of a single file, or a sorted (rel-path, file-hash) digest of a dir.
fn hash_tree(path: &Path) -> AppResult<(String, u64)> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_file() {
        let data = std::fs::read(path)?;
        let size = data.len() as u64;
        Ok((hex::encode(Sha256::digest(&data)), size))
    } else if md.is_dir() {
        let mut items: Vec<(String, String, u64)> = Vec::new();
        gather(path, path, &mut items)?;
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut h = Sha256::new();
        let mut size = 0u64;
        for (rel, chash, sz) in &items {
            h.update(rel.as_bytes());
            h.update(b"|");
            h.update(chash.as_bytes());
            h.update(b"\n");
            size += sz;
        }
        Ok((hex::encode(h.finalize()), size))
    } else {
        Ok((hex::encode(Sha256::digest(path.to_string_lossy().as_bytes())), 0))
    }
}

fn gather(root: &Path, dir: &Path, items: &mut Vec<(String, String, u64)>) -> AppResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            gather(root, &p, items)?;
        } else {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned();
            let data = std::fs::read(&p)?;
            items.push((rel, hex::encode(Sha256::digest(&data)), data.len() as u64));
        }
    }
    Ok(())
}

/// sha256 over sorted entries of "kind|original_path|checksum".
pub fn manifest_checksum(entries: &[SnapshotEntry]) -> String {
    let mut sorted: Vec<&SnapshotEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.original_path.cmp(&b.original_path));
    let mut h = Sha256::new();
    for e in sorted {
        let kind = serde_json::to_value(&e.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        h.update(kind.as_bytes());
        h.update(b"|");
        h.update(e.original_path.as_bytes());
        h.update(b"|");
        h.update(e.checksum.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

/// Capture a snapshot: back up each leftover artifact (file or dir tree) into
/// `<blob_root>/<snapshot_id>/` and record the package for reinstall.
pub fn capture(
    job_id: &str,
    package_name: &str,
    package_version: Option<&str>,
    to_backup: &[Artifact],
    blob_root: &Path,
) -> AppResult<Snapshot> {
    let snapshot_id = uuid::Uuid::new_v4().to_string();
    let blob_dir = blob_root.join(&snapshot_id);
    std::fs::create_dir_all(&blob_dir)?;

    let mut entries = Vec::new();

    // Package record (reinstall target) — checksum of the canonical record.
    let rec = format!("deb|{package_name}|{}", package_version.unwrap_or(""));
    entries.push(SnapshotEntry {
        artifact_id: None,
        category: "package".into(),
        original_path: format!("package:{package_name}"),
        blob_path: None,
        checksum: hex::encode(Sha256::digest(rec.as_bytes())),
        size_bytes: 0,
        kind: SnapshotEntryKind::PackageRecord,
        package_name: Some(package_name.to_string()),
    });

    for (i, a) in to_backup.iter().enumerate() {
        let p = Path::new(&a.target);
        if !p.exists() {
            continue;
        }
        let (checksum, size) = hash_tree(p)?;
        let blob_path = blob_dir.join(format!("b{i}"));
        if p.is_dir() {
            copy_dir(p, &blob_path)?;
        } else {
            std::fs::copy(p, &blob_path)?;
        }
        let category = jstr(&a.category)?;
        entries.push(SnapshotEntry {
            artifact_id: Some(a.artifact_id.clone()),
            category,
            original_path: a.target.clone(),
            blob_path: Some(blob_path.to_string_lossy().into_owned()),
            checksum,
            size_bytes: size,
            kind: SnapshotEntryKind::FileBackup,
            package_name: None,
        });
    }

    let total_bytes: u64 = entries.iter().map(|e| e.size_bytes).sum();
    Ok(Snapshot {
        snapshot_id,
        job_id: job_id.to_string(),
        captured_at: now_millis(),
        checksum_algo: "sha256".into(),
        manifest_checksum: manifest_checksum(&entries),
        total_bytes,
        excludes_cache: true,
        blob_root: blob_root.to_string_lossy().into_owned(),
        entries,
    })
}

/// Verify-before-restore (NFR-15): recompute each blob's checksum + the
/// manifest; refuse (SnapshotCorrupt) on any mismatch.
pub fn verify(snap: &Snapshot) -> AppResult<()> {
    for e in &snap.entries {
        if matches!(e.kind, SnapshotEntryKind::FileBackup) {
            if let Some(bp) = &e.blob_path {
                let (h, _) = hash_tree(Path::new(bp))?;
                if h != e.checksum {
                    return Err(AppError::SnapshotCorrupt(format!(
                        "blob checksum mismatch for {}",
                        e.original_path
                    )));
                }
            }
        }
    }
    if manifest_checksum(&snap.entries) != snap.manifest_checksum {
        return Err(AppError::SnapshotCorrupt("manifest checksum mismatch".into()));
    }
    Ok(())
}

/// Restore a verified snapshot: replay file backups + reinstall the package.
pub fn restore(snap: &Snapshot, backend: PackageBackend, executor: &dyn Executor) -> AppResult<usize> {
    let mut n = 0;
    for e in &snap.entries {
        match e.kind {
            SnapshotEntryKind::FileBackup => {
                if let Some(bp) = &e.blob_path {
                    executor.restore_file(bp, &e.original_path)?;
                    n += 1;
                }
            }
            SnapshotEntryKind::PackageRecord => {
                if let Some(pkg) = &e.package_name {
                    executor.reinstall_package(backend, pkg)?;
                    n += 1;
                }
            }
            SnapshotEntryKind::UnitRecord => {}
        }
    }
    Ok(n)
}

// --------------------------- persistence ---------------------------

pub fn persist_snapshot(conn: &Connection, snap: &Snapshot) -> AppResult<()> {
    // Children first: snapshot_entries FK is ON DELETE RESTRICT.
    conn.execute(
        "DELETE FROM snapshot_entries WHERE snapshot_id=?1",
        params![snap.snapshot_id],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO snapshots
         (snapshot_id, job_id, captured_at, checksum_algo, manifest_checksum,
          total_bytes, excludes_cache, blob_root)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            snap.snapshot_id,
            snap.job_id,
            snap.captured_at,
            snap.checksum_algo,
            snap.manifest_checksum,
            snap.total_bytes as i64,
            snap.excludes_cache as i64,
            snap.blob_root,
        ],
    )?;
    for e in &snap.entries {
        conn.execute(
            "INSERT INTO snapshot_entries
             (snapshot_id, artifact_id, category, original_path, blob_path, checksum,
              size_bytes, kind, package_name)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                snap.snapshot_id,
                e.artifact_id,
                e.category,
                e.original_path,
                e.blob_path,
                e.checksum,
                e.size_bytes as i64,
                jstr(&e.kind)?,
                e.package_name,
            ],
        )?;
    }
    Ok(())
}

pub fn load_snapshot(conn: &Connection, snapshot_id: &str) -> AppResult<Snapshot> {
    let row = conn
        .query_row(
            "SELECT job_id, captured_at, checksum_algo, manifest_checksum, total_bytes,
                    excludes_cache, blob_root FROM snapshots WHERE snapshot_id=?1",
            params![snapshot_id],
            |r| {
                Ok(SnapRow {
                    job_id: r.get(0)?,
                    captured_at: r.get(1)?,
                    checksum_algo: r.get(2)?,
                    manifest_checksum: r.get(3)?,
                    total_bytes: r.get(4)?,
                    excludes_cache: r.get(5)?,
                    blob_root: r.get(6)?,
                })
            },
        )
        .map_err(|_| AppError::NotFound(format!("snapshot {snapshot_id}")))?;

    let mut stmt = conn.prepare(
        "SELECT artifact_id, category, original_path, blob_path, checksum, size_bytes, kind,
                package_name FROM snapshot_entries WHERE snapshot_id=?1 ORDER BY original_path",
    )?;
    let mut rows = stmt.query_map(params![snapshot_id], |r| {
        Ok(EntryRow {
            artifact_id: r.get(0)?,
            category: r.get(1)?,
            original_path: r.get(2)?,
            blob_path: r.get(3)?,
            checksum: r.get(4)?,
            size_bytes: r.get(5)?,
            kind: r.get(6)?,
            package_name: r.get(7)?,
        })
    })?;
    let mut entries = Vec::new();
    for row in rows.by_ref() {
        let r = row?;
        entries.push(SnapshotEntry {
            artifact_id: r.artifact_id,
            category: r.category,
            original_path: r.original_path,
            blob_path: r.blob_path,
            checksum: r.checksum,
            size_bytes: r.size_bytes as u64,
            kind: serde_json::from_value(serde_json::Value::String(r.kind))
                .unwrap_or(SnapshotEntryKind::FileBackup),
            package_name: r.package_name,
        });
    }

    Ok(Snapshot {
        snapshot_id: snapshot_id.to_string(),
        job_id: row.job_id,
        captured_at: row.captured_at,
        checksum_algo: row.checksum_algo,
        manifest_checksum: row.manifest_checksum,
        total_bytes: row.total_bytes as u64,
        excludes_cache: row.excludes_cache != 0,
        blob_root: row.blob_root,
        entries,
    })
}

struct SnapRow {
    job_id: String,
    captured_at: i64,
    checksum_algo: String,
    manifest_checksum: String,
    total_bytes: i64,
    excludes_cache: i64,
    blob_root: String,
}
struct EntryRow {
    artifact_id: Option<String>,
    category: String,
    original_path: String,
    blob_path: Option<String>,
    checksum: String,
    size_bytes: i64,
    kind: String,
    package_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PackageBackend;
    use std::sync::Mutex;

    struct FakeExec {
        restored: Mutex<Vec<String>>,
        reinstalled: Mutex<Vec<String>>,
    }
    impl Executor for FakeExec {
        fn uninstall_package(&self, _: PackageBackend, _: &str, _: bool) -> AppResult<()> {
            Ok(())
        }
        fn reinstall_package(&self, _: PackageBackend, p: &str) -> AppResult<()> {
            self.reinstalled.lock().unwrap().push(p.into());
            Ok(())
        }
        fn delete_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        fn restore_file(&self, blob: &str, orig: &str) -> AppResult<()> {
            let b = Path::new(blob);
            if b.is_dir() {
                crate::executor::copy_dir(b, Path::new(orig))?;
            } else {
                std::fs::copy(blob, orig)?;
            }
            self.restored.lock().unwrap().push(orig.into());
            Ok(())
        }
    }

    fn write_tree(root: &Path) {
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.conf"), b"alpha").unwrap();
        std::fs::write(root.join("sub/b.dat"), b"beta").unwrap();
    }

    fn art(target: &str) -> Artifact {
        Artifact {
            artifact_id: uuid::Uuid::new_v4().to_string(),
            category: crate::domain::ArtifactCategory::Config,
            target: target.into(),
            owner_set: vec!["app1".into()],
            usage_kind: crate::domain::UsageKind::Exclusive,
            size_bytes: None,
            deletable: true,
            confidence: crate::domain::ConfidenceLevel::High,
            discovered_by: crate::domain::DiscoverySource::Heuristic,
            scope: Some(crate::domain::ScopeTag::User),
        }
    }

    #[test]
    fn capture_verify_restore_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("ar-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let src_dir = tmp.join("src_cfg");
        write_tree(&src_dir);
        let src_file = tmp.join("f.txt");
        std::fs::write(&src_file, b"hello").unwrap();

        let blob_root = tmp.join("blobs");
        let snap = capture(
            "job1",
            "foo",
            Some("1.0"),
            &[art(src_dir.to_str().unwrap()), art(src_file.to_str().unwrap())],
            &blob_root,
        )
        .unwrap();
        // package record + 2 backups
        assert_eq!(snap.entries.len(), 3);
        assert!(snap.entries.iter().any(|e| matches!(e.kind, SnapshotEntryKind::PackageRecord)));
        verify(&snap).expect("verify ok");

        // remove originals, then restore
        std::fs::remove_dir_all(&src_dir).unwrap();
        std::fs::remove_file(&src_file).unwrap();
        let fake = FakeExec { restored: Mutex::new(vec![]), reinstalled: Mutex::new(vec![]) };
        let n = restore(&snap, PackageBackend::Deb, &fake).unwrap();
        assert_eq!(n, 3);
        assert!(src_dir.join("a.conf").exists());
        assert!(src_file.exists());
        assert_eq!(fake.reinstalled.lock().unwrap().as_slice(), &["foo".to_string()]);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn verify_detects_tamper() {
        let tmp = std::env::temp_dir().join(format!("ar-snap-t-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("g.txt");
        std::fs::write(&src, b"original").unwrap();
        let snap = capture("job1", "foo", None, &[art(src.to_str().unwrap())], &tmp.join("blobs")).unwrap();
        // tamper the backup blob
        let blob = snap.entries[1].blob_path.clone().unwrap();
        std::fs::write(&blob, b"TAMPERED").unwrap();
        let err = verify(&snap).unwrap_err();
        assert!(matches!(err, AppError::SnapshotCorrupt(_)));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
