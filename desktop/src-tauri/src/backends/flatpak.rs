//! flatpak backend — inventory via `flatpak list`, residue under .var/app.

use std::path::Path;

use crate::domain::*;
use crate::error::AppResult;
use crate::exec::run_capture;

use super::{make_app, pkg_name, residue_paths, PackageManager};

pub struct Flatpak;

impl PackageManager for Flatpak {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Flatpak
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/flatpak").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let out = run_capture(
            "flatpak",
            &["list", "--app", "--columns=application,version"],
        )?;
        let mut apps = Vec::new();
        for line in out.lines() {
            let mut it = line.splitn(2, '\t');
            let Some(name) = it.next() else { continue };
            if name.is_empty() {
                continue;
            }
            let version = it.next().unwrap_or("").trim().to_string();
            apps.push(make_app(
                PackageBackend::Flatpak,
                InstallMethod::Flatpak,
                name,
                Some(version),
                EvidenceKind::FlatpakList,
                "flatpak list",
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
                (format!("{home}/.var/app/{name}"), ArtifactCategory::Data),
                (format!("/var/lib/flatpak/app/{name}"), ArtifactCategory::Binary),
            ],
        ))
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}
