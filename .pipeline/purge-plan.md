# Phase 1 — Redundant File Purge Plan (dry-run)

**Result: 0 files to move.** The repo is a fresh, deliberate build — no dead
weight surfaced.

## Method
- `.pipeline/inventory.csv` — per-file inbound-reference grep (imports/requires).
- Exact-duplicate scan: md5 of every tracked text file (`.rs/.ts/.js/.svelte/.md/.json/.toml/.sql/.txt/.yml`) → **0 collisions**.
- Artifact scan: `git ls-files | grep` for `__pycache__ /dist/ node_modules /target/ *.pyc *.bak *~ _old copy backup /v[12]/` → **0 hits** (build outputs are gitignored, not committed).
- Zero-inbound source files: only `babel.config.js`, `tests/*`, `svelte.config.js`, `vite.config.js` — all **build/test entry points**, load-bearing.

## Judgment calls (flagged, NOT purged)

| Path | Reason it looks redundant | Verdict | Why retained |
|---|---|---|---|
| `backend/` (8 files) | Superseded by `desktop/` as the shipped product; not imported by the Tauri app | **Retain** | Intentional spec-faithful Node reference; the SDD `Makefile` auto-detects it for `make lint`/`make test`; the spec was validated against it. Human explicitly chose to keep it as the reference. |
| `media/app-remover-logo.png` (2.1 MB) | Source is a 2D-vs-3D comparison sheet; the used logos are the derived crops in `desktop/static/` | **Retain** | Original artwork needed to regenerate `desktop/static/logo.png`, `logo-full.png`, and the Tauri icons. |
| `.vscode/` | Editor config | **Retain** | Intentional shared project settings (1 file). |

## Action
No `git mv` to `trash2review`. Awaiting `--apply` confirmation, but there is
nothing to apply. Any future redundant files (e.g. a stale `backend/dist/`
build, a copied experiment) should be caught here on re-run.
