# Phase 2 — Debloat & Refactor Plan (dry-run)

Behavior-preserving only. On `--apply`, executed unit-by-unit with tests
re-run after each. Nothing here changes observable behavior.

## Apply list (will execute on --apply)

| # | Unit | Change | Why | Risk |
|---|---|---|---|---|
| A | `serde_utils` (new) + scanner/planner/snapshot/runner | Extract one shared `jstr`/`eparse` (enum↔kebab (de)serialization); delete the 4 private copies (~24 lines) | 4 identical reimplementations of a 6-line pair — the clearest DRY win | none — pure move |
| B | `backends/apt.rs` | Replace apt's private `make_artifact` with the existing `residue::make_artifact`; delete the local copy (~18 lines) | Two artifact builders for the same thing; `residue::make_artifact` already does scope-inference | low — apt passes explicit `Manifest/High` vs `Heuristic/Medium`; verify the call sites map cleanly |
| C | `lib.rs` | Remove the unused scaffold `greet` command + its `generate_handler!` entry (~5 lines) | Frontend never invokes `greet`; leftover from `create-tauri-app` | none |

Estimated delta: **~−45 lines, 0 behavior change.**

## Explicitly NOT touching (flagged, retained with reason)

| Item | Agent said | Verdict | Reason |
|---|---|---|---|
| `Action::{StopService,RemoveAssociation,PruneOrphan}`, `RemovalScope::{CurrentUser,Both}`, `DiscoverySource::KnowledgeBase`, `PlanStatus::Superseded` | "dead enum variants" | **Retain** | Spec-mandated (SPEC §3); the impl doesn't construct all yet because those ops/scopes aren't wired, but removing them diverges from the spec and forces re-adding later. Decision-fork: ambiguous → keep + flag. |
| `event_log` table (schema.sql) | "dead table" | **Retain** | Spec-mandated (SPEC §6); forward-looking. |
| match arms for the above (`_ => Ok(())`) | "dead arms" | **Retain** | Required for exhaustive matching while the variants exist. |
| `Executor` trait | "single-impl abstraction" | **Retain** | Justified test-seam — `FakeExecutor` makes the destructive runner unit-testable without root. |
| `PackageManager` trait | "abstraction" | **Retain** | 7 real adapters; enables `real_adapters()` dispatch + `present()` detection. |
| `onKey` Escape handler (+page.svelte) | "vestigial" | **Retain** | Deliberate a11y/UX — keyboard close for modals; not redundant with backdrop click. |
| 6× try/catch/finally in +page.svelte | "extract a helper" | **Leave as-is** | "Boring over clever" — extracting adds indirection to readable, simple handlers; agent agreed it's borderline. Revisit only if the pattern grows past ~8. |
| `.ok()` suppressions (apt `dpkg -L`, backend `dpkg -S`) | "inconsistent error philosophy" | **Retain + document** | Intentional best-effort (missing tool / unpackaged file → empty, not fatal). Will add a one-line `// best-effort` comment on `--apply`. |
| Spec-clause comments (`// F3`, `// NFR-15`) | "comment noise" | **Retain** | These are traceability pointers to normative spec clauses (the *why*), not restatements. |
| `backend/src/app.ts` + its tests | various | **Out of scope** | Spec-faithful Node *reference* artifact, not the shipped product; refactoring generated reference code is low-value/risky. |
| `svelte.config.js` / `vite.config.js` comments | "restating" | **Leave** | Stock `create-tauri-app` scaffolding; editing is near-zero value. |
| `smoke.test.ts` (`expect(true).toBe(true)`) | "padding test" | **Needs-judgment** | Acts as a "jest harness runs" canary; harmless. Lean remove on `--apply` since `api.test.ts` already proves the harness. |

## Out-of-scope for Phase 2 (style → Phase 3)
Import ordering, `details?: unknown` typing, `resolveApp` return type — these
are style/typing consistency, handled in Phase 3, not bloat.
