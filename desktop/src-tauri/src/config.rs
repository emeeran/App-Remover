//! Optional user config from `${XDG_CONFIG_HOME:-~/.config}/app-remover/config.json`.
//! Best-effort: a missing or invalid file yields defaults.

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Extra protected-app names appended to the non-overridable core blocklist (D7).
    #[serde(default)]
    pub blocklist_user: Vec<String>,
    /// Override the snapshot cost cap (D11); None → default 1 GiB.
    #[serde(default)]
    pub snapshot_cost_cap_bytes: Option<u64>,
}

fn path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("app-remover")
        .join("config.json")
}

pub fn load() -> Config {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}
