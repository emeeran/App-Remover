//! Minimal `.desktop` entry parser (INI-like, [Desktop Entry] section).
//! Used by `resolve_app` to map a launcher to its owning package.

use std::collections::HashMap;

use crate::error::{AppError, AppResult};

#[derive(Debug, Default, Clone)]
pub struct DesktopEntry {
    pub name: String,
    pub icon: Option<String>,
    pub exec: Option<String>,
    /// Reverse-DNS app id derived from the filename (e.g. org.mozilla.firefox).
    pub app_id: Option<String>,
}

pub fn parse(path: &str) -> AppResult<DesktopEntry> {
    let content =
        std::fs::read_to_string(path).map_err(|e| AppError::NotFound(format!("{path}: {e}")))?;
    let mut entry = parse_str(&content);
    if entry.app_id.is_none() {
        entry.app_id = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
    }
    Ok(entry)
}

pub fn parse_str(content: &str) -> DesktopEntry {
    let mut in_entry = false;
    let mut map: HashMap<String, String> = HashMap::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            in_entry = rest.starts_with("Desktop Entry]");
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    DesktopEntry {
        name: map.get("Name").cloned().unwrap_or_default(),
        icon: map.get("Icon").cloned(),
        exec: map.get("Exec").cloned(),
        app_id: None,
    }
}

/// First token of an `Exec=` value with %f/%u placeholders stripped — the
/// binary name or absolute path the launcher invokes.
pub fn exec_binary(exec: &str) -> Option<String> {
    let first = exec.split_whitespace().next()?;
    // respect env-assignment prefixes (ENV=value bin ...)
    if first.contains('=') {
        return exec
            .split_whitespace()
            .find(|t| !t.contains('='))
            .map(|t| t.to_string());
    }
    Some(first.trim_start_matches("file://").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desktop_entry_section_only() {
        let s = "[Desktop Entry]\nName=Firefox\nIcon=firefox\nExec=/usr/lib/firefox/firefox %u\n\n[Other Section]\nName=ignore\n";
        let e = parse_str(s);
        assert_eq!(e.name, "Firefox");
        assert_eq!(e.icon.as_deref(), Some("firefox"));
        assert_eq!(e.exec.as_deref(), Some("/usr/lib/firefox/firefox %u"));
    }

    #[test]
    fn exec_binary_handles_placeholders_and_env() {
        assert_eq!(exec_binary("/usr/bin/foo %u").as_deref(), Some("/usr/bin/foo"));
        assert_eq!(exec_binary("foo").as_deref(), Some("foo"));
        assert_eq!(exec_binary("GST_DEBUG=3 /opt/app/bin/app").as_deref(), Some("/opt/app/bin/app"));
        assert!(exec_binary("").is_none());
    }
}
