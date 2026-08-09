//! npm backend — global packages via `npm list -g --json`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::domain::*;
use crate::error::AppResult;
use crate::exec::run_capture;

use super::{make_app, PackageManager};

#[derive(Deserialize)]
struct NpmOut {
    #[serde(default)]
    dependencies: HashMap<String, NpmDep>,
}
#[derive(Deserialize)]
struct NpmDep {
    #[serde(default)]
    version: Option<String>,
}

pub struct Npm;

impl PackageManager for Npm {
    fn backend(&self) -> PackageBackend {
        PackageBackend::Npm
    }
    fn present(&self) -> bool {
        Path::new("/usr/bin/npm").exists()
    }
    fn list_apps(&self) -> AppResult<Vec<CanonicalApplication>> {
        let out = run_capture("npm", &["list", "-g", "--json", "--depth=0"])?;
        let parsed: NpmOut = serde_json::from_str(&out).unwrap_or(NpmOut {
            dependencies: HashMap::new(),
        });
        Ok(parsed
            .dependencies
            .into_iter()
            .map(|(name, dep)| {
                make_app(
                    PackageBackend::Npm,
                    InstallMethod::LanguagePackage,
                    &name,
                    dep.version,
                    EvidenceKind::NpmRecord,
                    "npm list -g",
                )
            })
            .collect())
    }
    fn residue(&self, _app: &CanonicalApplication) -> AppResult<Vec<Artifact>> {
        Ok(Vec::new())
    }
    fn reverse_deps(&self, _app: &CanonicalApplication) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
}
