//! systemd backend — user service units via `systemctl --user list-unit-files`.

use std::path::Path;

use crate::domain::*;
use crate::error::AppResult;
use crate::exec::run_capture;

use super::{make_app, pkg_name, residue_paths, PackageManager};

pub struct Systemd;

impl PackageManager for Systemd {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Systemd
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/systemctl").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let out = run_capture(
            "systemctl",
            &["--user", "list-unit-files", "--type=service", "--no-legend"],
        )?;
        let mut apps = Vec::new();
        for line in out.lines() {
            let name = line.split_whitespace().next().unwrap_or("");
            if !name.ends_with(".service") {
                continue;
            }
            apps.push(make_app(
                PackageBackend::Systemd,
                InstallMethod::Manual,
                name,
                None,
                EvidenceKind::FilePath,
                "systemctl list-unit-files",
            ));
        }
        Ok(apps)
    }
    fn residue(&self, app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        let unit = pkg_name(app);
        let home = std::env::var("HOME").unwrap_or_default();
        Ok(residue_paths(
            app,
            &[
                (format!("{home}/.config/systemd/user/{unit}"), ArtifactCategory::Service),
                (format!("{home}/.local/share/systemd/user/{unit}"), ArtifactCategory::Service),
            ],
        ))
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}
