//! AppImage backend — heuristic scan of common dirs for `*.AppImage` files.
//! The file itself is the "package" (deleted via uninstall); integration
//! desktop files are residue.

use std::path::Path;

use crate::domain::*;
use crate::error::AppResult;
use crate::policy;

use super::{canonical_id, residue_paths, PackageManager};

pub struct AppImage;

impl PackageManager for AppImage {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Appimage
    }
    fn present(&self) -> bool {
        true // heuristic — no single tool
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut apps = Vec::new();
        for dir in [format!("{home}/Applications"), format!("{home}/Downloads")] {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    let is_appimage = path
                        .extension()
                        .map(|x| x.eq_ignore_ascii_case("AppImage"))
                        .unwrap_or(false);
                    if !is_appimage {
                        continue;
                    }
                    let stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("appimage")
                        .to_string();
                    apps.push(build(&stem, &path.to_string_lossy()));
                }
            }
        }
        Ok(apps)
    }
    fn residue(&self, app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = app
            .package_instance_refs
            .first()
            .map(|r| r.package_name.as_str())
            .unwrap_or("");
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Ok(residue_paths(
            app,
            &[(
                format!("{home}/.local/share/applications/appimagekit_{stem}.desktop"),
                ArtifactCategory::DesktopEntry,
            )],
        ))
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}

fn build(stem: &str, path: &str) -> CanonicalApplication {
    let canonical_app_id = canonical_id(PackageBackend::Appimage, path);
    let pkg_ref = PackageInstanceRef {
        backend: PackageBackend::Appimage,
        package_name: path.to_string(),
        version: None,
        scope: Some(ScopeTag::User),
    };
    CanonicalApplication {
        canonical_app_id,
        name: stem.to_string(),
        desktop_entry: None,
        install_sources: vec![InstallSource {
            method: InstallMethod::Appimage,
            confidence: ConfidenceLevel::High,
            evidence: vec![Evidence {
                kind: EvidenceKind::AppimageMagic,
                detail: "AppImage file".into(),
            }],
            package_ref: Some(pkg_ref.clone()),
        }],
        package_instance_refs: vec![pkg_ref],
        is_protected: policy::is_protected(stem),
        instances_disambiguated: true,
    }
}
