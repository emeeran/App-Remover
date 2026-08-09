# Production Readiness Audit — 2026-08-09

## Summary

**Verdict: not production-ready (solid MVP / prototype).** The app-remover is
well-structured, the non-destructive domain logic (inventory, scan, plan,
approve, snapshot hashing, audit chain, job state machine) is unit-tested
(28 Rust tests + 9 backend tests, all green), and it builds a clean `.deb`.
But it is **not safe to ship to real users yet**: the shipped desktop product
isn't exercised in CI; the actual destructive path (`pkexec apt-get remove`,
real snapshot/rollback) is untested; multi-step DB writes aren't transactional;
there's no server-side logging; and several operational gaps (orphaned jobs on
close, no snapshot GC, audit can't distinguish a failed rollback from success)
would bite in real use. Most of these are the known consequences of building
the destructive core without root/polkit/CI-for-desktop in the dev environment.

## Blockers (must fix before prod)

- [ ] **`.github/workflows/ci.yml` — CI tests only the Node reference, never the shipped product.** `make setup lint test` runs backend Jest only. `cargo test` (28 tests), `svelte-check`, and `tauri build` are never run in CI. The code that ships in the `.deb` has zero automated gating.
- [ ] **`desktop/src-tauri/src/executor.rs:31-47` + `runner.rs:197-202` — no validation at the execution boundary.** Package identifiers and artifact paths flow from the DB straight into `pkexec apt-get remove <pkg>` / `snap remove` / `flatpak uninstall` / `delete_path()` with no re-validation. apt validates names at *inventory* time only (regex blocks a leading `-`), but the other backends don't validate at all, and `DeleteFile`/`AppImage` pass a DB-stored path to `delete_path` with no canonicalization/bounds check. NFR-7 requires per-backend allowlists *at the ACL boundary*. Fix: validate the identifier/path in the executor immediately before spawn, and bound `delete_path` targets to expected roots.

## High priority

- [ ] **No DB transactions for multi-step writes** — `runner.rs` (`create_job` ~L48, `begin_job` ~L100) and `snapshot.rs` (`persist_snapshot` ~L200) issue several `INSERT`/`UPDATE`/blob-copy steps without `BEGIN…COMMIT`. A mid-sequence failure leaves orphaned blobs, a job with no `snapshot_id`, or a partial snapshot that then fails `verify()` as "corrupt." Wrap each in a transaction.
- [ ] **Destructive path untested** — `executor.rs` (`SystemExecutor`), real `pkexec` removal, and real filesystem snapshot/rollback are exercised only through `FakeExecutor`. The actual removal + restore-on-failure has never run. Needs at least one guarded, opt-in integration test on a throwaway VM.
- [ ] **No structured logging (NFR-13 unimplemented)** — no logging crate in the Rust core; errors are serialized to the UI and otherwise lost. A failed/half-completed removal leaves no server-side trace. Add `tracing`/`log` at info/warn/error around jobs, snapshots, and command spawn.
- [ ] **Audit cannot distinguish failed rollback / stuck failure from success** — `runner.rs:164-176` writes `outcome="rolled_back"` even when rollback *failed* (job stays `Failed`); jobs that crash mid-`running` never get an audit record at all. The audit trail's integrity guarantee is weaker than it appears. Distinguish outcomes (e.g. add `rollback_failed`) and ensure a record on every terminal transition.
- [ ] **DB `Mutex<Connection>` held across long `pkexec` + no orphan recovery** — `begin_job` (`runner.rs:100`) holds the connection lock for the whole removal (can be minutes), blocking every other command; closing the window mid-job leaves the job stuck in `running` forever. Add a job watchdog/timeout on startup (reap `running` jobs older than N) and don't hold the DB lock across subprocess waits.

## Medium / Low

- [ ] **`snapshot.rs` — no snapshot GC.** Blob trees accumulate forever; disk grows unbounded. Add retention (e.g. drop snapshots for jobs older than N days, or on explicit "forget").
- [ ] **`schema.sql` — migration can't evolve.** `CREATE TABLE IF NOT EXISTS` only; no `ALTER`. A schema change (e.g. the earlier `jobs.status` hyphen fix) won't apply to an existing DB. Add a `user_version`/migration runner before relying on it for upgrades.
- [ ] **`Cargo.toml` / `desktop/package.json` — deps not pinned** (caret ranges). Reproducibility risk; pin or rely on lockfiles + `npm ci`/`cargo update --lock`.
- [ ] **`runner.rs:159` — `snapshot_id.unwrap_or_default()`** yields `""` if a job lacks a snapshot, making `rollback()` fail with `NotFound` and mask the original op failure. Guard explicitly.
- [ ] **`runner.rs undo` — not idempotent**: re-undo reinstalls again; `audit.undoable` isn't flipped after undo. Mark spent records or make reinstall idempotent-by-check.
- [ ] **`schema.sql` `event_log` table** is created but never written (spec forward-looking). Wire it or drop it.
- [ ] **`db.rs:24` — no `Mutex` poison recovery.** A panic while locked bricks the app.
- [ ] **`backends/apt.rs:182` `owning_package`** swallows multi-owner `dpkg -S` results; `resolve_desktop` can fail to resolve genuinely-owned binaries.

## Explicitly out of scope / accepted risk

- **HOME env / TOCTOU / `strip_prefix` "traversal" (security agent)** — flagged as high, but on a single-user desktop the threat model is weak: an attacker who can set `HOME` or race the filesystem or plant filenames in the user's snapshot dir already has the user's privileges. The execution-boundary validation blocker above covers the realistic version of this. Accepted as low risk; revisit if this ever runs as a system service.
- **`backend/` (Node reference) findings** — it's the spec-faithful reference artifact, not shipped. Its header length, weak test assertion, etc. are out of scope for production-readiness of the product.
- **CI merge-gating / branch protection** — repo-level GitHub settings, not code; flagged for the human to configure.

## Positive notes
- All SQL is parameterized (`params![]`) — no injection.
- Subprocesses use arg-array `Command::new().args()` (no shell) — the injection barrier exists; it's the *validation gap* above that's the issue, not the spawn mechanism.
- Append-only, hash-chained audit; SHA-256 snapshot manifest + verify-before-restore (NFR-15).
- Protected-package blocklist (D7) enforced at scan/plan creation.

(Phase 5 reconciliation below.)

## Missed by pipeline, caught by blind review

Genuine Phase 4 misses — logged here rather than quietly folded into Phase 4's
numbers. (Full verbatim review: `.pipeline/blind-review.md`.)

- [ ] **`audit.rs` — the hash chain is never verified on read.** `load()`/`recent()` return records without recomputing/validating `hash = SHA256(prev || canonical)`. Tamper-evidence is write-only; a DB edit would be undetectable. Blind: MEDIUM. Fix: add `verify_chain()` used when reading history.
- [ ] **`snapshot.rs` `copy_dir`/`hash_tree` — no symlink-loop or depth guard.** A symlink cycle (or a link pointing outside the captured tree) could make capture/restore recurse unbounded or escape the tree. Blind: HIGH. Partially covered by the execution-boundary blocker, but the loop/depth aspect was missed. Fix: `walkdir` with `follow_links(false)` + a depth cap.
- [ ] **`lib.rs:3` `#![allow(dead_code)]` masks unused code.** Added during scaffolding; it now hides every unused item (including spec enum variants never constructed). Blind: MEDIUM. Phase 2 noted the variants but not that the blanket `allow` is itself the smell. Fix: drop the crate-level allow; annotate only the deliberate spec-forward items.
- [ ] **`planner.rs` / `backends/*` — non-apt reverse-dependency impact is empty.** snap/flatpak/pip/npm/systemd adapters return `Vec::new()` for `reverse_deps`, so removing them skips the dependency-impact/verdict check entirely. Blind: HIGH. Fix: implement per-backend rdep checks, or mark non-apt removals `risky`/`manual-review` pending that data.

## Higher-confidence findings (both pipeline and blind review agree)
Execution-boundary validation gap (blind's "injection points" / "unsafe fs
access" / "no boundary checking" all reduce to the AUDIT blocker on
`executor.rs`); destructive path untested; no job timeout / orphan recovery;
no transactions. These are corroborated, not new.

## Open questions (disagreements to surface, not silently resolve)
- **TOCTOU in `delete_path`/`restore_file`; "snapshot could copy sensitive files"; "scan has no authz"; "DB unencrypted"** — blind review rates these CRITICAL/HIGH; Phase 4 rated them **accepted risk** on the assumption this is a single-user desktop GUI (an attacker who can race the FS, set `HOME`, or write the local DB already has the user's privileges). **The correct severity depends on scope:** single-user GUI → pipeline's call stands; if this could ever run as a multi-user service or a SUID helper, the blind review is right and they become blockers. **Human decision needed.**
