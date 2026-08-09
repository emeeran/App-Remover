//! Destructive operation execution. Abstracted behind a trait so the job
//! runner is unit-testable without root (a fake executor); the SystemExecutor
//! does the real `apt-get` via `pkexec` (polkit, D5) and filesystem work.
//!
//! ⚠ The live `pkexec apt-get remove` path can only be confirmed on a real
//! desktop with polkit + root — it is wired but not exercised in the sandbox.

use std::path::Path;
use std::process::Command;

use crate::domain::PackageBackend;
use crate::error::{AppError, AppResult};

pub trait Executor: Send + Sync {
    fn uninstall_package(&self, backend: PackageBackend, package: &str, purge: bool) -> AppResult<()>;
    fn reinstall_package(&self, backend: PackageBackend, package: &str) -> AppResult<()>;
    fn delete_path(&self, path: &str) -> AppResult<()>;
    fn restore_file(&self, blob_path: &str, original_path: &str) -> AppResult<()>;
}

/// Real executor. System-package ops run via `pkexec` (arg array → shell:false,
/// NFR-7); filesystem ops run directly (system paths need the caller's polkit
/// context, which the pkexec-spawned process has when invoked appropriately).
pub struct SystemExecutor;

impl Executor for SystemExecutor {
    fn uninstall_package(&self, backend: PackageBackend, package: &str, purge: bool) -> AppResult<()> {
        match backend {
            PackageBackend::Deb => {
                let action = if purge { "purge" } else { "remove" };
                run_privileged(&["apt-get", "-y", action, package])
            }
            PackageBackend::Snap => run_privileged(&["snap", "remove", package]),
            PackageBackend::Flatpak => run_privileged(&["flatpak", "uninstall", "-y", package]),
            // User-scope backends — no root needed; run directly.
            PackageBackend::Pip => run_plain(&["pip3", "uninstall", "-y", package]),
            PackageBackend::Npm => run_plain(&["npm", "uninstall", "-g", package]),
            PackageBackend::Systemd => {
                run_plain(&["systemctl", "--user", "stop", package])?;
                run_plain(&["systemctl", "--user", "disable", package])
            }
            // AppImage: the package_name is the file path — delete it directly.
            PackageBackend::Appimage => self.delete_path(package),
            PackageBackend::Manual => Err(AppError::Internal(format!(
                "{backend:?} is removed via delete-file (the artifact), not an uninstall command"
            ))),
        }
    }

    fn reinstall_package(&self, backend: PackageBackend, package: &str) -> AppResult<()> {
        match backend {
            PackageBackend::Deb => run_privileged(&["apt-get", "-y", "install", package]),
            PackageBackend::Snap => run_privileged(&["snap", "install", package]),
            PackageBackend::Flatpak => run_privileged(&["flatpak", "install", "-y", package]),
            PackageBackend::Pip => run_plain(&["pip3", "install", package]),
            PackageBackend::Npm => run_plain(&["npm", "install", "-g", package]),
            // Best-effort: a deleted unit/file can't be "reinstalled" — treat as no-op.
            PackageBackend::Systemd | PackageBackend::Appimage | PackageBackend::Manual => Ok(()),
        }
    }

    fn delete_path(&self, path: &str) -> AppResult<()> {
        let p = Path::new(path);
        if p.is_dir() {
            std::fs::remove_dir_all(p)?;
        } else if p.exists() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }

    fn restore_file(&self, blob_path: &str, original_path: &str) -> AppResult<()> {
        let b = Path::new(blob_path);
        let o = Path::new(original_path);
        if let Some(parent) = o.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if b.is_dir() {
            copy_dir(b, o)?;
        } else {
            std::fs::copy(b, o)?;
        }
        Ok(())
    }
}

/// Run `pkexec <args...>` (the binary is args[0]). pkexec raises a polkit
/// prompt on a real desktop and execs the binary as root on success (D5).
fn run_privileged(args: &[&str]) -> AppResult<()> {
    if args.is_empty() {
        return Err(AppError::Validation("empty privileged command".into()));
    }
    let status = Command::new("pkexec")
        .args(args)
        .status()
        .map_err(|e| AppError::Internal(format!("spawn pkexec: {e}")))?;
    if !status.success() {
        return Err(AppError::Internal(format!(
            "pkexec {} failed: {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

/// Run a command directly (no privilege escalation) for user-scope backends.
fn run_plain(args: &[&str]) -> AppResult<()> {
    if args.is_empty() {
        return Err(AppError::Validation("empty command".into()));
    }
    let status = Command::new(args[0])
        .args(&args[1..])
        .status()
        .map_err(|e| AppError::Internal(format!("spawn {}: {e}", args[0])))?;
    if !status.success() {
        return Err(AppError::Internal(format!(
            "{} failed: {status}",
            args.join(" ")
        )));
    }
    Ok(())
}

/// Recursive directory copy (for snapshot backup/restore of leftover dirs).
pub fn copy_dir(src: &Path, dst: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
