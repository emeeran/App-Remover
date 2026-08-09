# Phase 3 — Style & Authorship Consistency Plan (dry-run)

The codebase was written in a short span by one hand, so it is already
consistent. This phase found little — recorded honestly below.

## Detected conventions
- **Formatter/linter:** none configured (no `rustfmt.toml`/`prettier`/`eslint`/`biome`/`ruff`). Current style IS the de-facto convention.
- **Rust docs:** consistent `//!` module headers + `///` fn docs throughout `desktop/src-tauri/src`.
- **Error philosophy:** uniform — Rust returns `AppResult<T>` everywhere; frontend funnels errors through `describeError()`. No mixed strategies.
- **Structure:** flat modules (`domain`, `db`, `backends/`, `runner`, …) — no unjustified `services/`/`repositories/`/`factories/` scaffolding.
- **Naming:** no reflexive `Manager`/`Handler`/`Util` suffixes; verb-first fns, noun-first types.
- **AI voice in code/logs:** none. UI badge glyphs (🔒 ⚠ ▶) are product tone for a desktop GUI, not log/comment noise — retained. (The lone `!!` is the stock `main.rs` "DO NOT REMOVE" scaffold comment — left as-is.)

## Apply list (will execute on --apply)

| # | File | Change | Why |
|---|---|---|---|
| 1 | `desktop/src/lib/api.ts` | `AppError.details?: unknown` → `Record<string, unknown>`; type `resolveApp` return (`unknown` → `CanonicalApplication`) | Tighten the API boundary; removes `unknown`-escape hatches |

## Recommend (separate, not auto-applied)
- **Add formatters** so consistency is enforced, not just incidental: `rustfmt.toml` (+ `cargo fmt`) for Rust and Prettier for TS/Svelte. Deliberately *not* run in this pass — a first `cargo fmt`/`prettier --write` is a large mechanical diff that deserves its own commit and review, not to be folded into a cleanup gate. Flagged for the human.

## Explicitly not changing
- Import ordering, minor TS typing beyond item 1 — below the bar for a consistency pass; the files already read alike.
- Stock scaffold files (`svelte.config.js`, `vite.config.js`, `main.rs`) — untouched.

**Estimated delta on --apply: ~4 lines in one file, no behavior change.**
