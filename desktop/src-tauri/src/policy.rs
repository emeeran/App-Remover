//! Normative policies & config defaults (SPEC §5, decisions D7/D11).
//!
//! The core blocklist (D7) is non-overridable; the user may *append* via
//! config. The snapshot cost cap (D11) defaults to 1 GiB and may be overridden.

use std::sync::OnceLock;

use crate::config::{self, Config};

const CORE_BLOCKLIST: &[&str] = &[
    "app-remover",
    "gnome-shell",
    "gnome-session",
    "kde-plasma",
    "xfce4-session",
    "glibc",
    "libc6",
    "libstdc++6",
    "xorg",
    "xwayland",
    "mutter",
    "kwin",
    "systemd",
    "dpkg",
    "apt",
    "snapd",
    "flatpak",
];

static CONFIG: OnceLock<Config> = OnceLock::new();

fn conf() -> &'static Config {
    CONFIG.get_or_init(config::load)
}

/// A package is protected if it's on the core blocklist OR the user's appended list.
pub fn is_protected(name: &str) -> bool {
    if CORE_BLOCKLIST.contains(&name) {
        return true;
    }
    conf().blocklist_user.iter().any(|n| n == name)
}

/// Default snapshot cost cap (1 GiB) — see [`snapshot_cost_cap`] for overrides.
pub const SNAPSHOT_COST_CAP_BYTES: u64 = 1024 * 1024 * 1024;

/// Effective snapshot cost cap (config override or the default).
pub fn snapshot_cost_cap() -> u64 {
    conf().snapshot_cost_cap_bytes.unwrap_or(SNAPSHOT_COST_CAP_BYTES)
}
