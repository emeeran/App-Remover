# Project Context: app-remover

## What this is
**App Remover** — a Linux desktop app (Tauri v2: Rust core + SvelteKit UI) that removes applications and their residue across apt/dpkg, snap, flatpak, pip, npm, systemd, and AppImage. Shipped as a `.deb`. The product lives in `desktop/`.

`backend/` is a **Node.js (Express + Zod) reference implementation** — the spec was validated against it — and is not shipped. The SDD pipeline (`Makefile`, `prompts/`, `docs/`) produced the spec the app implements.

## Tech stack
- **Product (`desktop/`):** Rust (Tauri 2, rusqlite, serde, sha2) + SvelteKit/TypeScript frontend.
- **Reference (`backend/`):** Node.js, Express, Zod, Jest.

## Build & test
```bash
cd desktop && npm run tauri build                      # produces the .deb
cargo test --manifest-path desktop/src-tauri/Cargo.toml
npm --prefix desktop run check
```
SDD pipeline: `make help` (domain → reqs → spec → review → code; stages need the `claude` CLI).

## Conventions
- Rust source: `desktop/src-tauri/src/` — flat modules (domain, db, backends/, residue, scanner, planner, snapshot, executor, runner, audit, commands). Error type `AppError` serializes to `{error:{code,message,details}}`; commands return `AppResult<T>`. Wire structs use `#[serde(rename_all = "camelCase")]` (frontend is camelCase); enums are kebab-case and the SQLite CHECK constraints must match.
- Frontend: `desktop/src/` (SvelteKit SPA, `ssr:false`); calls Rust via `@tauri-apps/api` `invoke`.
- SQLite schema is migrated on open via `CREATE TABLE IF NOT EXISTS` (no `ALTER` yet — see `AUDIT.md`).
- Read `AUDIT.md` before changing destructive paths.

## Security
Destructive ops run via `pkexec` with argument arrays (no shell). apt names are validated at scan time; execution-boundary validation for all backends is an open gap (`AUDIT.md`). Never pass unvalidated identifiers or paths to `Command::new` or `delete_path`.
