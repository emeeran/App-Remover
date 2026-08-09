# App Remover

**Safely remove Linux applications — and the files they leave behind — across every package source on your system.**

App Remover is a Linux desktop app (built with [Tauri](https://tauri.app): a Rust core + Svelte UI) shipped as a `.deb`. It inventories apps from apt/dpkg, snap, flatpak, pip, npm, systemd, and AppImages; computes the "residue" each leaves behind; walks you through a reviewed, approved removal plan; snapshots what it can restore; performs the removal via polkit; auto-rolls-back on failure; and records a hash-chained audit trail you can undo later.

> **Status:** functional MVP, **not production-ready** — see [`AUDIT.md`](./AUDIT.md). The destructive path (real `apt-get remove`/snap/flatpak via polkit, plus snapshot/rollback) is wired and unit-tested with a fake executor but has not been exercised against a live system in CI.

## Features
- **Eight package sources** — apt/dpkg, snap, flatpak, pip, npm, systemd user units, AppImage, and manual installs.
- **Residue scanning** — classifies leftover files by category (binary, config, cache, data, service, desktop-entry, …) with owner-set/usage analysis.
- **Dry-run plans with verdicts** — every operation is *safe* / *risky* (accept the risk) / *blocked* before anything runs.
- **Snapshots + auto-rollback** — pre-removal backup; any step failure restores automatically; verify-before-restore on undo.
- **Hash-chained audit + undo** — append-only, tamper-evident record of every removal, with one-click restore from History.
- **Least privilege** — removals run through `pkexec`/polkit per operation using argument arrays (no shell).
- **Configurable** — extend the protected blocklist and set a snapshot cost cap via `~/.config/app-remover/config.json`.
- **Light/dark UI** — inventory, workflow stepper, About and History modals.

## Install
```bash
sudo dpkg -i "desktop/src-tauri/target/release/bundle/deb/App Remover_0.1.0_amd64.deb"
sudo apt-get install -f   # only if dependencies are reported missing
app-remover
```
Runtime deps: `libwebkit2gtk-4.1-0`, `libgtk-3-0` (pulled in automatically).

## Build from source
```bash
cd desktop
npm install
npm run tauri build        # → src-tauri/target/release/bundle/deb/App Remover_0.1.0_amd64.deb
npm run tauri dev          # live development (needs a display)
```
**Test:**
```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml   # 28 Rust unit tests
npm --prefix desktop run check                            # svelte-check (frontend)
npm --prefix backend test                                 # Node reference (9 jest tests)
```

## How it works
`Inventory → Scan residue → Compose plan → Approve → Snapshot → Remove (polkit) → Audit → (Undo)`

The Rust core (`desktop/src-tauri/src/`) is organized as flat modules: `domain` (SPEC §3 types), `db` (SQLite, 13 tables), `backends/` (one adapter per package source), `residue` (classification + owner-set), `scanner`/`planner` (dry-run plans + verdicts), `snapshot` (capture/verify/restore), `executor` (the real `pkexec` commands behind a testable trait), `runner` (job state machine + auto-rollback), `audit` (hash chain), and `commands` (the Tauri command surface). The Svelte UI (`desktop/src/`) drives it via `invoke`.

## Configuration
```json
// ~/.config/app-remover/config.json
{
  "blocklist_user": ["my-important-tool"],
  "snapshot_cost_cap_bytes": 5368709120
}
```
- `blocklist_user` — extra protected package names (appended to the non-overridable core list).
- `snapshot_cost_cap_bytes` — refuse snapshots larger than this (default 1 GiB) unless you opt in.

## Data
- DB + audit: `~/.local/share/app-remover/app-remover.db`
- Snapshots: `~/.local/share/app-remover/snapshots/<id>/`
- Config: `~/.config/app-remover/config.json`

## Limitations
See [`AUDIT.md`](./AUDIT.md) for the full production-readiness picture. Headlines: the desktop product is not yet in CI; the live removal path is unit-tested only; multi-owner residue detection is simplified; pip/npm inventories include libraries.

## Repository layout
This repo also contains the **Spec-Driven Development (SDD)** artifacts used to derive the app:
- `docs/00-domain`, `docs/01-requirements`, `docs/02-spec`, `docs/03-review` — the spec pipeline outputs.
- `backend/` — a Node.js (Express + Zod) **reference implementation** the spec was validated against; not shipped in the `.deb`.
- `prompts/`, `Makefile` — the SDD pipeline (run `make help`; stages need the `claude` CLI).
- User manual: [`docs/USER_MANUAL.md`](./docs/USER_MANUAL.md).

## License
MIT.
