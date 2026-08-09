## App Remover
A Linux desktop app that safely removes applications and their leftover files across apt, snap, flatpak, pip, npm, systemd, and AppImage.
### What it does
It scans an app's residue, composes a reviewed removal plan, snapshots restorable files, runs the removal via polkit, auto-rolls-back on failure, and keeps a hash-chained audit trail you can undo.
### Key features
- Inventory and residue scanning across eight package sources
- Dry-run plans with safe, risky, and blocked verdicts before anything is deleted
- Pre-removal snapshots with verify-before-restore and automatic rollback
- Append-only hash-chained audit log with one-click undo
- Light and dark themed Svelte UI with a guided scan-to-remove workflow
### Run
```
sudo dpkg -i "desktop/src-tauri/target/release/bundle/deb/App Remover_0.1.0_amd64.deb"
app-remover
```
### GitHub
https://github.com/emeeran/App-Remover
