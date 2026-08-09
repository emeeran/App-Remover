//! Subprocess execution. Always uses an explicit argument array (never a
//! shell) — this is the primary injection barrier (NFR-7, SPEC §9).

use std::process::Command;

use crate::error::{AppError, AppResult};

/// Run `bin` with `args`, returning captured stdout. A non-zero exit or spawn
/// failure becomes `INTERNAL_ERROR`.
pub fn run_capture(bin: &str, args: &[&str]) -> AppResult<String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| AppError::Internal(format!("spawn {bin}: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(AppError::Internal(format!(
            "{bin} exited {}: {}",
            out.status,
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
