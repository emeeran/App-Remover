//! snap backend — inventory via `snap list`, residue at snap mount/data dirs.

use std::path::Path;

use crate::domain::*;
use crate::error::AppResult;
use crate::exec::run_capture;

use super::{make_app, pkg_name, residue_paths, PackageManager};

pub struct Snap;

impl PackageManager for Snap {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Snap
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/snap").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let out = run_capture("snap", &["list"])?;
        let mut apps = Vec::new();
        for line in out.lines() {
            let mut it = line.split_whitespace();
            let Some(name) = it.next() else { continue };
            if name == "Name" {
                continue; // header
            }
            let version = it.next().unwrap_or("").to_string();
            apps.push(make_app(
                PackageBackend::Snap,
                InstallMethod::Snap,
                name,
                Some(version),
                EvidenceKind::SnapList,
                "snap list",
            ));
        }
        Ok(apps)
    }
    fn residue(&self, app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        let name = pkg_name(app);
        let home = std::env::var("HOME").unwrap_or_default();
        Ok(residue_paths(
            app,
            &[
                (format!("/snap/{name}"), ArtifactCategory::Binary),
                (format!("/var/snap/{name}"), ArtifactCategory::Data),
                (format!("{home}/snap/{name}"), ArtifactCategory::Data),
            ],
        ))
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}
