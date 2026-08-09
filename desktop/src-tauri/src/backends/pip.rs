//! pip backend — inventory via `pip3 list --format=json`.

use std::path::Path;

use serde::Deserialize;

use crate::domain::*;
use crate::error::AppResult;
use crate::exec::run_capture;

use super::{make_app, PackageManager};

#[derive(Deserialize)]
struct PipPkg {
    name: String,
    version: String,
}

pub struct Pip;

impl PackageManager for Pip {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Pip
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/pip3").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let out = run_capture("pip3", &["list", "--format=json"])?;
        let pkgs: Vec<PipPkg> = serde_json::from_str(&out).unwrap_or_default();
        Ok(pkgs
            .into_iter()
            .map(|p| {
                make_app(
                    PackageBackend::Pip,
                    InstallMethod::LanguagePackage,
                    &p.name,
                    Some(p.version),
                    EvidenceKind::PipRecord,
                    "pip3 list",
                )
            })
            .collect())
    }
    fn residue(&self, _app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        // pip uninstall removes site-packages entries; user data is app-specific
        // and not generally discoverable — no heuristic residue for M7.
        Ok(Vec::new())
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}
