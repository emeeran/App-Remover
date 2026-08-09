//! deb/apt backend — real inventory via `dpkg-query`.

use std::path::Path;

use regex::Regex;
use uuid::Uuid;

use crate::domain::{
    Artifact, ArtifactCategory, CanonicalApplication, ConfidenceLevel, DiscoverySource, Evidence,
    EvidenceKind, InstallMethod, InstallSource, PackageBackend, PackageInstanceRef, ScopeTag,
    UsageKind,
};
use crate::error::{AppError, AppResult};
use crate::exec::run_capture;
use crate::{policy, residue};

pub struct AptBackend;

// Debian package name: lowercase, digits, +, ., - (policy §5.6.7).
const NAME_RE: &str = r"^[a-z0-9][a-z0-9+.\-]{0,255}$";

impl super::PackageManager for AptBackend {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Deb
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/dpkg").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        list_dpkg_apps()
    }
    fn residue(&self, app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        residue_for(app)
    }
    fn reverse_deps(&self, app: &CanonicalApplication) -> AppResult<Vec<String>> {
        let name = app
            .package_instance_refs
            .first()
            .map(|r| r.package_name.as_str())
            .unwrap_or(app.name.as_str());
        reverse_deps_for(name)
    }
}

fn list_dpkg_apps() -> AppResult<Vec<CanonicalApplication>> {
    let re = Regex::new(NAME_RE).map_err(|e| AppError::Internal(e.to_string()))?;
    // Tab-delimited: package \t version \t status. `${Package}` excludes arch.
    let out = run_capture(
        "dpkg-query",
        &["-W", "-f=${Package}\t${Version}\t${Status}\n"],
    )?;

    let mut apps = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(name), Some(version), Some(status)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // "install ok installed" — skip removed/config-files entries.
        if status != "install ok installed" {
            continue;
        }
        if !re.is_match(name) {
            continue;
        }
        apps.push(build_app(name, version));
    }
    Ok(apps)
}

fn build_app(name: &str, version: &str) -> CanonicalApplication {
    // Deterministic canonical id so the same package is stable across runs.
    let canonical_app_id =
        Uuid::new_v5(&Uuid::NAMESPACE_X500, format!("deb:{name}").as_bytes()).to_string();
    let package_ref = PackageInstanceRef {
        backend: PackageBackend::Deb,
        package_name: name.to_string(),
        version: Some(version.to_string()),
        scope: Some(ScopeTag::System),
    };
    CanonicalApplication {
        canonical_app_id,
        name: name.to_string(),
        desktop_entry: None,
        install_sources: vec![InstallSource {
            method: InstallMethod::Deb,
            confidence: ConfidenceLevel::High,
            evidence: vec![Evidence {
                kind: EvidenceKind::DpkgRecord,
                detail: "dpkg-query -W".into(),
            }],
            package_ref: Some(package_ref.clone()),
        }],
        package_instance_refs: vec![package_ref],
        is_protected: policy::is_protected(name),
        instances_disambiguated: true,
    }
}

/// Residue for a deb package: its `dpkg -L` files (stat-filtered to real
/// files/symlinks so shared dirs like `/usr` and `/usr/bin` are excluded) plus
/// D1 heuristic leftover paths the package manager won't remove.
fn residue_for(app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
    let name = app
        .package_instance_refs
        .first()
        .map(|r| r.package_name.as_str())
        .unwrap_or(app.name.as_str());
    let owner = app.canonical_app_id.clone();
    let mut artifacts = Vec::new();

    if let Ok(out) = run_capture("dpkg", &["-L", name]) {
        for line in out.lines() {
            let p = line.trim();
            if !p.starts_with('/') {
                continue;
            }
            // Only real files/symlinks — skip directories (the shared system dirs).
            let is_file = std::fs::symlink_metadata(p)
                .map(|m| m.is_file() || m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            artifacts.push(make_artifact(
                p,
                residue::classify(p),
                &owner,
                DiscoverySource::Manifest,
                ConfidenceLevel::High,
            ));
        }
    }

    for (path, category) in residue::leftover_paths(name) {
        if let Some(p) = path.to_str() {
            artifacts.push(make_artifact(
                p,
                category,
                &owner,
                DiscoverySource::Heuristic,
                ConfidenceLevel::Medium,
            ));
        }
    }

    artifacts.sort_by(|a, b| a.target.cmp(&b.target));
    artifacts.dedup_by(|a, b| a.target == b.target);
    Ok(artifacts)
}

fn make_artifact(
    target: &str,
    category: ArtifactCategory,
    owner: &str,
    discovered_by: DiscoverySource,
    confidence: ConfidenceLevel,
) -> Artifact {
    Artifact {
        artifact_id: Uuid::new_v4().to_string(),
        category,
        target: target.to_string(),
        owner_set: vec![owner.to_string()],
        usage_kind: UsageKind::Exclusive, // single owner; multi-owner detection in M4/M7
        size_bytes: None,
        deletable: true,
        confidence,
        discovered_by,
        scope: Some(if target.starts_with("/home/") {
            ScopeTag::User
        } else {
            ScopeTag::System
        }),
    }
}

/// `dpkg -S <path>` → the owning package (first of any co-owners), or None if
/// unowned. Used by resolve to map a launcher binary to its package.
pub fn owning_package(path: &str) -> Option<String> {
    let out = run_capture("dpkg", &["-S", path]).ok()?;
    let line = out.lines().next()?;
    // "pkg1, pkg2: /path" → take the first package token before ':'.
    let pkg = line.split(':').next()?.split(',').next()?.trim();
    Some(pkg.to_string())
}

/// `apt-cache rdepends --installed <pkg>` → names of installed packages that
/// depend on `pkg` (drives plan impact/verdicts).
fn reverse_deps_for(name: &str) -> AppResult<Vec<String>> {
    let out = run_capture("apt-cache", &["rdepends", "--installed", name])?;
    let mut deps = Vec::new();
    let mut after_header = false;
    for line in out.lines() {
        let t = line.trim();
        if t == "Reverse Depends:" {
            after_header = true;
            continue;
        }
        if !after_header {
            continue;
        }
        if t.is_empty() || t.starts_with('-') {
            continue;
        }
        let dep = t.split_whitespace().next().unwrap_or("");
        // strip :arch suffix
        let dep = dep.rsplit_once(':').map(|(n, _)| n).unwrap_or(dep);
        if !dep.is_empty() && dep != name {
            deps.push(dep.to_string());
        }
    }
    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_app_marks_protected_and_stable_id() {
        let a = build_app("libc6", "1.2.3");
        assert!(a.is_protected);
        let b = build_app("libc6", "9.9.9");
        // same package → same canonical id regardless of version
        assert_eq!(a.canonical_app_id, b.canonical_app_id);
        assert_eq!(a.install_sources[0].method, InstallMethod::Deb);
    }

    #[test]
    fn non_protected_app() {
        assert!(!build_app("firefox", "1.0").is_protected);
    }

    #[test]
    fn name_regex_accepts_and_rejects() {
        let re = Regex::new(NAME_RE).unwrap();
        assert!(re.is_match("firefox"));
        assert!(re.is_match("libstdc++6"));
        assert!(re.is_match("0ad"));
        assert!(!re.is_match("UPPER"));
        assert!(!re.is_match("a b"));
        assert!(!re.is_match(""));
    }

    #[test]
    fn apt_inventory_returns_real_packages() {
        // End-to-end against the live dpkg database; skipped on non-deb hosts.
        if !Path::new("/usr/bin/dpkg").exists() {
            eprintln!("skipping: no dpkg");
            return;
        }
        let apps = list_dpkg_apps().expect("dpkg-query should succeed");
        assert!(!apps.is_empty(), "expected installed packages");
        assert!(
            apps.iter().any(|a| a.name == "dpkg"),
            "expected to find the 'dpkg' package"
        );
        // Every parsed app must carry a deb install source + system scope.
        for a in &apps {
            assert!(!a.install_sources.is_empty());
            assert_eq!(a.install_sources[0].method, InstallMethod::Deb);
        }
    }

    #[test]
    fn apt_residue_returns_real_files() {
        // End-to-end residue against a known-installed package.
        if !Path::new("/usr/bin/dpkg").exists() {
            eprintln!("skipping: no dpkg");
            return;
        }
        let apps = list_dpkg_apps().expect("dpkg-query");
        let app = apps
            .iter()
            .find(|a| a.name == "coreutils")
            .or_else(|| apps.iter().find(|a| a.name == "bash"))
            .expect("expected coreutils or bash installed");
        let arts = residue_for(app).expect("residue for coreutils/bash");
        assert!(!arts.is_empty(), "expected residue files");
        // Owned exclusively by the app, and at least one binary present.
        for a in &arts {
            assert_eq!(a.owner_set, vec![app.canonical_app_id.clone()]);
        }
        assert!(
            arts.iter().any(|a| a.target.starts_with("/usr/")),
            "expected /usr files in residue"
        );
    }
}
