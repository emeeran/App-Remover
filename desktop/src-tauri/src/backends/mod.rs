//! Package-manager adapters. A uniform query surface across backends; each
//! adapter detects presence, enumerates installed apps, and (in later
//! milestones) computes residue and executes removal.

use std::path::Path;

use crate::domain::{
    Artifact, ArtifactCategory, CanonicalApplication, ConfidenceLevel, Evidence, EvidenceKind,
    InstallMethod, InstallSource, PackageBackend, PackageInstanceRef, ScopeTag,
};
use crate::dto::BackendStatus;
use crate::error::AppResult;
use crate::{policy, residue};

mod apt;
mod appimage;
mod flatpak;
mod npm;
mod pip;
mod snap;
mod systemd;

use apt::AptBackend;
pub use apt::owning_package;

pub trait PackageManager: Send + Sync {
    fn backend(&self) -> PackageBackend;
    fn present(&self) -> bool;
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>>;
    /// Residue artifacts for one application (SPEC §3.3 ResidueGraph input).
    fn residue(&self, app: &CanonicalApplication) -> AppResult<Vec<Artifact>>;
    /// Packages that depend on this app's package (for plan impact/verdicts).
    fn reverse_deps(&self, app: &CanonicalApplication) -> AppResult<Vec<String>>;
}

/// Honest tool-presence detection for all 8 backends (by known-path existence).
/// AppImage/manual have no single tool and are always considered available.
pub fn detect_backends() -> Vec<BackendStatus> {
    let checks: &[(PackageBackend, &str)] = &[
        (PackageBackend::Deb, "/usr/bin/dpkg"),
        (PackageBackend::Snap, "/usr/bin/snap"),
        (PackageBackend::Flatpak, "/usr/bin/flatpak"),
        (PackageBackend::Pip, "/usr/bin/pip3"),
        (PackageBackend::Npm, "/usr/bin/npm"),
        (PackageBackend::Systemd, "/usr/bin/systemctl"),
        (PackageBackend::Appimage, ""),
        (PackageBackend::Manual, ""),
    ];
    checks
        .iter()
        .map(|(backend, path)| BackendStatus {
            backend: *backend,
            present: path.is_empty() || Path::new(path).exists(),
        })
        .collect()
}

/// The real adapters currently implemented (M7: all eight backends).
fn real_adapters() -> Vec<Box<dyn PackageManager>> {
    vec![
        Box::new(AptBackend),
        Box::new(snap::Snap),
        Box::new(flatpak::Flatpak),
        Box::new(pip::Pip),
        Box::new(npm::Npm),
        Box::new(systemd::Systemd),
        Box::new(appimage::AppImage),
    ]
}

/// Flat list of installed apps from every present real adapter.
pub fn inventory() -> AppResult<Vec<CanonicalApplication>> {
    let mut apps = Vec::new();
    for adapter in real_adapters() {
        if adapter.present() {
            apps.extend(adapter.list_apps()?);
        }
    }
    Ok(apps)
}

/// Residue for an app, dispatched to the adapter matching its backend.
pub fn residue(app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
    let backend = app.package_instance_refs.first().map(|r| r.backend);
    for adapter in real_adapters() {
        if Some(adapter.backend()) == backend && adapter.present() {
            return adapter.residue(app);
        }
    }
    // No adapter (e.g., manual installs) → no package-managed residue.
    Ok(Vec::new())
}

/// Reverse dependencies for an app, dispatched to its adapter.
pub fn reverse_deps(app: &CanonicalApplication) -> AppResult<Vec<String>> {
    let backend = app.package_instance_refs.first().map(|r| r.backend);
    for adapter in real_adapters() {
        if Some(adapter.backend()) == backend && adapter.present() {
            return adapter.reverse_deps(app);
        }
    }
    Ok(Vec::new())
}

/// Deterministic canonical id for a `<backend>:<name>` pair — matches the ids
/// produced by the apt adapter so impact lists line up with inventory ids.
pub fn canonical_id(backend: PackageBackend, name: &str) -> String {
    let b = serde_json::to_value(backend)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_X500,
        format!("{b}:{name}").as_bytes(),
    )
    .to_string()
}

/// The app's primary package name (first package instance ref), or its name.
pub fn pkg_name(app: &CanonicalApplication) -> &str {
    app.package_instance_refs
        .first()
        .map(|r| r.package_name.as_str())
        .unwrap_or(&app.name)
}

/// Build a single-source CanonicalApplication for a backend (shared by the
/// non-apt adapters).
pub fn make_app(
    backend: PackageBackend,
    method: InstallMethod,
    name: &str,
    version: Option<String>,
    ev_kind: EvidenceKind,
    ev_detail: &str,
) -> CanonicalApplication {
    let canonical_app_id = canonical_id(backend, name);
    let pkg_ref = PackageInstanceRef {
        backend,
        package_name: name.to_string(),
        version: version.clone(),
        scope: Some(ScopeTag::System),
    };
    CanonicalApplication {
        canonical_app_id,
        name: name.to_string(),
        desktop_entry: None,
        install_sources: vec![InstallSource {
            method,
            confidence: ConfidenceLevel::High,
            evidence: vec![Evidence {
                kind: ev_kind,
                detail: ev_detail.to_string(),
            }],
            package_ref: Some(pkg_ref.clone()),
        }],
        package_instance_refs: vec![pkg_ref],
        is_protected: policy::is_protected(name),
        instances_disambiguated: true,
    }
}

/// Build residue artifacts from candidate `(path, category)` pairs, keeping
/// only those that exist on disk.
pub fn residue_paths(app: &CanonicalApplication, candidates: &[(String, ArtifactCategory)]) -> Vec<Artifact> {
    let owner = app.canonical_app_id.clone();
    candidates
        .iter()
        .filter(|(p, _)| Path::new(p).exists())
        .map(|(p, c)| {
            residue::make_artifact(p, *c, &owner, crate::domain::DiscoverySource::Heuristic, ConfidenceLevel::Medium)
        })
        .collect()
}
