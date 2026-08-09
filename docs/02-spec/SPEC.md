# SPEC.md — App-Remover

> **Stage 2 (Specification).** Translates [REQUIREMENTS.md](../01-requirements/REQUIREMENTS.md) into an
> implementable design: data schemas, API contracts (OpenAPI 3.1 style), and ORM/data models.
> *Inputs:* [DOMAIN.md](../00-domain/DOMAIN.md), [CONTEXT_MAP.md](../00-domain/CONTEXT_MAP.md),
> [REQUIREMENTS.md](../01-requirements/REQUIREMENTS.md). `CLAUDE.md` (C1) binds the implementation stack.
> *Downstream consumer:* `docs/03-review/VERDICT.md` (PASS gate), then `docs/04-design/` and code.
> *Glossary:* the Ubiquitous Language in [DOMAIN.md §3](../00-domain/DOMAIN.md) is normative. Terms are
> reused verbatim; this spec adds none.

---

## 0. Document Control

| Field | Value |
|---|---|
| Stage | 2 — Specification |
| Status | Draft (pending `make review` PASS) |
| Implementation stack (C1) | Node.js + Express 5 + TypeScript (strict) + Zod 4 + Jest; CommonJS / `nodenext` |
| Traceability convention | Every schema, table, and endpoint cites one or more `FR-n` / `NFR-n` / `D-n` IDs from REQUIREMENTS.md. |
| OpenAPI version | 3.1 |
| ID format | All entity identifiers are UUIDv4 strings (`z.string().uuid()`). `ScanVersion` is a monotonic integer per `CanonicalAppId`. |

### 0.1 Traceability legend

| Marker | Meaning |
|---|---|
| `[FR-n]` | Realizes Functional Requirement `n`. |
| `[NFR-n]` | Satisfies Non-Functional Requirement `n`. |
| `[D-n]` | Implements Requirements Decision `n` (resolves DOMAIN.md §6 OQ). |
| `[R-n]` | Realizes CONTEXT_MAP.md relationship `n`. |

---

## 1. Scope & System Context

### 1.1 In scope (restated from REQUIREMENTS §1.1)

A **local Node service** exposing an **Application API** (an OHS/PL per [CONTEXT_MAP §6](../00-domain/CONTEXT_MAP.md))
consumed by a desktop client. The service detects install provenance, computes residue, plans a safe
removal, executes it reversibly, and undoes it — across apt/deb, snap, flatpak, AppImage, pip/npm,
manual/script installs, and user/system `systemd` services `[D13, FR-36]`. Operation is **local host
only** `[A2]`; a single interactive desktop user is assumed `[A2]`.

### 1.2 Out of scope (from REQUIREMENTS §1.2)

Non-Debian formats (RPM, pacman, Homebrew), remote/fleet management, macOS/Windows/mobile, GUI toolkit,
antivirus/malware remediation. No requirement in this spec derives from those areas.

### 1.3 Layering (contexts → modules)

The core (Residue, Safety, Execution) never imports a package-manager symbol; all backend tooling is
isolated behind the PM ACL `[NFR-12, R1, R7]`. Module layout mirrors the bounded contexts:

```
backend/src/
  app.ts                      Express bootstrap; binds the Application API server (FR-36)
  api/                        Application API (OHS/PL) — routes, request Zod validation, DTO mapping
  domain/                     Core domain (aggregates, value objects, invariants) — no I/O
    application/              Provenance & Canonicalization (FR-4..FR-7)
    residue/                  ResidueGraph (FR-8..FR-13)
    safety/                   RemovalPlan (FR-14..FR-22)
    execution/                RemovalJob, Snapshot, state machine (FR-23..FR-29)
    audit/                    AuditRecord, hash chain (FR-33..FR-35)
  acl/                        Package Manager ACL — uniform query/command surface (FR-30, FR-31)
    adapters/                 deb, snap, flatpak, appimage, pip, npm, manual, systemd
  privilege/                  Privilege gateway: polkit/pkexec, injection-safe exec (FR-32)
  persistence/                Drizzle ORM schema + repositories (§6)
  policy/                     Residue policy, blocklist, cost cap (D1, D7, D11)
```

The dependency rule is one-directional: `api → domain`; `domain → (acl interfaces, persistence
interfaces)`; `acl`/`persistence`/`privilege` are leaf infrastructure `[NFR-12]`.

---

## 2. Technology Decisions

Each decision is bound to a requirement or constraint. No decision introduces capability outside scope.

| Decision | Choice | Rationale / binding |
|---|---|---|
| Transport | HTTP/1.1 over a **Unix domain socket** (loopback TCP fallback) | Local-only OHS for the desktop client `[D13, FR-36, R5]`; filesystem permissions replace network auth. Socket path: `${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/app-remover.sock`, mode `0600`. Fallback: `127.0.0.1:${PORT:-17763}`. |
| API style | REST resources + action sub-resources | Matches the discrete operations required by `[FR-36 AC1]` (enumerate, resolve, dry-run, approve, execute, undo). |
| Validation | **Zod 4** at every API and ACL boundary | C1; `[NFR-7]` injection-safety; schemas double as the OpenAPI component schemas (§5). |
| Primary store | **SQLite** (embedded, `better-sqlite3`) | Local, single-user, serverless `[A2]`; transactional integrity for the immutable audit trail `[NFR-9]`. DB path: `${XDG_DATA_HOME:-$HOME/.local/share}/app-remover/app-remover.db`. |
| ORM / schema | **Drizzle ORM** (SQLite dialect) | TypeScript-first, strict-mode-friendly; schema-as-code migratable alongside the repo `[NFR-12]`. |
| Snapshot blob store | **Filesystem** under a configured root | D6 file-level backup; cost-cap measurable `[D11, FR-23]`. System root `/var/lib/app-remover/snapshots`; user root `${XDG_DATA_HOME:-$HOME/.local/share}/app-remover/snapshots`; mode `0700` `[NFR-8]`. |
| Hashing | SHA-256 (`node:crypto`) | Snapshot + audit integrity `[NFR-9, NFR-15]`. |
| Privilege | **polkit/pkexec**, per-session, least privilege | `[D5, FR-32, NFR-7]`; never an interactive root shell. |
| Concurrency | One in-flight `RemovalJob` per `CanonicalAppId`; serialized by a single-write lock | `[D10, FR-28]`; prevents concurrent mutation of shared targets. |
| Logging | Structured JSON to stderr/journald | `[NFR-13]`; secrets scrubbed `[NFR-8]`. |

> **No ORM or HTTP middleware is invented here.** Drizzle, better-sqlite3, and the existing Express/Zod
> dependencies are added to `backend/package.json` at the code stage; their absence now is expected (this
> is the spec stage).

---

## 3. Core Domain Schemas (Zod) — the Published Language

These Zod schemas are the canonical, runtime-validated representation of the Ubiquitous Language
(`[R5, R6]` published languages). They are reused verbatim as the OpenAPI `components.schemas` (§7.3).
All enums are closed; `exactOptionalPropertyTypes`-safe (absent vs. `null` is explicit).

### 3.1 Shared value objects

```ts
// zod v4
import { z } from "zod";

// --- Identifiers ---
export const CanonicalAppId = z.string().uuid();
export const PackageInstanceId = z.string().uuid();
export const ArtifactId = z.string().uuid();
export const OperationId = z.string().uuid();
export const StepId = z.string().uuid();
export const PlanId = z.string().uuid();
export const JobId = z.string().uuid();
export const SnapshotId = z.string().uuid();
export const AuditRecordId = z.string().uuid();
export const ResolveToken = z.string().uuid();
export const ScanVersion = z.number().int().nonnegative();        // monotonic per CanonicalAppId (FR-8 AC3)
export const PathString = z.string().min(1).max(4096);
export const IsoTimestamp = z.string().datetime();                // UTC, e.g. 2026-08-08T14:30:00.000Z

// --- Enums (closed) ---
export const PackageBackend = z.enum([                            // ACL adapter identifiers (FR-30)
  "deb", "snap", "flatpak", "appimage", "pip", "npm", "manual", "systemd",
]);
export const InstallMethod = z.enum([                             // Provenance (DOMAIN §3)
  "deb", "snap", "flatpak", "appimage", "manual", "language-package", "container",
]);
export const ConfidenceLevel = z.enum(["high", "medium", "low"]); // FR-5 AC2, FR-12
export const ArtifactCategory = z.enum([                          // FR-9 (exactly these nine)
  "binary", "config", "cache", "data", "state", "service",
  "desktop-entry", "association", "dependency",
]);
export const UsageKind = z.enum(["exclusive", "shared", "system"]); // DOMAIN §4.2 / FR-11
export const DiscoverySource = z.enum(["manifest", "knowledge-base", "heuristic"]); // FR-12 / D4
export const RemovalScope = z.enum(["system-wide", "current-user", "both"]); // FR-22 / D8
export const RemovalMode = z.enum(["remove", "purge"]);           // FR-21
export const Action = z.enum([                                    // FR-31 (exactly these five)
  "uninstall-package", "stop-service", "delete-file", "remove-association", "prune-orphan",
]);
export const SafetyVerdict = z.enum(["safe", "risky", "blocked", "manual-review"]); // FR-16
export const PlanStatus = z.enum(["draft", "approved", "superseded"]); // FR-14 AC2
export const JobStatus = z.enum(["created", "running", "completed", "failed", "rolled_back"]); // FR-26
export const StepStatus = z.enum(["pending", "running", "succeeded", "failed", "skipped"]); // FR-29
export const SnapshotEntryKind = z.enum(["file-backup", "package-record", "unit-record"]); // D6
export const ScopeTag = z.enum(["system", "user"]);               // FR-22
```

### 3.2 Provenance & Canonical Application `[FR-4, FR-5, FR-6, FR-7]`

```ts
export const PackageInstanceRef = z.object({
  packageInstanceId: PackageInstanceId,
  backend: PackageBackend,
  packageName: z.string().min(1).max(256),          // canonical/ref form (length-bounded); ACL allowlist applied at exec (§8.2, NFR-7, F1)
  version: z.string().min(1).max(128).nullable(),
  scope: ScopeTag.nullable(),                        // user vs system unit/package
});

export const Evidence = z.object({
  kind: z.enum([
    "dpkg-record", "snap-list-record", "flatpak-list-record",     // manifest-backed (FR-5 AC1)
    "appimage-magic", "appimage-integration",                     // D9
    "pip-record", "npm-record", "oci-manifest",                   // D9
    "desktop-entry", "file-path", "reverse-dependency",
  ]),
  detail: z.string().min(1).max(1024),               // human + machine readable proof
});

export const InstallSource = z.object({
  method: InstallMethod,
  confidence: ConfidenceLevel,
  evidence: z.array(Evidence).min(1),                // >=1 evidence required (FR-4 AC1, FR-5)
  packageRef: PackageInstanceRef.nullable(),         // null only for purely manual installs
});

export const DesktopEntryRef = z.object({
  path: PathString,
  appId: z.string().min(1).max(256).nullable(),      // reverse-DNS desktop appId, if present
  icon: z.string().min(1).max(1024).nullable(),
  mimetypes: z.array(z.string().min(1).max(128)),
});

export const Application = z.object({
  canonicalAppId: CanonicalAppId,
  name: z.string().min(1).max(256),
  desktopEntry: DesktopEntryRef.nullable(),
  installSources: z.array(InstallSource).min(1),     // INVARIANT (DOMAIN §4.1): >=1, else manual-classified
  packageInstanceRefs: z.array(PackageInstanceRef),
  isProtected: z.boolean(),                          // FR-20 / D7
  instancesDisambiguated: z.boolean(),               // FR-7 AC2: planning blocked until true
});
```

### 3.3 ResidueGraph `[FR-8..FR-13]`

```ts
export const Artifact = z.object({
  artifactId: ArtifactId,
  category: ArtifactCategory,                        // FR-9: exactly one
  target: PathString,                                // filesystem path or system-object reference
  ownerSet: z.array(CanonicalAppId).min(1),          // INVARIANT (FR-8 AC2): includes owning app
  usageKind: UsageKind,                              // FR-11: derived from ownerSet (pure function)
  sizeBytes: z.number().int().nonnegative().nullable(),
  deletable: z.boolean(),                            // false for usageKind === 'system' (FR-11 AC2)
  confidence: ConfidenceLevel,                       // FR-12 AC3: heuristic => 'low'
  discoveredBy: DiscoverySource,                     // FR-12 / D4
  scope: ScopeTag.nullable(),                        // user vs system location (FR-22)
});

export const ResidueGraph = z.object({
  applicationId: CanonicalAppId,
  scanVersion: ScanVersion,                          // FR-8 AC3: re-scan => new version, prior immutable
  sealedAt: IsoTimestamp,
  artifacts: z.array(Artifact),
  counts: z.object({
    exclusive: z.number().int().nonnegative(),
    shared: z.number().int().nonnegative(),
    system: z.number().int().nonnegative(),
  }),
  runtimeWarnings: z.array(RuntimeWarning),          // FR-28: running processes / held locks
});

export const RuntimeWarning = z.object({             // FR-28 / D10
  kind: z.enum(["process-running", "lock-held", "service-active"]),
  detail: z.string().min(1).max(1024),
  autoStoppable: z.boolean(),                        // app-owned service => graceful stop attempted
});
```

### 3.4 RemovalPlan `[FR-14..FR-22]`

```ts
export const OperationTarget = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("artifact"), artifactId: ArtifactId }),
  z.object({ kind: z.literal("package"), packageInstanceId: PackageInstanceId }),
]);

export const RemovalOperation = z.object({
  operationId: OperationId,
  order: z.number().int().nonnegative(),             // FR-17 execution order
  action: Action,                                    // FR-31
  target: OperationTarget,
  verdict: SafetyVerdict,                            // FR-16 (exactly one)
  impact: z.array(CanonicalAppId),                   // FR-15: OTHER apps that break (non-empty => != 'safe')
  rationale: z.string().min(1).max(1024),
  accepted: z.boolean().default(false),              // FR-18 AC2: recorded risky acceptance
});

export const RemovalPlan = z.object({
  planId: PlanId,
  applicationId: CanonicalAppId,
  scanVersion: ScanVersion,
  mode: RemovalMode,                                 // FR-21
  scope: RemovalScope,                               // FR-22
  status: PlanStatus,                                // FR-14 AC2: immutable once 'approved'
  operations: z.array(RemovalOperation),
  projectedSnapshotBytes: z.number().int().nonnegative(),
  exceedsCostCap: z.boolean(),                       // FR-23 AC2 / D11
  blockedReasons: z.array(z.string().min(1).max(256)),
  composedAt: IsoTimestamp,
});
```

**Plan invariants enforced by the `RemovalPlan` aggregate (code stage):**
- `operations` is ordered: `stop-service` < `uninstall-package` < `delete-file`/`remove-association` <
  `prune-orphan` (last) `[FR-17]`; dependents precede dependencies `[FR-17 AC1/AC2]`.
- `verdict === 'blocked'` for any `system`/shared-non-acceptable operation; `plan.status` may not become
  `'approved'` while any `blocked` operation remains `[FR-16 AC2, FR-18 AC1]`.
- `verdict !== 'safe'` whenever `impact.length > 0` `[FR-15 AC2]`.
- `mode === 'remove'` yields only `uninstall-package` operations `[FR-21 AC1]`; `mode === 'purge'` adds
  exclusive `config`/`data`/`cache` deletes `[FR-21 AC2]`.
- `scope === 'current-user'` restricts operations to `scope === 'user'` artifacts/units `[FR-22 AC1]`.
- A `RuntimeWarning` (`§3.3`) on an operation's target forces its verdict away from `safe`: an
  `autoStoppable` running service / `process-running` with recorded user acceptance → `risky`; a
  non-`autoStoppable` or un-releasable `lock-held` → `manual-review`. Such operations are never
  auto-executed `[FR-28 AC2, D10, F5]`.

### 3.5 Snapshot `[FR-23, D6, D11]`

```ts
export const SnapshotEntry = z.object({
  artifactId: ArtifactId.nullable(),                 // null for package-record / unit-record
  category: ArtifactCategory,
  originalPath: PathString,
  blobPath: PathString.nullable(),                   // null when cache-excluded or reinstall-only
  checksum: z.string().length(64),                   // sha256 hex (NFR-15)
  sizeBytes: z.number().int().nonnegative(),
  kind: SnapshotEntryKind,                           // D6: files+units backed up; cache excluded
  packageName: z.string().min(1).max(256).nullable(),// for package-record reinstall (best-effort, D6)
});

export const Snapshot = z.object({
  snapshotId: SnapshotId,
  jobId: JobId,
  capturedAt: IsoTimestamp,
  checksumAlgo: z.literal("sha256"),
  manifestChecksum: z.string().length(64),           // NFR-15: over the entries manifest
  entries: z.array(SnapshotEntry),
  totalBytes: z.number().int().nonnegative(),
  excludesCache: z.boolean().default(true),          // D6 / FR-23 AC3
  blobRoot: PathString,
});
```

### 3.6 RemovalJob & ExecutedStep `[FR-24..FR-29]`

```ts
export const ErrorDetail = z.object({
  code: z.string().min(1).max(128),                  // machine code, e.g. 'PKG_UNINSTALL_FAILED'
  message: z.string().min(1).max(2048),
  retryable: z.boolean(),
});

export const ExecutedStep = z.object({               // FR-29
  stepId: StepId,
  operationId: OperationId,
  order: z.number().int().nonnegative(),
  status: StepStatus,
  startedAt: IsoTimestamp.nullable(),
  finishedAt: IsoTimestamp.nullable(),
  error: ErrorDetail.nullable(),                     // FR-29 AC2
});

export const RemovalJob = z.object({
  jobId: JobId,
  planId: PlanId,
  snapshotId: SnapshotId.nullable(),                 // INVARIANT: set before status leaves 'created' (FR-23 AC1)
  status: JobStatus,                                 // FR-26 state machine
  steps: z.array(ExecutedStep),
  createdAt: IsoTimestamp,
  startedAt: IsoTimestamp.nullable(),
  finishedAt: IsoTimestamp.nullable(),
  failure: ErrorDetail.nullable(),
});
```

### 3.7 AuditRecord `[FR-33, NFR-9]`

```ts
export const AuditRecord = z.object({
  auditRecordId: AuditRecordId,
  jobId: JobId,
  planSnapshot: RemovalPlan,                         // immutable deep copy (FR-33)
  steps: z.array(ExecutedStep),
  snapshotId: SnapshotId,
  outcome: z.enum(["completed", "rolled_back"]),     // FR-33 terminal outcomes only (ORM §6.1 agrees; F2)
  undoable: z.boolean(),                             // FR-34: restorable iff snapshot intact
  createdAt: IsoTimestamp,
  hash: z.string().length(64),                       // NFR-9: sha256 over (prevHash || canonical record)
  prevHash: z.string().length(64).nullable(),        // NFR-9: hash chain (genesis record has null)
});
```

---

## 4. Job State Machine & Domain Events `[FR-26, DOMAIN §4.4/§4.6]`

### 4.1 Allowed transitions (enforced in `execution/`)

| From | To | Guard | Realizes |
|---|---|---|---|
| (none) | `created` | source `RemovalPlan.status === 'approved'`; **Snapshot captured** | FR-23 AC1, FR-24 |
| `created` | `running` | `begin()` invoked | FR-24 AC1 |
| `running` | `completed` | all steps `succeeded` | FR-26 |
| `running` | `failed` | a step errored | FR-25 AC1 |
| `failed` | `rolled_back` | `rollback(Snapshot)` completes | FR-25 AC1/AC2 |
| any other | any | **rejected** | FR-26 AC1 |

`completed → running` and `failed → completed` are forbidden; a partial removal is never terminal
`[FR-25 AC2, FR-26 AC1]`. On any step failure the runner automatically drives `failed → rolled_back`
before returning a terminal state `[FR-25]`. The `RemovalPlan.status === 'approved'` guard is enforced at
**both** job creation (`POST /jobs`) and `begin()` (`POST /jobs/{id}/begin`), so a Snapshot is never
captured against a non-approved plan `[F3]`.

### 4.2 Domain event stream → AuditRecord projection `[R10, NFR-9, NFR-13]`

Each event is emitted **after** its invariant holds, appended to `event_log` (§6), and projected into
exactly one immutable `AuditRecord` at job termination `[DOMAIN §4.6]`. Progress endpoints read this stream
`[FR-29, NFR-13]`.

| Event | Emitted when | Carries | Realizes |
|---|---|---|---|
| `ApplicationResolved` | Canonical identity + provenance established | `Application` | FR-4 |
| `DisambiguationRequired` | distinct instances exist, not yet chosen | candidate `Application[]` | FR-7 |
| `ResidueGraphSealed` | scan completed, immutable at `ScanVersion` | `ResidueGraph` ref | FR-8 |
| `RemovalPlanComposed` | plan computed (Dry Run output) | `RemovalPlan` | FR-14, FR-19 |
| `RemovalPlanApproved` | every `risky` acceptance recorded, no `blocked` | `planId`, accepted op ids | FR-18 |
| `SnapshotCaptured` | pre-execution snapshot taken | `Snapshot` ref | FR-23 |
| `StepExecuted` / `StepFailed` | one operation succeeded/failed | `ExecutedStep` | FR-29 |
| `JobCompleted` / `JobRolledBack` | terminal | `outcome` | FR-25, FR-26 |
| `LeftoversDetected` | post-completion sweep found missed residue | fresh `ResidueGraph` ref | FR-27 |

---

## 5. Policy & Configuration `[D1, D7, D11]`

Configuration is loaded from `${XDG_CONFIG_HOME:-$HOME/.config}/app-remover/config.json` and validated by a
Zod schema at startup. Defaults are normative; only listed keys may be overridden.

### 5.1 Residue policy `[D1]`

Defines artifact roots by category. Categories are fixed (§3.1); policy only adjusts roots.

```jsonc
{
  "residuePolicy": {
    "binary":        { "system": ["/usr/bin", "/usr/lib", "/opt"], "user": [] },
    "config":        { "system": ["/etc/<app>"], "user": ["~/.config/<app>"] },
    "cache":         { "system": ["/var/cache/<app>"], "user": ["~/.cache/<app>"] },
    "data":          { "system": ["/var/lib/<app>"], "user": ["~/.local/share/<app>"] },
    "state":         { "system": ["/run", "/var/run"], "user": [], "extra": ["dconf:/<app>", "gsettings:/<app>"] },
    "service":       { "system": ["/etc/systemd/system", "/lib/systemd/system"], "user": ["~/.config/systemd/user"] },
    "desktop-entry": { "system": ["/usr/share/applications"], "user": ["~/.local/share/applications"] },
    "association":   { "system": ["/usr/share/applications/defaults.list"], "user": ["~/.config/mimeapps.list"] },
    "dependency":    { "resolvedVia": "reverse-dependency-query" }
  }
}
```

`<app>` is bound to the resolved package/app name at scan time; additions/exceptions do not change the
category model `[D1]`.

### 5.2 Protected-Apps blocklist `[D7, FR-20]`

```jsonc
{
  "blocklist": {
    "core": [                                          // NON-OVERRIDABLE (FR-20 AC2)
      "app-remover",                                   // App-Remover itself
      { "match": "exact", "names": ["gnome-shell", "gnome-session", "kde-plasma", "xfce4-session"] },
      { "match": "exact", "names": ["glibc", "libc6", "libstdc++6"] },
      { "match": "exact", "names": ["xorg", "xwayland", "mutter", "kwin"] },
      { "match": "exact", "names": ["systemd"] },
      { "match": "exact", "names": ["dpkg", "apt", "snapd", "flatpak"] }
    ],
    "user": []                                         // overridable additions by the user
  }
}
```

Any target matching `core` is refused with reason `"protected-app"` and cannot be overridden `[FR-20]`.

### 5.3 Snapshot cost cap `[D11, FR-23 AC2]`

```jsonc
{ "maxSnapshotBytes": 1073741824 }   // 1 GiB default; FR-23 AC2 => warn + abort before deletion unless opt-in
```

---

## 6. ORM / Data Models (Drizzle ORM + SQLite)

Relational persistence for aggregate metadata + audit; snapshot **file blobs** live on the filesystem
(paths referenced from `snapshots`/`snapshot_entries`) `[D6]`. All enums are enforced by SQLite `CHECK`
constraints; all foreign keys are `ON DELETE RESTRICT`. Timestamps are integer epoch-millis
(`integer({ mode: "timestamp_ms" })`). JSON aggregates are stored as `text({ mode: "json" })`.

### 6.1 Schema definition

```ts
// persistence/schema.ts  — Drizzle ORM (SQLite dialect)
import type { AnySQLiteColumn } from "drizzle-orm/sqlite-core";
import { sqliteTable, text, integer, primaryKey, index, uniqueIndex } from "drizzle-orm/sqlite-core";

// --- Inventory / Provenance (read model + canonical identity) [FR-1..FR-7] ---
export const applications = sqliteTable("applications", {
  canonicalAppId:        text("canonical_app_id").primaryKey().notNull(),       // CanonicalAppId
  name:                  text("name").notNull(),
  desktopEntryPath:      text("desktop_entry_path"),                            // nullable
  desktopAppId:          text("desktop_app_id"),
  isProtected:           integer("is_protected", { mode: "boolean" }).notNull().default(false), // FR-20
  disambiguated:         integer("disambiguated", { mode: "boolean" }).notNull().default(false),// FR-7
  firstSeenAt:           integer("first_seen_at", { mode: "timestamp_ms" }).notNull(),
});

export const install_sources = sqliteTable("install_sources", {
  installSourceId:       integer("install_source_id").primaryKey({ autoIncrement: true }),
  canonicalAppId:        text("canonical_app_id").notNull()
                          .references(() => applications.canonicalAppId),
  method:                text("method").notNull().$type<"deb"|"snap"|"flatpak"|"appimage"|"manual"|"language-package"|"container">(),
  confidence:            text("confidence").notNull().$type<"high"|"medium"|"low">(),
  evidence:              text("evidence", { mode: "json" }).notNull().$type<unknown[]>(),       // Evidence[]
}, (t) => ({ appIdx: index("ix_install_sources_app").on(t.canonicalAppId) }));

export const package_instances = sqliteTable("package_instances", {
  packageInstanceId:     text("package_instance_id").primaryKey().notNull(),
  canonicalAppId:        text("canonical_app_id").notNull()
                          .references(() => applications.canonicalAppId),
  backend:               text("backend").notNull().$type<PackageBackendId>(),
  packageName:           text("package_name").notNull(),
  version:               text("version"),
  scope:                 text("scope").$type<"system"|"user">(),
}, (t) => ({ uniq: uniqueIndex("ux_package_instances").on(t.backend, t.packageName, t.scope) }));

// --- ResidueGraph [FR-8..FR-13] ---
export const scans = sqliteTable("scans", {
  scanRowId:             integer("scan_row_id").primaryKey({ autoIncrement: true }),
  canonicalAppId:        text("canonical_app_id").notNull()
                          .references(() => applications.canonicalAppId),
  scanVersion:           integer("scan_version").notNull(),                     // ScanVersion (FR-8 AC3)
  sealedAt:              integer("sealed_at", { mode: "timestamp_ms" }).notNull(),
}, (t) => ({ pk: primaryKey({ columns: [t.canonicalAppId, t.scanVersion] }) }));

export const artifacts = sqliteTable("artifacts", {
  artifactId:            text("artifact_id").primaryKey().notNull(),
  canonicalAppId:        text("canonical_app_id").notNull(),
  scanVersion:           integer("scan_version").notNull(),
  category:              text("category").notNull()
                          .$type<"binary"|"config"|"cache"|"data"|"state"|"service"|"desktop-entry"|"association"|"dependency">(),
  target:                text("target").notNull(),
  ownerSet:              text("owner_set", { mode: "json" }).notNull().$type<string[]>(),       // CanonicalAppId[]
  usageKind:             text("usage_kind").notNull().$type<"exclusive"|"shared"|"system">(),
  sizeBytes:             integer("size_bytes"),
  deletable:             integer("deletable", { mode: "boolean" }).notNull(),
  confidence:            text("confidence").notNull(),
  discoveredBy:          text("discovered_by").notNull(),
  scope:                 text("scope").$type<"system"|"user">(),
}, (t) => ({ scanIdx: index("ix_artifacts_scan").on(t.canonicalAppId, t.scanVersion) }));

// --- RemovalPlan [FR-14..FR-22] ---
export const plans = sqliteTable("plans", {
  planId:                text("plan_id").primaryKey().notNull(),
  canonicalAppId:        text("canonical_app_id").notNull(),
  scanVersion:           integer("scan_version").notNull(),
  mode:                  text("mode").notNull().$type<"remove"|"purge">(),
  scope:                 text("scope").notNull().$type<"system-wide"|"current-user"|"both">(),
  status:                text("status").notNull().$type<"draft"|"approved"|"superseded">(),
  projectedSnapshotBytes:integer("projected_snapshot_bytes").notNull(),
  exceedsCostCap:        integer("exceeds_cost_cap", { mode: "boolean" }).notNull(),
  composedAt:            integer("composed_at", { mode: "timestamp_ms" }).notNull(),
});

export const plan_operations = sqliteTable("plan_operations", {
  operationId:           text("operation_id").primaryKey().notNull(),
  planId:                text("plan_id").notNull().references(() => plans.planId),
  order:                 integer("order").notNull(),
  action:                text("action").notNull()
                          .$type<"uninstall-package"|"stop-service"|"delete-file"|"remove-association"|"prune-orphan">(),
  targetKind:            text("target_kind").notNull().$type<"artifact"|"package">(),
  targetRef:             text("target_ref").notNull(),
  verdict:               text("verdict").notNull().$type<"safe"|"risky"|"blocked"|"manual-review">(),
  impact:                text("impact", { mode: "json" }).notNull().$type<string[]>(),
  rationale:             text("rationale").notNull(),
  accepted:              integer("accepted", { mode: "boolean" }).notNull().default(false),
}, (t) => ({ orderIdx: index("ix_plan_ops_order").on(t.planId, t.order) }));

// --- Accepted risky operations (recorded user acceptance) [FR-18 AC2] ---
export const accepted_risks = sqliteTable("accepted_risks", {
  planId:                text("plan_id").notNull().references(() => plans.planId),
  operationId:           text("operation_id").notNull().references(() => plan_operations.operationId),
  acceptedAt:            integer("accepted_at", { mode: "timestamp_ms" }).notNull(),
}, (t) => ({ pk: primaryKey({ columns: [t.planId, t.operationId] }) }));

// --- Execution [FR-23..FR-29] ---
export const jobs = sqliteTable("jobs", {
  jobId:                 text("job_id").primaryKey().notNull(),
  planId:                text("plan_id").notNull().references(() => plans.planId),
  snapshotId:            text("snapshot_id"),                                    // set before leaving 'created' (FR-23 AC1)
  status:                text("status").notNull()
                          .$type<"created"|"running"|"completed"|"failed"|"rolled_back">(),
  createdAt:             integer("created_at", { mode: "timestamp_ms" }).notNull(),
  startedAt:             integer("started_at", { mode: "timestamp_ms" }),
  finishedAt:            integer("finished_at", { mode: "timestamp_ms" }),
  failure:               text("failure", { mode: "json" }).$type<unknown>(),
}, (t) => ({ statusIdx: index("ix_jobs_status").on(t.status) }));

export const executed_steps = sqliteTable("executed_steps", {
  stepId:                text("step_id").primaryKey().notNull(),
  jobId:                 text("job_id").notNull().references(() => jobs.jobId),
  operationId:           text("operation_id").notNull(),
  order:                 integer("order").notNull(),
  status:                text("status").notNull()
                          .$type<"pending"|"running"|"succeeded"|"failed"|"skipped">(),
  startedAt:             integer("started_at", { mode: "timestamp_ms" }),
  finishedAt:            integer("finished_at", { mode: "timestamp_ms" }),
  error:                 text("error", { mode: "json" }).$type<unknown>(),
}, (t) => ({ jobIdx: index("ix_steps_job").on(t.jobId, t.order) }));

// --- Snapshot (metadata; blobs on filesystem) [FR-23, D6, NFR-15] ---
export const snapshots = sqliteTable("snapshots", {
  snapshotId:            text("snapshot_id").primaryKey().notNull(),
  jobId:                 text("job_id").notNull().references(() => jobs.jobId),
  capturedAt:            integer("captured_at", { mode: "timestamp_ms" }).notNull(),
  checksumAlgo:          text("checksum_algo").notNull().default("sha256"),
  manifestChecksum:      text("manifest_checksum").notNull(),                   // NFR-15
  totalBytes:            integer("total_bytes").notNull(),
  excludesCache:         integer("excludes_cache", { mode: "boolean" }).notNull().default(true),
  blobRoot:              text("blob_root").notNull(),
});

export const snapshot_entries = sqliteTable("snapshot_entries", {
  entryRowId:            integer("entry_row_id").primaryKey({ autoIncrement: true }),
  snapshotId:            text("snapshot_id").notNull().references(() => snapshots.snapshotId),
  artifactId:            text("artifact_id"),
  category:              text("category").notNull(),
  originalPath:          text("original_path").notNull(),
  blobPath:              text("blob_path"),
  checksum:              text("checksum").notNull(),                            // sha256 hex, 64 chars
  sizeBytes:             integer("size_bytes").notNull(),
  kind:                  text("kind").notNull().$type<"file-backup"|"package-record"|"unit-record">(),
  packageName:           text("package_name"),
}, (t) => ({ snapIdx: index("ix_snapshot_entries").on(t.snapshotId) }));

// --- Audit (append-only; hash-chained) [FR-33, NFR-9] ---
export const audit_records = sqliteTable("audit_records", {
  auditRecordId:         text("audit_record_id").primaryKey().notNull(),
  jobId:                 text("job_id").notNull().references(() => jobs.jobId),
  planSnapshot:          text("plan_snapshot", { mode: "json" }).notNull().$type<unknown>(), // immutable RemovalPlan copy
  steps:                 text("steps", { mode: "json" }).notNull().$type<unknown[]>(),
  snapshotId:            text("snapshot_id").notNull(),
  outcome:               text("outcome").notNull().$type<"completed"|"rolled_back">(),
  undoable:              integer("undoable", { mode: "boolean" }).notNull(),
  createdAt:             integer("created_at", { mode: "timestamp_ms" }).notNull(),
  hash:                  text("hash").notNull(),                                 // sha256 hex
  prevHash:              text("prev_hash"),                                      // genesis => null
});

// --- Event log (append-only projection; progress + observability) [DOMAIN §4.6, NFR-13] ---
export const event_log = sqliteTable("event_log", {
  eventId:               integer("event_id").primaryKey({ autoIncrement: true }),
  jobId:                 text("job_id"),
  type:                  text("type").notNull(),                                 // e.g. 'StepExecuted'
  payload:               text("payload", { mode: "json" }).notNull().$type<unknown>(),
  emittedAt:             integer("emitted_at", { mode: "timestamp_ms" }).notNull(),
}, (t) => ({ jobIdx: index("ix_event_log_job").on(t.jobId) }));
```

`type PackageBackendId = "deb"|"snap"|"flatpak"|"appimage"|"pip"|"npm"|"manual"|"systemd"`.

### 6.2 Persistence responsibilities

| Concern | Mechanism | Realizes |
|---|---|---|
| Immutability of audit | Repository-level contract: the repository exposes `insertAuditRecord` only — no `update`/`delete` on `audit_records`. Enforced by code-review gate + a unit test asserting no mutating query targets that table (this is the NFR-9 tamper/append-only test). The guarantee is **code-level, not a DB trigger** — the table has no UPDATE/DELETE path by construction `[F6]`. | NFR-9, FR-33 |
| Tamper-evidence | Each record's `hash = sha256(prevHash ‖ canonical(record))`; `prevHash` chains to the prior record. | NFR-9 |
| Snapshot integrity | `manifestChecksum` recomputed on restore/verify; mismatch ⇒ Undo refused. | NFR-15 |
| Plan immutability once approved | `plans.status` transitions `draft → approved` only; a re-compose inserts a **new** `planId` and sets the prior `status = 'superseded'`. | FR-14 AC2 |
| Owner-set authority | Computed in Residue Mapping and stored denormalized in `artifacts.owner_set`; never recomputed from raw ACL at planning time. | FR-10, D2 |

---

## 7. Application API (OpenAPI 3.1 style) `[FR-36, D13]`

### 7.1 Server / transport

```yaml
openapi: 3.1.0
info:
  title: App-Remover Application API
  version: 1.0.0
servers:
  - url: http://unix:/run/user/{uid}/app-remover.sock:/api/v1   # primary (D13)
    variables: { uid: { default: "1000" } }
  - url: http://127.0.0.1:{port}/api/v1                          # loopback fallback
    variables: { port: { default: "17763" } }
```

The server binds the socket with mode `0600` owned by the desktop user; no remote interface is exposed
`[D13]`. All request bodies and responses are JSON. Every response uses the error envelope (§7.4) on
failure. `[FR-36]`

### 7.2 Endpoint overview `[FR-36 AC1]`

| Method | Path | Operation | Realizes |
|---|---|---|---|
| GET | `/health` | Service liveness + backend availability | FR-1 AC2, NFR-13, NFR-14 |
| GET | `/backends` | List PM backends + presence | FR-30, NFR-14 |
| GET | `/inventory` | Enumerate installed apps (`?q=` search) | FR-1, FR-2, NFR-1 |
| GET | `/inventory/{canonicalAppId}` | Fetch one app | FR-2 |
| POST | `/resolve` | Resolve entry/id → Canonical Application | FR-3, FR-4, FR-5, FR-6 |
| POST | `/resolve/disambiguate` | Choose distinct instance(s) | FR-7 |
| POST | `/scans` | Build ResidueGraph | FR-8..FR-13, NFR-2 |
| GET | `/scans/{canonicalAppId}/{scanVersion}` | Read a sealed graph | FR-8 |
| POST | `/plans` | Compose plan (Dry Run) | FR-14..FR-22, FR-19, NFR-3 |
| GET | `/plans/{planId}` | Read a plan | FR-14 |
| POST | `/plans/{planId}/approve` | Record risk acceptance + approve | FR-18, FR-20 |
| POST | `/jobs` | Create job + capture snapshot | FR-23, FR-24 |
| POST | `/jobs/{jobId}/begin` | Start execution | FR-24, FR-26 |
| GET | `/jobs/{jobId}` | Job + step progress | FR-26, FR-29, NFR-13 |
| POST | `/jobs/{jobId}/sweep` | Leftover sweep (post-completion) | FR-27 |
| GET | `/history` | List removal history | FR-35 |
| GET | `/audit/{auditRecordId}` | Read an audit record | FR-33, FR-35, NFR-9 |
| POST | `/audit/{auditRecordId}/undo` | Restore from snapshot | FR-34, NFR-6, NFR-15 |
| GET | `/snapshots/{snapshotId}` | Snapshot metadata | FR-23 |
| POST | `/snapshots/{snapshotId}/verify` | Verify snapshot integrity | NFR-15 |

`[FR-36 AC2]` Every capability is reachable without a GUI; the client is a pure consumer of these endpoints.

### 7.3 Component schemas (DTOs)

Request/response bodies reference the Zod schemas in §3 by name (they ARE the JSON Schemas). Additional
DTOs:

```ts
// --- Inventory [FR-1, FR-2] ---
export const InventoryItemDTO = z.object({
  canonicalAppId: CanonicalAppId,
  name: z.string(),
  installMethods: z.array(InstallMethod).min(1),    // >=1 (FR-1 AC1)
  version: z.string().nullable(),
  icon: z.string().nullable(),
  instanceCount: z.number().int().positive(),       // >1 => disambiguation may be required (FR-7)
});
export const InventoryDTO = z.object({
  items: z.array(InventoryItemDTO),
  skippedBackends: z.array(PackageBackend),         // absent backends (FR-1 AC2 / NFR-14)
  generatedAt: IsoTimestamp,
});

// --- Resolve [FR-3..FR-7] ---
export const ResolveRequest = z.object({
  source: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("desktop-entry"), path: PathString }),  // FR-3
    z.object({ kind: z.literal("canonical-id"), canonicalAppId: CanonicalAppId }),
  ]),
});
export const ResolveResult = z.discriminatedUnion("status", [
  z.object({ status: z.literal("resolved"), application: Application }),                 // FR-4/FR-6
  z.object({ status: z.literal("disambiguation-required"),                              // FR-7
             resolveToken: ResolveToken,
             candidates: z.array(Application).min(2) }),
  z.object({ status: z.literal("unresolvable"), reason: z.string() }),                  // FR-4 AC2
]);
export const DisambiguateRequest = z.object({
  resolveToken: ResolveToken,
  selectedCanonicalAppIds: z.array(CanonicalAppId).min(1),                              // FR-7 AC1
});

// --- Scan [FR-8..FR-13] ---
export const ScanRequest = z.object({
  canonicalAppId: CanonicalAppId,
  scope: RemovalScope.default("both"),                // FR-22
});

// --- Plan / Dry Run [FR-14..FR-22] ---
export const ComposePlanRequest = z.object({
  canonicalAppId: CanonicalAppId,
  scanVersion: ScanVersion,
  mode: RemovalMode.default("remove"),               // FR-21
  scope: RemovalScope.default("both"),               // FR-22
});
export const ApproveRequest = z.object({
  acceptedRiskOperationIds: z.array(OperationId).default([]),  // FR-18 AC2: risky acceptances
});

// --- Execution [FR-23..FR-27] ---
export const CreateJobRequest = z.object({
  planId: PlanId,
  proceedDespiteCostCap: z.boolean().default(false),  // FR-23 AC2 opt-in
});
export const SweepResult = z.object({                 // FR-27
  missedResidueGraph: ResidueGraph.nullable(),        // null when clean
  followUpPlanId: PlanId.nullable(),                  // offered, not auto-executed (FR-27 AC2)
});

// --- Audit / Undo / History [FR-33..FR-35] ---
export const HistoryItemDTO = z.object({
  auditRecordId: AuditRecordId,
  jobId: JobId,
  targetName: z.string(),
  outcome: z.enum(["completed", "rolled_back"]),
  createdAt: IsoTimestamp,
  undoable: z.boolean(),                              // FR-35 AC1
});
export const UndoResult = z.object({                  // FR-34
  restoreJobId: JobId,                                // restore runs as a job (best-effort packages, D6)
  deferredPackages: z.array(z.string()),              // FR-34 AC2: reinstall deferred (offline)
});

// --- Health / backends [FR-1 AC2, FR-30, NFR-14] ---
export const HealthDTO = z.object({
  status: z.enum(["ok", "degraded"]),
  backends: z.array(z.object({ backend: PackageBackend, present: z.boolean() })),
  dbWritable: z.boolean(),
});
export const SnapshotVerifyResult = z.object({        // NFR-15
  snapshotId: SnapshotId,
  intact: z.boolean(),
  failures: z.array(z.object({ entryPath: PathString, reason: z.string() })),
});

// --- Common ---
export const ApiError = z.object({                    // §7.4
  error: z.object({
    code: z.string(),                                 // machine code, e.g. 'PLAN_HAS_BLOCKED_OP'
    message: z.string(),
    details: z.record(z.string(), z.unknown()).optional(),
  }),
});
```

### 7.4 Error model

| HTTP | `error.code` | Meaning | Realizes |
|---|---|---|---|
| 400 | `VALIDATION_ERROR` | Zod request validation failed | NFR-7 |
| 404 | `NOT_FOUND` | Resource id unknown | — |
| 409 | `DISAMBIGUATION_REQUIRED` | Distinct instances, not chosen | FR-7 AC2 |
| 409 | `PLAN_NOT_APPROVED` | `begin()` on an unapproved plan | FR-24 AC1 |
| 409 | `PLAN_HAS_BLOCKED_OP` | Approve attempted with a `blocked` op | FR-18 AC1 |
| 409 | `RISK_NOT_ACCEPTED` | Approve attempted with unaccepted `risky` op | FR-18 AC2 |
| 409 | `PROTECTED_APP` | Target is on the core blocklist | FR-20 |
| 409 | `ILLEGAL_TRANSITION` | Job state-machine violation (incl. sweep on non-completed/non-Purge job) | FR-26 AC1, F8 |
| 409 | `CONFLICTING_JOB` | A job is already in-flight for this `CanonicalAppId` (single-write lock, §2) | FR-28, F8 |
| 422 | `SNAPSHOT_COST_CAP_EXCEEDED` | Projected snapshot over cap, no opt-in | FR-23 AC2 |
| 422 | `SNAPSHOT_CORRUPT` | Snapshot failed integrity check; Undo refused | NFR-15 |
| 500 | `INTERNAL_ERROR` | Unexpected failure (logged, no secrets) | NFR-8 |

### 7.5 Operation contracts (OpenAPI path-item style)

Each entry below is an OpenAPI 3.1 `paths` path-item. `responses` list only non-2xx when notable; all
operations also return `400 VALIDATION_ERROR` and `500 INTERNAL_ERROR`.

**`GET /health`** `[FR-1 AC2, NFR-13, NFR-14]`
- `200` → `HealthDTO`.

**`GET /backends`** `[FR-30, NFR-14]`
- `200` → `{ backends: Array<{ backend: PackageBackend, present: boolean }> }`.

**`GET /inventory?q={query}`** `[FR-1, FR-2, NFR-1]`
- parameters: `q` (string, optional, name substring).
- `200` → `InventoryDTO`. Absent backends appear in `skippedBackends`, not as errors `[FR-1 AC2]`.

**`GET /inventory/{canonicalAppId}`** `[FR-2]`
- `200` → `Application`; `404 NOT_FOUND`.

**`POST /resolve`** `[FR-3, FR-4, FR-5, FR-6]`
- requestBody: `ResolveRequest`.
- `200` → `ResolveResult` (`resolved` | `disambiguation-required` | `unresolvable`).

**`POST /resolve/disambiguate`** `[FR-7]`
- requestBody: `DisambiguateRequest`.
- `200` → `Application` (selected); `409 DISAMBIGUATION_REQUIRED` if selection still ambiguous.

**`POST /scans`** `[FR-8..FR-13, NFR-2]`
- requestBody: `ScanRequest`.
- `201` → `ResidueGraph`. Runtime warnings (`runtimeWarnings`) are populated here `[FR-28]`.
- `409 PROTECTED_APP` if target is protected; `409 DISAMBIGUATION_REQUIRED` if
  `Application.instancesDisambiguated === false` `[FR-7 AC2, F4]`.

**`GET /scans/{canonicalAppId}/{scanVersion}`** `[FR-8]`
- `200` → `ResidueGraph`; `404 NOT_FOUND`.

**`POST /plans`** `[FR-14..FR-22, FR-19, NFR-3]` *(this IS the Dry Run — D12)*
- requestBody: `ComposePlanRequest`.
- `201` → `RemovalPlan` (`status: "draft"`). No filesystem/package/service mutation occurs `[FR-19 AC1/AC2]`.
- `409 PROTECTED_APP` if target protected `[FR-20]`; `409 DISAMBIGUATION_REQUIRED` if the target is not yet
  disambiguated `[FR-7 AC2, F4]`.

**`GET /plans/{planId}`** `[FR-14]`
- `200` → `RemovalPlan`; `404 NOT_FOUND`.

**`POST /plans/{planId}/approve`** `[FR-18, FR-20]`
- requestBody: `ApproveRequest`.
- `200` → `RemovalPlan` (`status: "approved"`).
- `409 PLAN_HAS_BLOCKED_OP` `[FR-18 AC1]`; `409 RISK_NOT_ACCEPTED` `[FR-18 AC2]`. Blocked ops remain blocked
  on any force attempt `[FR-18 AC3]`.

**`POST /jobs`** `[FR-23, FR-24]`
- requestBody: `CreateJobRequest`.
- `201` → `RemovalJob` (`status: "created"`, `snapshotId` set) — snapshot captured before leaving `created`
  `[FR-23 AC1]`.
- `409 PLAN_NOT_APPROVED` when the source plan `status !== 'approved'`; the approval guard runs here **and**
  at `begin()` (§4.1) `[FR-24 AC1, F3]`.
- `409 CONFLICTING_JOB` when a job is already in-flight for this `CanonicalAppId` (single-write lock, §2)
  `[FR-28, F8]`.
- `422 SNAPSHOT_COST_CAP_EXCEEDED` when `exceedsCostCap && !proceedDespiteCostCap` `[FR-23 AC2]`.

**`POST /jobs/{jobId}/begin`** `[FR-24, FR-26]`
- `200` → `RemovalJob` (`status: "running"`).
- `409 PLAN_NOT_APPROVED` `[FR-24 AC1]`; `409 ILLEGAL_TRANSITION` otherwise `[FR-26 AC1]`.

**`GET /jobs/{jobId}`** `[FR-26, FR-29, NFR-13]`
- `200` → `RemovalJob` (current step + completed steps with statuses `[FR-29 AC1]`, errors+timestamps
  `[FR-29 AC2]`). On step failure the runner auto-rolls-back; the terminal state is `rolled_back`, never
  a half-removed `failed` `[FR-25 AC2]`.

**`POST /jobs/{jobId}/sweep`** `[FR-27]`
- `200` → `SweepResult`. Only valid on a `completed` Purge job; a follow-up plan is offered, not executed
  `[FR-27 AC2]`.
- `409 ILLEGAL_TRANSITION` if the job is not `completed` or the source plan `mode !== 'purge'` `[F8]`.

**`GET /history`** `[FR-35]`
- `200` → `{ items: HistoryItemDTO[] }`, newest first; `undoable` flags restorable records `[FR-35 AC1]`.

**`GET /audit/{auditRecordId}`** `[FR-33, FR-35, NFR-9]`
- `200` → `AuditRecord` (immutable) `[FR-33 AC1]`; `404 NOT_FOUND`.

**`POST /audit/{auditRecordId}/undo`** `[FR-34, NFR-6, NFR-15]`
- `201` → `UndoResult`. Restores backed-up `config`/`data` files and unit state; package reinstall is
  best-effort and reported via `deferredPackages` when offline `[FR-34 AC1/AC2, D6]`.
- `422 SNAPSHOT_CORRUPT` when integrity check fails — restore refused rather than partial `[NFR-15]`.

**`GET /snapshots/{snapshotId}`** `[FR-23]`
- `200` → `Snapshot`; `404 NOT_FOUND`.

**`POST /snapshots/{snapshotId}/verify`** `[NFR-15]`
- `201` → `SnapshotVerifyResult`.

### 7.6 Example client flows

**Flow A — Remove an app clicked from the desktop (happy path):**
1. `POST /resolve` `{ source: { kind: "desktop-entry", path: "/usr/share/applications/firefox.desktop" } }`
   → `resolved` `Application` (apt + snap instances merged into one `canonicalAppId`) `[FR-3, FR-4, FR-6]`.
2. `POST /scans` `{ canonicalAppId, scope: "both" }` → `ResidueGraph` (config + service artifacts,
   `runtimeWarnings: []`) `[FR-8]`.
3. `POST /plans` `{ canonicalAppId, scanVersion, mode: "purge", scope: "both" }` → `RemovalPlan`
   (`status: "draft"`); this is the Dry Run `[FR-19]`.
4. Client reviews verdicts/impact; `POST /plans/{planId}/approve` `{ acceptedRiskOperationIds: [] }`
   → `status: "approved"` `[FR-18]`.
5. `POST /jobs` `{ planId }` → `created` (snapshot captured) `[FR-23]`; `POST /jobs/{jobId}/begin` →
   `running` `[FR-24]`; poll `GET /jobs/{jobId}` → `completed` `[FR-26]`.

**Flow B — Distinct apt + snap Firefox (disambiguation):** step 1 returns
`disambiguation-required` with two candidates; planning is blocked until
`POST /resolve/disambiguate` records the choice `[FR-7 AC1/AC2]`.

**Flow C — Undo a completed removal:** `GET /history` → pick an `undoable` record →
`POST /audit/{auditRecordId}/undo` → `UndoResult` (files/units restored; offline packages deferred)
`[FR-34, FR-35]`.

---

## 8. Package Manager ACL — uniform surface `[FR-30, FR-31, R1, R7]`

The core invokes **only** this interface; no `dpkg`/`snap`/`flatpak`/`systemctl` shell command appears
outside `acl/adapters/*` `[FR-30 AC2, NFR-12]`. Every method's inputs are validated by the **ACL
input-validation layer** (`§8.2`) on top of the `§3` schemas, and executed via
`node:child_process.execFile` with an **argument array** (`shell: false`) so identifiers cannot inject
shell arguments `[NFR-7, F1]`.

```ts
// acl/PackageBackendAdapter.ts
import type { PackageBackend, PackageInstanceRef, RemovalMode, ScopeTag } from "../domain/schemas.js";

export interface CommandResult {
  readonly ok: boolean;
  readonly stdout?: string;          // structured, parsed by the adapter; never echoed raw to logs (NFR-8)
  readonly error?: { code: string; message: string };
}

export interface PackageInstance {
  readonly packageInstanceId: string;
  readonly backend: PackageBackend;
  readonly packageName: string;
  readonly version: string | null;
  readonly scope: ScopeTag | null;
}

export interface FileOwnership {
  readonly path: string;
  readonly owners: readonly string[];   // CanonicalAppId[] that claim the path
}

export interface ServiceUnitRef {
  readonly unitName: string;            // validated; e.g. "firefox.service"
  readonly scope: ScopeTag;
}

export interface PackageBackendAdapter {
  readonly backendId: PackageBackend;
  readonly present: boolean;            // false => adapter no-ops (NFR-14)

  // --- Queries (FR-30) ---
  listInstalled(): Promise<readonly PackageInstance[]>;
  queryFileOwnership(paths: readonly string[]): Promise<readonly FileOwnership[]>;      // D2 signal (a)
  queryReverseDependencies(packageName: string): Promise<readonly PackageInstance[]>;   // FR-31 AC2 / D2 signal (b)
  queryOrphans(): Promise<readonly PackageInstance[]>;
  detectSharedPaths(): Promise<readonly FileOwnership[]>;                               // D2 signal (c)

  // --- Commands (FR-31) ---
  uninstallPackage(ref: PackageInstanceRef, mode: RemovalMode): Promise<CommandResult>;  // FR-31 AC1
  stopService(unit: ServiceUnitRef): Promise<CommandResult>;
  deleteFile(path: string): Promise<CommandResult>;
  pruneOrphan(ref: PackageInstanceRef): Promise<CommandResult>;
  reinstallPackage(ref: PackageInstanceRef): Promise<CommandResult>;                     // Undo (FR-34, D6)
}

// acl/index.ts — registry; absent backends return a present:false stub (NFR-14)
export interface BackendRegistry {
  get(backend: PackageBackend): PackageBackendAdapter;
  all(): readonly PackageBackendAdapter[];
}
```

Adapter roster: `deb` (`dpkg`/`apt`), `snap`, `flatpak`, `appimage`, `pip`, `npm`, `manual`, `systemd`
`[FR-30]`. Adding a backend touches only its adapter + registry registration `[NFR-12]`.

### 8.1 Owner-Set computation `[FR-10, D2]`

Blending (in the Residue Mapping context, before sealing the graph):
1. `queryFileOwnership` — authoritative where the PM tracks the path (deb/`dpkg`) `[D2 (a)]`;
2. `queryReverseDependencies` — package-level owners `[D2 (b)]`;
3. `detectSharedPaths` — heuristic for unpackaged files `[D2 (c)]`.

Result drives `usageKind` (`size === 1 && owner === target → exclusive`; `>1 → shared`; OS base-package
owner → `system`) `[FR-11]`, which is stored on `artifacts.usage_kind` and never recomputed downstream.

### 8.2 ACL input validation (defense-in-depth) `[NFR-7, F1]`

The `§3` identifier schemas are the **canonical/ref form** (length-bounded, JSON-serializable) used by the
domain and persisted by the ORM. They are *not* the injection barrier. Before any identifier reaches a
backend tool, the ACL layer re-validates it against a **backend-specific allowlist** and then constructs
the command via `execFile(argArray, { shell: false })`. The primary safety guarantee is the arg-array +
`shell:false` boundary; the allowlists are defense-in-depth.

| Backend | Identifier | Allowlist (basis) |
|---|---|---|
| `deb` | package name | `/^[a-z0-9][a-z0-9+.-]{0,255}$/` (Debian Policy §5.6.7) |
| `snap` | snap name | `/^[a-z0-9][a-z0-9-]{0,39}$/` (snap naming rules) |
| `flatpak` | app id | `/^[a-z0-9.][a-z0-9.-]{0,254}$/` (reverse-DNS) |
| `npm` | package name | `/^(@[a-z0-9-~][a-z0-9-._~]*\/)?[a-z0-9-~][a-z0-9-._~]*$/` (scoped; npm registry) |
| `pip` | dist name | `/^([A-Z0-9]|[A-Z0-9][A-Z0-9._-]*[A-Z0-9])$/i` (PEP 503, pre-normalize) |
| `systemd` | unit name | `/^[A-Za-z0-9@:_.+-]{1,255}\.(service|socket|timer|target|mount)$/` |
| all | path | absolute, no NUL byte; rejected if it escapes its residue-policy root (`../`, off-root symlink) |

A mismatched identifier is rejected with `VALIDATION_ERROR` and never reaches `execFile` `[NFR-7]`. Paths
are additionally confined to their residue-policy root (`§5.1`) so a crafted `target` cannot delete
arbitrary files `[NFR-4]`.

---

## 9. Security & Privilege `[FR-32, D5, NFR-7, NFR-8]`

| Concern | Design |
|---|---|
| Least-privilege elevation | Privileged operations route through a `PrivilegeGateway` that invokes `pkexec` against named polkit actions (e.g. `org.appremover.remove-system`), acquired **per operation** and released on completion. No persistent root shell `[FR-32 AC1/AC2, D5]`. |
| Injection safety | The actual barrier is `execFile` with arg arrays and `shell: false` (`§8`), which is injection-safe regardless of identifier content. Defense-in-depth: a dedicated **ACL input-validation layer** (`§8.2`) applies per-backend allowlists before exec; the `§3` schemas are the canonical storage/ref form (length-bounded) `[NFR-7, F1]`. |
| Local-only surface | HTTP binds a `0600` Unix socket owned by the desktop user; no network listener is required `[D13]`. |
| Secret hygiene | Backed-up file blobs are written `0600` under a `0700` root; logs scrub file contents, env vars, and credentials; only sizes/paths/checksums are logged `[NFR-8]`. |
| Audit tamper-evidence | Append-only `audit_records` with a sha256 hash chain; no UPDATE/DELETE path in the repository `[NFR-9]`. |
| Fuzzing | Identifier-allowlist and adapter-arg-construction paths are covered by injection/fuzz unit tests (code stage) `[NFR-7]`. |

---

## 10. NFR Realization

| NFR | Design mechanism | Section |
|---|---|---|
| NFR-1 inventory ≤ 8 s for ≤ 300 apps | Fan-out enumeration across present adapters in parallel; inventory cached as a read model | §7.5 `/inventory`, §6 `applications` |
| NFR-2 scan ≤ 30 s; cost cap pre-check | Scan completes ≤ 30 s. The projected snapshot size is computed from `artifacts.sizeBytes` at **plan time** (`RemovalPlan.projectedSnapshotBytes`/`exceedsCostCap`) and the cap is enforced at **job creation** (`POST /jobs`, before any capture or deletion) — so an over-cap job is reported before expensive/destructive work `[F7]` | §3.4, §5.3, §7.5 `/plans`, `/jobs` |
| NFR-3 Dry Run ≤ 15 s | Plan composition is in-memory over a sealed graph; no I/O mutations | §7.5 `/plans` |
| NFR-4 zero touches to shared/system | Operations are generated only for `usageKind === 'exclusive'`; `system`/`shared` yield `blocked`/omitted; verified by before/after checksum suite | §3.3/§3.4, §6 |
| NFR-5 ≥ 99 % auto-rollback | Runner enforces `failed → rolled_back` before terminal; step-failure injection tests | §4.1 |
| NFR-6 Undo restores ≥ 99 % (offline) | Snapshot stores file/unit backups; package reinstall deferred, not silently failed | §3.5, §7.5 `/undo` |
| NFR-7 least-privilege + injection-safe | polkit/pkexec per-op; Zod allowlists; `execFile(shell:false)`; fuzz tests | §8, §9 |
| NFR-8 no secret leakage | `0600` blobs, log scrubbing, no raw stdout echo | §9 |
| NFR-9 immutable audit | Append-only hash-chained `audit_records` | §3.7, §6.2 |
| NFR-10 Ubuntu 22.04/24.04 × GNOME/KDE/XFCE × deb/snap/flatpak | Adapter roster + CI matrix (code/CI stage) | §8 |
| NFR-11 ≤ 2 actions; safe-by-default | Dry Run is the default path; plain-language risk labels with concrete `impact` | §7.5, §3.4 |
| NFR-12 core free of backend symbols | One-directional dependency rule; backend-isolation dependency test | §1.3, §8 |
| NFR-13 queryable progress + structured logs | `event_log` + `/jobs/{id}`; JSON logs per operation | §4.2, §6, §7.5 |
| NFR-14 graceful degradation | `present:false` stub adapters; absent backends in `skippedBackends` | §7.5 `/health`,`/inventory`, §8 |
| NFR-15 integrity-verifiable snapshots | `manifestChecksum` + per-entry sha256; verify-before-restore; corrupt ⇒ Undo refused | §3.5, §6.2, §7.5 `/verify` |

---

## 11. Traceability Matrix (FR/NFR → spec artifact)

| Requirement | Domain schema (§3) | ORM table (§6) | Endpoint (§7) | Other |
|---|---|---|---|---|
| FR-1 | `Application`, `InventoryDTO` | `applications`, `package_instances` | `GET /inventory` | — |
| FR-2 | `Application` | `applications` | `GET /inventory/{id}` | — |
| FR-3 | `ResolveRequest` | — | `POST /resolve` | — |
| FR-4 | `Application`, `ResolveResult` | `applications`, `install_sources` | `POST /resolve` | Domain Rule 1 |
| FR-5 | `InstallSource`, `Evidence`, `ConfidenceLevel` | `install_sources` | `POST /resolve` | — |
| FR-6 | `Application` (multi-ref) | `package_instances` (FK) | `POST /resolve` | D3 |
| FR-7 | `ResolveResult` (`disambiguation-required`), `DisambiguateRequest` | `applications.disambiguated` | `POST /resolve/disambiguate` | D3 |
| FR-8 | `ResidueGraph` | `scans`, `artifacts` | `POST /scans`, `GET /scans/{id}/{v}` | immutability §6.2 |
| FR-9 | `ArtifactCategory` | `artifacts.category` (+CHECK) | `POST /scans` | — |
| FR-10 | `Artifact.ownerSet` | `artifacts.owner_set` | — | §8.1 (D2 blend) |
| FR-11 | `UsageKind` | `artifacts.usage_kind` | — | pure-function §3.4 |
| FR-12 | `DiscoverySource`, `ConfidenceLevel` | `artifacts.discovered_by` | `POST /scans` | D4 |
| FR-13 | `Evidence` kinds (appimage/pip/npm/oci) | `install_sources` | `POST /scans` | D9 |
| FR-14 | `RemovalPlan`, `PlanStatus` | `plans`, `plan_operations` | `POST /plans`, `GET /plans/{id}` | immutability §6.2 |
| FR-15 | `RemovalOperation.impact` | `plan_operations.impact` | `POST /plans` | D2 |
| FR-16 | `SafetyVerdict` | `plan_operations.verdict` | `POST /plans` | verdict rules §3.4 |
| FR-17 | `RemovalOperation.order`, `Action` | `plan_operations.order` | `POST /plans` | ordering §3.4 |
| FR-18 | `ApproveRequest`, `accepted_risks` | `accepted_risks` | `POST /plans/{id}/approve` | D14 |
| FR-19 | `RemovalPlan` (`draft`) | `plans` | `POST /plans` (Dry Run) | D12 |
| FR-20 | `Application.isProtected`, blocklist | `applications.is_protected` | approve/scan `409 PROTECTED_APP` | D7, §5.2 |
| FR-21 | `RemovalMode` | `plans.mode` | `POST /plans` | mode rules §3.4 |
| FR-22 | `RemovalScope`, `ScopeTag` | `plans.scope`, `artifacts.scope` | `POST /scans`, `POST /plans` | D8 |
| FR-23 | `Snapshot`, `CreateJobRequest` | `snapshots`, `snapshot_entries` | `POST /jobs` | D6/D11, §5.3 |
| FR-24 | `RemovalJob`, `JobStatus` | `jobs` | `POST /jobs`, `POST /jobs/{id}/begin` | guard §4.1 |
| FR-25 | `RemovalJob` (`failed→rolled_back`) | `jobs` | `GET /jobs/{id}` | §4.1 |
| FR-26 | `JobStatus`, `ExecutedStep` | `jobs`, `executed_steps` | begin/GET job | state machine §4.1 |
| FR-27 | `SweepResult` | — | `POST /jobs/{id}/sweep` | — |
| FR-28 | `RuntimeWarning` | — | `POST /scans` (`runtimeWarnings`) | D10 |
| FR-29 | `ExecutedStep`, `ErrorDetail` | `executed_steps` | `GET /jobs/{id}` | — |
| FR-30 | `PackageBackend`, `PackageInstance` | — | `GET /backends` | §8 ACL |
| FR-31 | `Action`, `CommandResult` | `plan_operations.action` | — | §8 commands |
| FR-32 | — | — | — | §9 privilege (D5) |
| FR-33 | `AuditRecord` | `audit_records` (append-only) | `GET /audit/{id}` | §6.2 |
| FR-34 | `UndoResult` | `snapshot_entries`, `audit_records` | `POST /audit/{id}/undo` | D6 |
| FR-35 | `HistoryItemDTO` | `audit_records` | `GET /history` | — |
| FR-36 | all DTOs | all tables | all endpoints | D13, §7.1 |
| NFR-1..NFR-15 | — | — | — | §10 (per-NFR mechanisms) |

**Decisions coverage:** D1 §5.1 · D2 §8.1 · D3 §3.2/§7.6-B · D4 §3.3 · D5 §9 · D6 §3.5 · D7 §5.2 ·
D8 §3.1/§3.4 · D9 §3.2/§8 · D10 §3.3/§4 · D11 §5.3 · D12 §7.5 (`POST /plans`) · D13 §1.1/§7.1 · D14 §3.4/§7.4.

**Domain Rules coverage (DOMAIN §5):** 1 Resolve-before-remove → §3.2/FR-4 · 2 Owner-aware → §3.3/§8.1 ·
3 Impact-gated → §3.4/§7.5 approve · 4 Protected Apps → §5.2 · 5 Snapshot-before-execute → §3.5/§4.1 ·
6 Fail-safe ordering → §4.1/§3.4.
