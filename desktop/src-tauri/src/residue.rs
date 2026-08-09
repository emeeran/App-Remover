//! Residue computation helpers: path→category classification (D1 policy),
//! the owner-set→usageKind pure function (NFR-4 invariant), leftover-path
//! probing, usage tallies, and runtime-warning detection (D10).

use std::path::{Path, PathBuf};

use crate::domain::{
    Artifact, ArtifactCategory, ConfidenceLevel, DiscoverySource, ResidueCounts, RuntimeWarning,
    RuntimeWarningKind, ScopeTag, UsageKind,
};
use crate::exec::run_capture;

/// Classify a filesystem path into a residue category (D1).
pub fn classify(path: &str) -> ArtifactCategory {
    // Most-specific first: association lists can live under config or share dirs.
    if path.ends_with("mimeapps.list") || path.ends_with("defaults.list") {
        return ArtifactCategory::Association;
    }
    if path.ends_with(".desktop") || path.contains("/applications/") {
        return ArtifactCategory::DesktopEntry;
    }
    if path.ends_with(".service") || path.contains("/systemd/") {
        return ArtifactCategory::Service;
    }
    if path.starts_with("/run/") || path.starts_with("/var/run/") {
        return ArtifactCategory::State;
    }
    if path.starts_with("/etc/") {
        return ArtifactCategory::Config;
    }
    if path.contains("/.config/") {
        return ArtifactCategory::Config;
    }
    if path.starts_with("/var/cache/") || path.contains("/.cache/") {
        return ArtifactCategory::Cache;
    }
    if path.starts_with("/var/lib/") || path.contains("/.local/share/") || path.contains("/var/log/")
    {
        return ArtifactCategory::Data;
    }
    if path.starts_with("/usr/bin/")
        || path.starts_with("/usr/sbin/")
        || path.starts_with("/usr/lib/")
        || path.starts_with("/bin/")
        || path.starts_with("/sbin/")
        || path.starts_with("/lib/")
        || path.starts_with("/opt/")
    {
        return ArtifactCategory::Binary;
    }
    if path.starts_with("/usr/share/") || path.contains("/share/") {
        return ArtifactCategory::Data;
    }
    ArtifactCategory::Data
}

/// NFR-4 invariant: usageKind is a pure function of owner-set cardinality.
/// 0 owners → System (OS-owned, not deletable); 1 → Exclusive; >1 → Shared.
pub fn usage_kind(owner_set_len: usize) -> UsageKind {
    match owner_set_len {
        0 => UsageKind::System,
        1 => UsageKind::Exclusive,
        _ => UsageKind::Shared,
    }
}

/// Build a single-owner (exclusive) artifact. Scope is inferred from the path.
pub fn make_artifact(
    target: &str,
    category: ArtifactCategory,
    owner: &str,
    discovered_by: DiscoverySource,
    confidence: ConfidenceLevel,
) -> Artifact {
    Artifact {
        artifact_id: uuid::Uuid::new_v4().to_string(),
        category,
        target: target.to_string(),
        owner_set: vec![owner.to_string()],
        usage_kind: UsageKind::Exclusive,
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

/// D1 heuristic leftover locations for an app name (config/cache/data in both
/// user and system scope). Returns those that currently exist on disk.
pub fn leftover_paths(name: &str) -> Vec<(PathBuf, ArtifactCategory)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates: [(String, ArtifactCategory); 6] = [
        (format!("{home}/.config/{name}"), ArtifactCategory::Config),
        (format!("{home}/.cache/{name}"), ArtifactCategory::Cache),
        (format!("{home}/.local/share/{name}"), ArtifactCategory::Data),
        (format!("/var/lib/{name}"), ArtifactCategory::Data),
        (format!("/var/cache/{name}"), ArtifactCategory::Cache),
        (format!("/etc/{name}"), ArtifactCategory::Config),
    ];
    candidates
        .into_iter()
        .filter(|(p, _)| Path::new(p).exists())
        .map(|(p, c)| (PathBuf::from(p), c))
        .collect()
}

/// Tally artifacts by usage kind.
pub fn count_usage(artifacts: &[Artifact]) -> ResidueCounts {    let mut c = ResidueCounts::default();
    for a in artifacts {
        match a.usage_kind {
            UsageKind::Exclusive => c.exclusive += 1,
            UsageKind::Shared => c.shared += 1,
            UsageKind::System => c.system += 1,
        }
    }
    c
}

/// D10: best-effort runtime-warning detection. Guards on tool presence so it
/// is harmless on hosts without `pgrep`/`systemctl`.
pub fn detect_runtime_warnings(name: &str) -> Vec<RuntimeWarning> {
    let mut warnings = Vec::new();
    if Path::new("/usr/bin/pgrep").exists() {
        if let Ok(out) = run_capture("pgrep", &["-x", name]) {
            let n = out.lines().filter(|l| !l.trim().is_empty()).count();
            if n > 0 {
                warnings.push(RuntimeWarning {
                    kind: RuntimeWarningKind::ProcessRunning,
                    detail: format!("{n} running process(es) match '{name}'"),
                });
            }
        }
    }
    if Path::new("/usr/bin/systemctl").exists() {
        if let Ok(out) = run_capture("systemctl", &["is-active", name]) {
            let s = out.trim();
            if s == "active" {
                warnings.push(RuntimeWarning {
                    kind: RuntimeWarningKind::ServiceActive,
                    detail: format!("systemd unit '{name}' is active"),
                });
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ConfidenceLevel;

    #[test]
    fn classify_paths() {
        assert_eq!(classify("/usr/bin/firefox"), ArtifactCategory::Binary);
        assert_eq!(classify("/usr/lib/x86_64-linux-gnu/libfoo.so"), ArtifactCategory::Binary);
        assert_eq!(classify("/etc/firefox/firefox.conf"), ArtifactCategory::Config);
        assert_eq!(classify("/usr/share/applications/firefox.desktop"), ArtifactCategory::DesktopEntry);
        assert_eq!(classify("/home/u/.config/firefox/prefs"), ArtifactCategory::Config);
        assert_eq!(classify("/home/u/.cache/firefox/cache2"), ArtifactCategory::Cache);
        assert_eq!(classify("/var/lib/firefox/state"), ArtifactCategory::Data);
        assert_eq!(classify("/run/firefox.pid"), ArtifactCategory::State);
        assert_eq!(classify("/etc/systemd/system/foo.service"), ArtifactCategory::Service);
        assert_eq!(
            classify("/home/u/.config/mimeapps.list"),
            ArtifactCategory::Association
        );
    }

    #[test]
    fn usage_kind_purity() {
        assert_eq!(usage_kind(0), UsageKind::System);
        assert_eq!(usage_kind(1), UsageKind::Exclusive);
        assert_eq!(usage_kind(2), UsageKind::Shared);
        assert_eq!(usage_kind(7), UsageKind::Shared);
    }

    #[test]
    fn count_usage_tallies() {
        let mk = |k: UsageKind| Artifact {
            artifact_id: "x".into(),
            category: ArtifactCategory::Binary,
            target: "t".into(),
            owner_set: vec!["a".into()],
            usage_kind: k,
            size_bytes: None,
            deletable: true,
            confidence: ConfidenceLevel::High,
            discovered_by: crate::domain::DiscoverySource::Manifest,
            scope: None,
        };
        let arts = vec![
            mk(UsageKind::Exclusive),
            mk(UsageKind::Exclusive),
            mk(UsageKind::Shared),
            mk(UsageKind::System),
        ];
        let c = count_usage(&arts);
        assert_eq!((c.exclusive, c.shared, c.system), (2, 1, 1));
    }
}
