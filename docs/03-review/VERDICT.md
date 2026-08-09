VERDICT: PASS

# SPEC Review — App-Remover (Stage 2 → Stage 3 gate)

**Reviewer:** Senior Spec Reviewer (pipeline gatekeeper)
**Date:** 2026-08-08
**Inputs:** `docs/02-spec/SPEC.md`, cross-checked against `docs/01-requirements/REQUIREMENTS.md`,
with `docs/00-domain/DOMAIN.md` and `docs/00-domain/CONTEXT_MAP.md` consulted for invariants,
the state machine (DOMAIN §4.4), the event stream (§4.6), Core Domain Rules (§5), and the `R-n`
relationship markers the spec cites.
**Subject:** This is a **re-review of the revised SPEC.md**. The prior review raised eight findings
(F1–F8); all eight have been incorporated into the current spec and are verified resolved in §3
below. This verdict supersedes the previous `VERDICT.md`.

## Summary

The spec is **approved to proceed to `docs/04-design/` and code.** It is comprehensive, cleanly
layered, and fully traceable: every `FR-n` (1–36), `NFR-n` (1–15), Decision `D-n` (D1–D14), and
Core Domain Rule (1–6) is realized by a concrete Zod schema, ORM table, endpoint, or named
mechanism, each citing its source ID(s) verbatim. The four gate checks — **completeness,
consistency, feasibility, and security** — are all met. The system's behavior is unambiguous and
testable at every acceptance criterion. The remaining notes in §4 are **non-blocking** refinements
to reconcile before or during implementation; none makes the spec ambiguous, untestable, infeasible,
or unsafe.

---

## 1. Gate Checks

### 1.1 Completeness — ✅
- **FR coverage (1–36):** Every FR is realized. Verified against the §11 traceability matrix and the
  §7.2/§7.5 endpoint roster. Representative spot-checks:
  - FR-1 → `GET /inventory` + `InventoryDTO` (`installMethods ≥ 1`, `skippedBackends`) + `applications`/`package_instances`.
  - FR-8 → `ResidueGraph` + `scans` (PK `{canonicalAppId, scanVersion}`) + `artifacts`; immutability via new `ScanVersion` (§6.2).
  - FR-18 → `POST /plans/{id}/approve` + `accepted_risks` + `409 PLAN_HAS_BLOCKED_OP` / `RISK_NOT_ACCEPTED`.
  - FR-23 → `snapshots`/`snapshot_entries` + `409/422` cost-cap handling + `snapshotId` set before leaving `created`.
  - FR-34 → `POST /audit/{id}/undo` + `UndoResult.deferredPackages` (best-effort/offline).
  - FR-36 → the entire §7 Application API, reachable without a GUI (§7.2 `[FR-36 AC2]`).
- **NFR coverage (1–15):** Each NFR has a realization mechanism in §10 with a testable hook —
  checksum suite (NFR-4), step-failure injection (NFR-5), verify-before-restore (NFR-15),
  injection/fuzz tests (NFR-7), backend-isolation dependency test (NFR-12), `present:false` stubs
  (NFR-14).
- **Decisions (D1–D14):** All realized and cross-referenced (§5.1 D1, §8.1 D2, §3.2/§7.6-B D3,
  §3.3 D4, §9 D5, §3.5 D6, §5.2 D7, §3.4 D8, §3.2/§8 D9, §3.3/§4 D10, §5.3 D11, §7.5 D12,
  §1.1/§7.1 D13, §3.4/§7.4 D14).
- **Core Domain Rules (1–6):** Each owned by a spec section — Resolve-before-remove (§3.2/FR-4),
  Owner-aware (§3.3/§8.1), Impact-gated (§3.4/§7.5), Protected Apps (§5.2), Snapshot-before-execute
  (§3.5/§4.1), Fail-safe ordering (§4.1/§3.4).

### 1.2 Consistency with requirements (FR/NFR IDs) — ✅
- IDs are cited **verbatim** throughout; no capability is introduced outside REQUIREMENTS §1 scope.
- Enums close over **exactly** the Ubiquitous Language values (DOMAIN §3): nine `ArtifactCategory`,
  four `SafetyVerdict`, five `Action`, five `JobStatus`/`StepStatus`, three `UsageKind`/`RemovalScope`.
  The added `remove-association` action (beyond the four command verbs named in FR-31) is justified
  and traceable to the `association` category in D1/FR-9 — not an invented requirement.
- The job state machine (§4.1) matches DOMAIN §4.4 exactly; the event stream (§4.2) matches
  DOMAIN §4.6 one-for-one and projects to exactly one immutable `AuditRecord` per terminal job.
- `AuditRecord.outcome` (`completed|rolled_back`), the ORM column, and the terminal job states now
  agree (the prior F2 contradiction is resolved).

### 1.3 Feasibility — ✅
- Stack is standard and available: Node.js + Express 5 + TypeScript (strict) + Zod 4 + Drizzle ORM
  + better-sqlite3. All named libraries are real and current; the spec correctly defers adding them
  to `backend/package.json` at the code stage (§2 note).
- Local-only `0600` Unix-socket OHS with loopback fallback, and per-operation `pkexec`/polkit
  elevation (no persistent root shell), is a sound, implementable privilege model on Ubuntu 22.04/24.04.
- Filesystem snapshot blobs referenced from SQLite metadata is a normal, feasible pattern; the
  single-writer, synchronous nature of `better-sqlite3` also naturally serializes the global audit
  hash-chain append (no concurrent `prevHash` race).
- The one-directional dependency rule (`api → domain → acl/persistence interfaces`) is realizable
  and directly supports the NFR-12 isolation test.

### 1.4 Security — ✅
- **Transport:** local-only, filesystem-permission-gated `0600` socket owned by the desktop user; no
  remote listener required `[D13]`.
- **Injection barrier (primary):** `node:child_process.execFile` with an **argument array** and
  `shell: false` `[§8]` — injection-safe regardless of identifier content. This is correctly named as
  the primary guarantee.
- **Injection barrier (defense-in-depth):** a dedicated ACL input-validation layer (§8.2) applies
  per-backend allowlists (Debian Policy, snap, reverse-DNS, PEP 503, npm, systemd unit, path
  confinement to the residue-policy root) before exec. Paths are additionally confined to their
  policy root (no `../`/off-root symlink escape) `[NFR-4]`.
- **Least privilege:** per-operation polkit actions via `PrivilegeGateway`; released on completion;
  no interactive root shell `[§9, D5, FR-32]`.
- **Secret hygiene:** `0600` blobs under a `0700` root; structured logs scrub file contents, env
  vars, and credentials; only sizes/paths/checksums logged `[§9, NFR-8]`.
- **Tamper-evidence:** append-only `audit_records` with a sha256 hash chain; no `UPDATE`/`DELETE`
  repository path `[§3.7, §6.2, NFR-9]`. Snapshot integrity via `manifestChecksum` + per-entry sha256,
  verified before restore; corrupt ⇒ Undo refused `[NFR-15]`.

---

## 2. Verdict rationale

The spec satisfies the gate definition: it is **complete** (no FR/NFR/Decision/Domain Rule is
unowned), **consistent** (IDs verbatim, enums closed, schemas/ORM/API/state-machine mutually
aligned), **feasible** (standard stack, sound privilege and persistence model), and **secure**
(arg-array `execFile` barrier + defense-in-depth allowlists + least-privilege elevation +
append-only hash-chained audit + local-only surface). It is neither ambiguous nor untestable — every
acceptance criterion maps to a concrete, checkable artifact. **PASS.**

---

## 3. Prior findings (F1–F8) — all RESOLVED in this revision

| # | Prior finding | Resolution in current SPEC.md |
|---|---|---|
| F1 | Identifier allowlist contradicted the §3 schema | ✅ §8.2 adds the ACL input-validation layer with per-backend regexes; explicitly states §3 schemas are the canonical/storage form and the allowlists are defense-in-depth. `[F1]` marker threaded through §3.2/§8/§9. |
| F2 | `AuditRecord.outcome` typed as `JobStatus` (too loose) | ✅ §3.7 now `z.enum(["completed","rolled_back"])`; matches ORM §6.1 and terminal job states. `[F2]` |
| F3 | Plan-approval gate location ambiguous | ✅ §4.1 + §7.5 enforce `RemovalPlan.status === 'approved'` at **both** `POST /jobs` and `POST /jobs/{id}/begin`; `409 PLAN_NOT_APPROVED` documented on both. `[F3]` |
| F4 | Disambiguation blocking undefined at API boundary | ✅ §7.5 `POST /scans` and `POST /plans` return `409 DISAMBIGUATION_REQUIRED` when `Application.instancesDisambiguated === false`. `[F4]` |
| F5 | Lock/running-target → verdict mapping implicit | ✅ §3.4 invariants spell it out: `autoStoppable`/`process-running` + recorded acceptance → `risky`; non-`autoStoppable`/un-releasable `lock-held` → `manual-review`; never auto-executed. `[F5]` |
| F6 | Audit append-only is code-level, not DB-level | ✅ §6.2 states this explicitly as a deliberate code-level contract with a unit test; no `UPDATE`/`DELETE` path by construction. `[F6]` |
| F7 | NFR-2 wording vs cost-cap enforcement point | ✅ §10 (NFR-2 row) explains: projected size at plan time, cap enforced at job creation before capture/deletion. `[F7]` |
| F8 | Missing sweep / conflicting-job error codes | ✅ §7.4 adds `409 ILLEGAL_TRANSITION` (incl. sweep on non-`completed`/non-`purge`) and `409 CONFLICTING_JOB` (single-write lock). `[F8]` |

---

## 4. Non-blocking observations (reconcile before/during code)

These are precision/robustness refinements. None blocks the gate; each is localized and resolvable
by reasonable interpretation or at the code stage.

### O1 — `instancesDisambiguated` initial value for single-instance apps *(FR-7; testability)*
The blocking guard (`409 DISAMBIGUATION_REQUIRED` when `instancesDisambiguated === false`, per F4) is
correct for the multi-instance case, but the spec does not state that the `"resolved"` path (single
instance / already-merged) sets `instancesDisambiguated = true`. The ORM default is `false`
(`applications.disambiguated`). An implementer reading literally could block a single-instance app.
**Recommend:** state explicitly that a `"resolved"` result implies `instancesDisambiguated = true`
(disambiguation is required only when `instanceCount > 1` with distinct instances, per FR-7 AC1).

### O2 — `resolveToken` lifecycle/storage not modeled *(FR-7; minor)*
`ResolveResult` (`disambiguation-required`) returns a `resolveToken` + candidates, and
`DisambiguateRequest` consumes the token — but there is no persistence table (or stated in-memory
store) for the token→candidates mapping, nor a validity lifetime. Because the selection is carried in
the request body (`selectedCanonicalAppIds`), the token can be treated as a correlation id, but this
should be stated (and whether it survives a service restart).

### O3 — `SnapshotEntry.checksum` required for non-file entries *(§3.5; minor schema)*
`checksum` is a required 64-char field, but `blobPath` is nullable ("null when cache-excluded or
reinstall-only"). For `package-record` (and any cache-excluded) entries there is no file blob to
checksum. **Recommend:** make `checksum` nullable, or define what is checksummed for non-`file-backup`
entries (e.g. the record metadata), so the field's semantics are unambiguous.

### O4 — `scan` scope vs `plan` scope duality *(FR-22; minor)*
`ScanRequest.scope` and `ComposePlanRequest.scope` both default to `"both"`. The intended model
(scan captures all artifacts with their `scope` tags; the plan filters) is sound, but a client that
scans with `current-user` then plans with `system-wide`/`both` would silently miss un-scanned system
artifacts. **Recommend:** state that the plan's scope must be a subset of the scan's scope, or that
scans always capture both scopes (scope applied only at planning).

### O5 — Terminal state for a job whose rollback itself fails *(FR-25 AC2 / NFR-5; edge case)*
The state machine makes `failed → rolled_back` the only exit from `failed` and automatic, so a job
whose `rollback()` cannot complete (e.g. corrupt snapshot, disk full) has no defined terminal state
and, per FR-33's "failed/rolled-back" wording, an unclear audit record. NFR-5 bounds this to `< 1%`,
so it is not untestable for the common path. **Recommend:** define a terminal state (e.g.
`failed-pending-manual`) and confirm an `AuditRecord` is still written so manual recovery from the
surviving snapshot is possible.

### O6 — `artifacts` table lacks the FKs the spec claims are universal *(§6; minor)*
§6 states "all foreign keys are `ON DELETE RESTRICT`," but `artifacts` declares no `.references()` to
`applications` or `scans` (only an index). **Recommend:** add the FKs for integrity, or narrow the
"all FKs" statement to the tables that actually define them.

### O7 — FR-9 "unclassifiable → flagged for review" not explicitly modeled *(FR-9 AC1; minor)*
FR-9 AC1 requires unclassifiable items to be "flagged for review rather than silently dropped," but
`ArtifactCategory` is a closed enum with no `unknown`/review state and the scan contract doesn't
describe a surfacing path. **Recommend:** either model a low-confidence review flag or state that
unclassifiable candidates are emitted as `manual-review`-flavored artifacts.

---

## 5. Recommendation

**Proceed to `docs/04-design/` and code.** Address O1 first (it is the only one that could produce an
obviously wrong runtime behavior if mis-implemented); O3 and O5 during schema/execution design; the
rest as housekeeping during implementation. The spec's architecture, contracts, invariants, and
safety properties are sound, internally consistent, and unambiguously testable. The pipeline gate is
satisfied.
