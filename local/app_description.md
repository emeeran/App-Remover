# App Remover

**Tagline:** Safely remove Linux applications — and the files they leave behind — across every package source.

---

## Short Description

App Remover is a Linux desktop application (Tauri: a Rust core + Svelte UI, shipped as a `.deb`) that removes applications *and* their leftover "residue" across apt/dpkg, snap, flatpak, pip, npm, systemd, and AppImage. It reviews every removal with you first, snapshots what it can restore, runs the removal through polkit, auto-rolls-back on failure, and keeps a tamper-evident audit trail with undo.

---

## Full Description

Removing a Linux app cleanly is hard: a single app can be spread across apt, snap, and flatpak, and its config, cache, and data linger long after the package is gone. Doing it by hand means guessing which files are safe to delete and risking breakage.

App Remover builds a complete picture first. It inventories apps from every package source, scans each one's residue (binaries, config, cache, data, services, desktop entries), and computes who owns each file — so only files exclusive to the target are ever touched. It then composes a dry-run plan where every operation carries a safety verdict (safe / risky / blocked), which you review and approve before anything happens.

Safety is the default, not an option. Every removal captures a snapshot first; if any step fails the job rolls back automatically, and a verify-before-restore check guards undo. Removals run through `pkexec`/polkit per operation with argument arrays (no shell), protected system packages sit on a non-overridable blocklist, and every finished job appends to a hash-chained audit log you can undo from the History view.

The app is a self-contained `.deb` (SQLite is statically linked) depending only on the system WebKit/GTK libraries. It is a functional MVP — see `AUDIT.md` for the production-readiness picture.

---

## Key Features

- **Eight package sources** — apt/dpkg, snap, flatpak, pip, npm, systemd user units, AppImage, and manual installs.
- **Residue scanning** — classifies leftover files into nine categories with owner-set/usage analysis (exclusive / shared / system).
- **Reviewed dry-run plans** — each operation is marked safe, risky (accept-to-proceed), or blocked before approval.
- **Snapshots + auto-rollback** — pre-removal backup; any failure restores automatically; verify-before-restore on undo.
- **Hash-chained audit + undo** — append-only, tamper-evident record of every removal, with one-click restore.
- **Least-privilege removal** — per-operation `pkexec`/polkit with argument arrays; no shell, no persistent root.
- **Protected-app blocklist** — a non-overridable core set (glibc, systemd, gnome-shell, …) can never be removed.
- **Configurable** — extend the blocklist and set a snapshot cost cap via `~/.config/app-remover/config.json`.
- **Light/dark UI** — inventory, workflow stepper (Scan → Plan → Approve → Remove), About and History modals.

---

## Common Use Cases

- **Clean uninstall** — remove an app and its config/cache/data residue in one reviewed pass.
- **Cross-source cleanup** — remove the same app whether it came from apt, snap, or flatpak.
- **Safe experimentation** — install a throwaway package, remove it, and undo if needed.
- **Audit and reverse** — review what was removed and when, and roll a removal back.

---

## Requirements & Installation

- **Linux (Debian/Ubuntu)** — `.deb` package; root (via polkit) for system-package removal.
- **Runtime libs:** `libwebkit2gtk-4.1-0`, `libgtk-3-0` (installed automatically).

```bash
# install the built package
sudo dpkg -i "desktop/src-tauri/target/release/bundle/deb/App Remover_0.1.0_amd64.deb"
sudo apt-get install -f   # only if dependencies are reported missing

# launch
app-remover
```

**Build from source:** `cd desktop && npm install && npm run tauri build`.

---

## GitHub

Source code, issues, and releases: **https://github.com/emeeran/App-Remover**

---

## License

MIT — use freely for any purpose.
