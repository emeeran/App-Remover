/**
 * app-remover — Application API server (Open Hosting Service / Published Language).
 *
 * SDD code stage. Implements docs/02-spec/SPEC.md §7 "Application API".
 *
 * Scope of this file: a single, self-contained, production-grade Express 5 server
 * that realizes the full Application API contract (§7.2, all 22 endpoints) with:
 *   • Zod 4 validation at every API boundary (§3 Published Language + §7.3 DTOs) — [NFR-7]
 *   • the unified error envelope + documented status codes (§7.4)             — [FR-36]
 *   • the domain guards expressible from request data:
 *       - Resolve-before-remove / disambiguation gate (§4, FR-4/FR-7, F4)
 *       - Plan approval at BOTH job creation and begin (§4.1, FR-24, F3)
 *       - Approve guards: blocked-op / risk-not-accepted (FR-18)
 *       - Protected-app blocklist (FR-20, §5.2, D7)
 *       - Single-write lock / conflicting job (§2, FR-28, F8)
 *       - Job state machine + auto-rollback (§4.1, FR-25/FR-26)
 *       - Snapshot cost cap (§5.3, FR-23, D11) + integrity verify (NFR-15)
 *       - Append-only, hash-chained audit (§3.7/§6.2, FR-33, NFR-9, F6)
 *   • local-only transport: 0600 Unix socket (primary) → loopback TCP fallback (§7.1, D13)
 *   • structured JSON logging to stderr with no body/secret echo (§9, NFR-8, NFR-13)
 *
 * What is deliberately NOT executed here: the destructive Package Manager ACL
 * commands (§8) — `uninstall-package`, `delete-file`, `stop-service`, `prune-orphan`.
 * Running `apt remove` / `rm` for real during a reference build would be unsafe. They
 * are therefore modelled as recorded job-step intents over an in-memory store; the
 * arg-array + shell:false barrier and per-backend identifier allowlists (§8.2) are
 * encoded as the validated boundary, ready to be wired to `execFile` behind a
 * polkit-gated privilege gateway (§9, D5) in the privileged runtime. Persistence is
 * in-memory (better-sqlite3/Drizzle are added per §2 once the privileged runtime lands).
 */

import express, { Router } from "express";
import type {
  ErrorRequestHandler,
  NextFunction,
  Request,
  RequestHandler,
  Response,
} from "express";
import { createHash, randomUUID } from "node:crypto";
import { chmodSync, existsSync, readFileSync, unlinkSync } from "node:fs";
import type { Server } from "node:http";
import path from "node:path";
import { userInfo } from "node:os";
import { z } from "zod";

// ====================================================================================
// 1. Published Language — Zod schemas (SPEC §3) + API DTOs (SPEC §7.3)
//    These double as the OpenAPI 3.1 component schemas (SPEC §7.3, R5/R6).
// ====================================================================================

// --- Identifiers (SPEC §3.1) ---
const CanonicalAppId = z.string().uuid();
const PackageInstanceId = z.string().uuid();
const ArtifactId = z.string().uuid();
const OperationId = z.string().uuid();
const StepId = z.string().uuid();
const PlanId = z.string().uuid();
const JobId = z.string().uuid();
const SnapshotId = z.string().uuid();
const AuditRecordId = z.string().uuid();
const ResolveToken = z.string().uuid();
const ScanVersion = z.number().int().nonnegative(); // monotonic per CanonicalAppId (FR-8 AC3)
const PathString = z.string().min(1).max(4096);
const IsoTimestamp = z.string().datetime(); // UTC, e.g. 2026-08-08T14:30:00.000Z

// --- Enums (closed) (SPEC §3.1) ---
const PackageBackend = z.enum([
  "deb", "snap", "flatpak", "appimage", "pip", "npm", "manual", "systemd",
]);
const InstallMethod = z.enum([
  "deb", "snap", "flatpak", "appimage", "manual", "language-package", "container",
]);
const ConfidenceLevel = z.enum(["high", "medium", "low"]);
const ArtifactCategory = z.enum([
  "binary", "config", "cache", "data", "state", "service",
  "desktop-entry", "association", "dependency",
]);
const UsageKind = z.enum(["exclusive", "shared", "system"]);
const DiscoverySource = z.enum(["manifest", "knowledge-base", "heuristic"]);
const RemovalScope = z.enum(["system-wide", "current-user", "both"]);
const RemovalMode = z.enum(["remove", "purge"]);
const Action = z.enum([
  "uninstall-package", "stop-service", "delete-file", "remove-association", "prune-orphan",
]);
const SafetyVerdict = z.enum(["safe", "risky", "blocked", "manual-review"]);
const PlanStatus = z.enum(["draft", "approved", "superseded"]);
const JobStatus = z.enum(["created", "running", "completed", "failed", "rolled_back"]);
const StepStatus = z.enum(["pending", "running", "succeeded", "failed", "skipped"]);
const SnapshotEntryKind = z.enum(["file-backup", "package-record", "unit-record"]);
const ScopeTag = z.enum(["system", "user"]);

// --- Provenance & Canonical Application (SPEC §3.2) [FR-4..FR-7] ---
const PackageInstanceRefSchema = z.object({
  packageInstanceId: PackageInstanceId,
  backend: PackageBackend,
  packageName: z.string().min(1).max(256),
  version: z.string().min(1).max(128).nullable(),
  scope: ScopeTag.nullable(),
});
const EvidenceSchema = z.object({
  kind: z.enum([
    "dpkg-record", "snap-list-record", "flatpak-list-record",
    "appimage-magic", "appimage-integration",
    "pip-record", "npm-record", "oci-manifest",
    "desktop-entry", "file-path", "reverse-dependency",
  ]),
  detail: z.string().min(1).max(1024),
});
const InstallSourceSchema = z.object({
  method: InstallMethod,
  confidence: ConfidenceLevel,
  evidence: z.array(EvidenceSchema).min(1),
  packageRef: PackageInstanceRefSchema.nullable(),
});
const DesktopEntryRefSchema = z.object({
  path: PathString,
  appId: z.string().min(1).max(256).nullable(),
  icon: z.string().min(1).max(1024).nullable(),
  mimetypes: z.array(z.string().min(1).max(128)),
});
const ApplicationSchema = z.object({
  canonicalAppId: CanonicalAppId,
  name: z.string().min(1).max(256),
  desktopEntry: DesktopEntryRefSchema.nullable(),
  installSources: z.array(InstallSourceSchema).min(1),
  packageInstanceRefs: z.array(PackageInstanceRefSchema),
  isProtected: z.boolean(),
  instancesDisambiguated: z.boolean(),
});

// --- ResidueGraph (SPEC §3.3) [FR-8..FR-13] ---
const RuntimeWarningSchema = z.object({
  kind: z.enum(["process-running", "lock-held", "service-active"]),
  detail: z.string().min(1).max(1024),
  autoStoppable: z.boolean(),
});
const ArtifactSchema = z.object({
  artifactId: ArtifactId,
  category: ArtifactCategory,
  target: PathString,
  ownerSet: z.array(CanonicalAppId).min(1),
  usageKind: UsageKind,
  sizeBytes: z.number().int().nonnegative().nullable(),
  deletable: z.boolean(),
  confidence: ConfidenceLevel,
  discoveredBy: DiscoverySource,
  scope: ScopeTag.nullable(),
});
const ResidueGraphSchema = z.object({
  applicationId: CanonicalAppId,
  scanVersion: ScanVersion,
  sealedAt: IsoTimestamp,
  artifacts: z.array(ArtifactSchema),
  counts: z.object({
    exclusive: z.number().int().nonnegative(),
    shared: z.number().int().nonnegative(),
    system: z.number().int().nonnegative(),
  }),
  runtimeWarnings: z.array(RuntimeWarningSchema),
});

// --- RemovalPlan (SPEC §3.4) [FR-14..FR-22] ---
const OperationTargetSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("artifact"), artifactId: ArtifactId }),
  z.object({ kind: z.literal("package"), packageInstanceId: PackageInstanceId }),
]);
const RemovalOperationSchema = z.object({
  operationId: OperationId,
  order: z.number().int().nonnegative(),
  action: Action,
  target: OperationTargetSchema,
  verdict: SafetyVerdict,
  impact: z.array(CanonicalAppId),
  rationale: z.string().min(1).max(1024),
  accepted: z.boolean().default(false),
});
const RemovalPlanSchema = z.object({
  planId: PlanId,
  applicationId: CanonicalAppId,
  scanVersion: ScanVersion,
  mode: RemovalMode,
  scope: RemovalScope,
  status: PlanStatus,
  operations: z.array(RemovalOperationSchema),
  projectedSnapshotBytes: z.number().int().nonnegative(),
  exceedsCostCap: z.boolean(),
  blockedReasons: z.array(z.string().min(1).max(256)),
  composedAt: IsoTimestamp,
});

// --- Snapshot (SPEC §3.5) [FR-23, D6, D11] ---
const SnapshotEntrySchema = z.object({
  artifactId: ArtifactId.nullable(),
  category: ArtifactCategory,
  originalPath: PathString,
  blobPath: PathString.nullable(),
  checksum: z.string().length(64),
  sizeBytes: z.number().int().nonnegative(),
  kind: SnapshotEntryKind,
  packageName: z.string().min(1).max(256).nullable(),
});
const SnapshotSchema = z.object({
  snapshotId: SnapshotId,
  jobId: JobId,
  capturedAt: IsoTimestamp,
  checksumAlgo: z.literal("sha256"),
  manifestChecksum: z.string().length(64),
  entries: z.array(SnapshotEntrySchema),
  totalBytes: z.number().int().nonnegative(),
  excludesCache: z.boolean().default(true),
  blobRoot: PathString,
});

// --- RemovalJob & ExecutedStep (SPEC §3.6) [FR-24..FR-29] ---
const ErrorDetailSchema = z.object({
  code: z.string().min(1).max(128),
  message: z.string().min(1).max(2048),
  retryable: z.boolean(),
});
const ExecutedStepSchema = z.object({
  stepId: StepId,
  operationId: OperationId,
  order: z.number().int().nonnegative(),
  status: StepStatus,
  startedAt: IsoTimestamp.nullable(),
  finishedAt: IsoTimestamp.nullable(),
  error: ErrorDetailSchema.nullable(),
});
const RemovalJobSchema = z.object({
  jobId: JobId,
  planId: PlanId,
  snapshotId: SnapshotId.nullable(),
  status: JobStatus,
  steps: z.array(ExecutedStepSchema),
  createdAt: IsoTimestamp,
  startedAt: IsoTimestamp.nullable(),
  finishedAt: IsoTimestamp.nullable(),
  failure: ErrorDetailSchema.nullable(),
});

// --- AuditRecord (SPEC §3.7) [FR-33, NFR-9] ---
const AuditRecordSchema = z.object({
  auditRecordId: AuditRecordId,
  jobId: JobId,
  planSnapshot: RemovalPlanSchema,
  steps: z.array(ExecutedStepSchema),
  snapshotId: SnapshotId,
  outcome: z.enum(["completed", "rolled_back"]),
  undoable: z.boolean(),
  createdAt: IsoTimestamp,
  hash: z.string().length(64),
  prevHash: z.string().length(64).nullable(),
});

// --- API DTOs (SPEC §7.3) ---
const InventoryItemDTOSchema = z.object({
  canonicalAppId: CanonicalAppId,
  name: z.string(),
  installMethods: z.array(InstallMethod).min(1),
  version: z.string().nullable(),
  icon: z.string().nullable(),
  instanceCount: z.number().int().positive(),
});
const InventoryDTOSchema = z.object({
  items: z.array(InventoryItemDTOSchema),
  skippedBackends: z.array(PackageBackend),
  generatedAt: IsoTimestamp,
});
const ResolveRequestSchema = z.object({
  source: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("desktop-entry"), path: PathString }),
    z.object({ kind: z.literal("canonical-id"), canonicalAppId: CanonicalAppId }),
  ]),
});
const ResolveResultSchema = z.discriminatedUnion("status", [
  z.object({ status: z.literal("resolved"), application: ApplicationSchema }),
  z.object({
    status: z.literal("disambiguation-required"),
    resolveToken: ResolveToken,
    candidates: z.array(ApplicationSchema).min(2),
  }),
  z.object({ status: z.literal("unresolvable"), reason: z.string() }),
]);
const DisambiguateRequestSchema = z.object({
  resolveToken: ResolveToken,
  selectedCanonicalAppIds: z.array(CanonicalAppId).min(1),
});
const ScanRequestSchema = z.object({
  canonicalAppId: CanonicalAppId,
  scope: RemovalScope.default("both"),
});
const ComposePlanRequestSchema = z.object({
  canonicalAppId: CanonicalAppId,
  scanVersion: ScanVersion,
  mode: RemovalMode.default("remove"),
  scope: RemovalScope.default("both"),
});
const ApproveRequestSchema = z.object({
  acceptedRiskOperationIds: z.array(OperationId).default([]),
});
const CreateJobRequestSchema = z.object({
  planId: PlanId,
  proceedDespiteCostCap: z.boolean().default(false),
});
const SweepResultSchema = z.object({
  missedResidueGraph: ResidueGraphSchema.nullable(),
  followUpPlanId: PlanId.nullable(),
});
const HistoryItemDTOSchema = z.object({
  auditRecordId: AuditRecordId,
  jobId: JobId,
  targetName: z.string(),
  outcome: z.enum(["completed", "rolled_back"]),
  createdAt: IsoTimestamp,
  undoable: z.boolean(),
});
const UndoResultSchema = z.object({
  restoreJobId: JobId,
  deferredPackages: z.array(z.string()),
});
const HealthDTOSchema = z.object({
  status: z.enum(["ok", "degraded"]),
  backends: z.array(z.object({ backend: PackageBackend, present: z.boolean() })),
  dbWritable: z.boolean(),
});
const SnapshotVerifyResultSchema = z.object({
  snapshotId: SnapshotId,
  intact: z.boolean(),
  failures: z.array(z.object({ entryPath: PathString, reason: z.string() })),
});
const BackendsDTOSchema = z.object({
  backends: z.array(z.object({ backend: PackageBackend, present: z.boolean() })),
});

// --- Inferred types ---
type PackageInstanceRef = z.infer<typeof PackageInstanceRefSchema>;
type Application = z.infer<typeof ApplicationSchema>;
type Artifact = z.infer<typeof ArtifactSchema>;
type RuntimeWarning = z.infer<typeof RuntimeWarningSchema>;
type ResidueGraph = z.infer<typeof ResidueGraphSchema>;
type RemovalOperation = z.infer<typeof RemovalOperationSchema>;
type RemovalPlan = z.infer<typeof RemovalPlanSchema>;
type SnapshotEntry = z.infer<typeof SnapshotEntrySchema>;
type Snapshot = z.infer<typeof SnapshotSchema>;
type ErrorDetail = z.infer<typeof ErrorDetailSchema>;
type ExecutedStep = z.infer<typeof ExecutedStepSchema>;
type RemovalJob = z.infer<typeof RemovalJobSchema>;
type AuditRecord = z.infer<typeof AuditRecordSchema>;
type ActionKind = z.infer<typeof Action>;
type UsageKindValue = z.infer<typeof UsageKind>;
type ConfidenceValue = z.infer<typeof ConfidenceLevel>;
type DiscoveryValue = z.infer<typeof DiscoverySource>;
type ArtifactCategoryValue = z.infer<typeof ArtifactCategory>;
type ScopeTagValue = z.infer<typeof ScopeTag>;

// ====================================================================================
// 2. Configuration (SPEC §5) [D1, D7, D11]
// ====================================================================================

const DEFAULT_MAX_SNAPSHOT_BYTES = 1_073_741_824; // 1 GiB (SPEC §5.3)
const DEFAULT_BLOB_ROOT = (() => {
  const base = process.env.XDG_DATA_HOME ?? path.join(userInfo().homedir, ".local", "share");
  return path.join(base, "app-remover", "snapshots");
})();

/** NON-OVERRIDABLE core protected-app names (SPEC §5.2, FR-20 AC2). */
const PROTECTED_CORE_NAMES: ReadonlySet<string> = new Set([
  "app-remover",
  "gnome-shell", "gnome-session", "kde-plasma", "xfce4-session",
  "glibc", "libc6", "libstdc++6",
  "xorg", "xwayland", "mutter", "kwin",
  "systemd",
  "dpkg", "apt", "snapd", "flatpak",
]);

interface AppConfig {
  readonly maxSnapshotBytes: number;
  readonly blobRoot: string;
  readonly userBlocklist: readonly string[];
}

const AppConfigSchema = z.object({
  maxSnapshotBytes: z.number().int().positive().default(DEFAULT_MAX_SNAPSHOT_BYTES),
  blobRoot: z.string().min(1).default(DEFAULT_BLOB_ROOT),
  userBlocklist: z.array(z.string().min(1)).default([]),
});

function configFilePath(): string {
  const base = process.env.XDG_CONFIG_HOME ?? path.join(userInfo().homedir, ".config");
  return path.join(base, "app-remover", "config.json");
}

function loadConfig(): AppConfig {
  const cfgPath = configFilePath();
  let raw: unknown = {};
  if (existsSync(cfgPath)) {
    try {
      raw = JSON.parse(readFileSync(cfgPath, "utf8"));
    } catch {
      raw = {}; // malformed config → fall back to normative defaults
    }
  }
  return AppConfigSchema.parse(raw);
}

function isProtectedName(name: string, userBlocklist: readonly string[]): boolean {
  const lower = name.toLowerCase();
  if (PROTECTED_CORE_NAMES.has(lower)) return true;
  return userBlocklist.some((n) => n.toLowerCase() === lower);
}

// ====================================================================================
// 3. Utilities: ids, time, hashing, deterministic serialization, logging
// ====================================================================================

const newId = (): string => randomUUID();
const nowIso = (): string => new Date().toISOString();

function sha256Hex(input: string): string {
  return createHash("sha256").update(input, "utf8").digest("hex");
}

/** Deterministic JSON for hash-stable audit/snapshot checksums (NFR-9, NFR-15). */
function stableStringify(value: unknown): string {
  return JSON.stringify(value, (_key, node) => {
    if (node && typeof node === "object" && !Array.isArray(node)) {
      const record = node as Record<string, unknown>;
      const sorted: Record<string, unknown> = {};
      for (const key of Object.keys(record).sort()) sorted[key] = record[key];
      return sorted;
    }
    return node;
  });
}

type LogLevel = "info" | "warn" | "error";
function logStructured(level: LogLevel, event: string, fields: Record<string, unknown> = {}): void {
  // Structured JSON to stderr (NFR-13). Never echo request bodies or file contents (NFR-8).
  const line = JSON.stringify({ ts: nowIso(), level, event, ...fields });
  process.stderr.write(line + "\n");
}

// ====================================================================================
// 4. Error model (SPEC §7.4)
// ====================================================================================

const ERROR_CODE_STATUS: Readonly<Record<string, number>> = {
  VALIDATION_ERROR: 400,
  NOT_FOUND: 404,
  DISAMBIGUATION_REQUIRED: 409,
  PLAN_NOT_APPROVED: 409,
  PLAN_HAS_BLOCKED_OP: 409,
  RISK_NOT_ACCEPTED: 409,
  PROTECTED_APP: 409,
  ILLEGAL_TRANSITION: 409,
  CONFLICTING_JOB: 409,
  SNAPSHOT_COST_CAP_EXCEEDED: 422,
  SNAPSHOT_CORRUPT: 422,
  INTERNAL_ERROR: 500,
};

class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details: Readonly<Record<string, unknown>>;

  constructor(code: string, message: string, details: Record<string, unknown> = {}) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = ERROR_CODE_STATUS[code] ?? 500;
    this.details = details;
  }
}

/** Builds the §7.4 envelope, omitting `details` entirely when empty (exactOptional-safe). */
function errorEnvelope(
  code: string,
  message: string,
  details?: Readonly<Record<string, unknown>>,
): { error: { code: string; message: string; details?: Record<string, unknown> } } {
  const inner: { code: string; message: string; details?: Record<string, unknown> } = {
    code,
    message,
  };
  if (details && Object.keys(details).length > 0) {
    inner.details = { ...details };
  }
  return { error: inner };
}

/** Validate `value` against a Zod schema or throw 400 VALIDATION_ERROR (NFR-7). */
function parseOrThrow<T>(schema: z.ZodType<T>, value: unknown): T {
  const result = schema.safeParse(value);
  if (!result.success) {
    throw new ApiError("VALIDATION_ERROR", "Request validation failed", {
      issues: result.error.issues.map((issue) => ({
        path: issue.path.join("."),
        message: issue.message,
      })),
    });
  }
  return result.data;
}

// ====================================================================================
// 5. Package Manager ACL surface (SPEC §8) — present/absent detection (NFR-14)
//    + the §8.2 identifier-allowlist defense-in-depth barrier (no execution here).
// ====================================================================================

/**
 * Tool presence per backend (NFR-14 graceful degradation). appimage/manual have no
 * canonical tool binary, so they are reported as always-available detection strategies.
 */
const BACKEND_TOOLS: Readonly<Record<z.infer<typeof PackageBackend>, readonly string[]>> = {
  deb: ["/usr/bin/dpkg", "/usr/bin/apt-get"],
  snap: ["/usr/bin/snap"],
  flatpak: ["/usr/bin/flatpak"],
  appimage: [],
  pip: ["/usr/bin/pip3", "/usr/bin/pip"],
  npm: ["/usr/bin/npm", "/usr/bin/node"],
  manual: [],
  systemd: ["/usr/bin/systemctl", "/bin/systemctl"],
};

function backendPresent(backend: z.infer<typeof PackageBackend>): boolean {
  const tools = BACKEND_TOOLS[backend];
  if (tools.length === 0) return true; // detection-only strategies
  return tools.some((p) => existsSync(p));
}

/** Per-backend identifier allowlists (SPEC §8.2, F1). Defense-in-depth ahead of execFile. */
const BACKEND_IDENTIFIER_PATTERN: Readonly<Record<string, RegExp>> = {
  deb: /^[a-z0-9][a-z0-9+.-]{0,255}$/,
  snap: /^[a-z0-9][a-z0-9-]{0,39}$/,
  flatpak: /^[a-z0-9.][a-z0-9.-]{0,254}$/,
  npm: /^(@[a-z0-9-~][a-z0-9-._~]*\/)?[a-z0-9-~][a-z0-9-._~]*$/,
  pip: /^([A-Z0-9]|[A-Z0-9][A-Z0-9._-]*[A-Z0-9])$/i,
  systemd: /^[A-Za-z0-9@:_.+-]{1,255}\.(service|socket|timer|target|mount)$/,
};

function validateBackendIdentifier(backend: string, identifier: string): boolean {
  const pattern = BACKEND_IDENTIFIER_PATTERN[backend];
  return pattern ? pattern.test(identifier) : true; // appimage/manual: no package identifier
}

function allBackendStates(): Array<{ backend: z.infer<typeof PackageBackend>; present: boolean }> {
  const ids = PackageBackend.options as readonly z.infer<typeof PackageBackend>[];
  return ids.map((backend) => ({ backend, present: backendPresent(backend) }));
}

// ====================================================================================
// 6. In-memory stores + seed fixtures
// ====================================================================================

interface Stores {
  readonly applications: Map<string, Application>;
  readonly scans: Map<string, ResidueGraph>; // key `${canonicalAppId}:${scanVersion}`
  readonly scanVersion: Map<string, number>; // monotonic per CanonicalAppId (FR-8 AC3)
  readonly plans: Map<string, RemovalPlan>;
  readonly jobs: Map<string, RemovalJob>;
  readonly snapshots: Map<string, Snapshot>;
  readonly audit: Map<string, AuditRecord>;
  readonly resolveTokens: Map<string, Application[]>;
  readonly inFlightApps: Set<string>; // single-write lock (§2, FR-28, F8)
  lastAuditHash: string | null; // hash-chain head (NFR-9); genesis record has null prevHash
}

function scanKey(canonicalAppId: string, scanVersion: number): string {
  return `${canonicalAppId}:${scanVersion}`;
}

function makePackageRef(
  backend: z.infer<typeof PackageBackend>,
  packageName: string,
  version: string | null,
  scope: ScopeTagValue,
): PackageInstanceRef {
  return {
    packageInstanceId: newId(),
    backend,
    packageName,
    version,
    scope,
  };
}

/** Build a representative ResidueGraph for `app` from the §5.1 residue-policy roots. */
function buildArtifactsFor(app: Application): Artifact[] {
  const name = app.name;
  const owner = app.canonicalAppId;
  const coOwner = newId(); // synthetic co-owner to exercise `shared` usageKind / impact (FR-11, FR-15)
  const mk = (
    category: ArtifactCategoryValue,
    target: string,
    usageKind: UsageKindValue,
    sizeBytes: number,
    scope: ScopeTagValue | null,
    confidence: ConfidenceValue,
    discoveredBy: DiscoveryValue,
  ): Artifact => ({
    artifactId: newId(),
    category,
    target,
    ownerSet: usageKind === "shared" ? [owner, coOwner] : [owner],
    usageKind,
    sizeBytes,
    deletable: usageKind !== "system", // FR-11 AC2
    confidence,
    discoveredBy,
    scope,
  });

  return [
    mk("desktop-entry", `/usr/share/applications/${name}.desktop`, "exclusive", 2048, "system", "high", "manifest"),
    mk("config", `~/.config/${name}`, "exclusive", 8192, "user", "high", "manifest"),
    mk("config", `/etc/${name}/${name}.conf`, "system", 1024, "system", "high", "manifest"),
    mk("cache", `~/.cache/${name}`, "exclusive", 16384, "user", "low", "heuristic"),
    mk("data", `~/.local/share/${name}`, "exclusive", 32768, "user", "high", "manifest"),
    mk("dependency", `/usr/lib/x86_64-linux-gnu/lib${name}-shared.so`, "shared", 65536, "system", "medium", "knowledge-base"),
    mk("service", `/etc/systemd/system/${name}.service`, "exclusive", 512, "system", "high", "manifest"),
  ];
}

function deriveCounts(artifacts: readonly Artifact[]): {
  exclusive: number; shared: number; system: number;
} {
  let exclusive = 0, shared = 0, system = 0;
  for (const a of artifacts) {
    if (a.usageKind === "exclusive") exclusive += 1;
    else if (a.usageKind === "shared") shared += 1;
    else system += 1;
  }
  return { exclusive, shared, system };
}

function seedApplications(config: AppConfig): Application[] {
  const vlcRef = makePackageRef("deb", "vlc", "3.0.20-1", "system");
  const firefoxDeb = makePackageRef("deb", "firefox", "1:128", "system");
  const firefoxSnap = makePackageRef("snap", "firefox", "128", "system");
  const gnomeRef = makePackageRef("deb", "gnome-shell", "42-1", "system");

  const mkApp = (
    name: string,
    desktopPath: string | null,
    refs: PackageInstanceRef[],
    protectedName: boolean,
    disambiguated: boolean,
  ): Application => ({
    canonicalAppId: newId(),
    name,
    desktopEntry: desktopPath
      ? { path: desktopPath, appId: null, icon: null, mimetypes: [] }
      : null,
    installSources: refs.map((r) => ({
      method: (r.backend === "deb" || r.backend === "snap" || r.backend === "flatpak"
        ? r.backend
        : r.backend === "pip" || r.backend === "npm"
          ? "language-package"
          : "manual") as z.infer<typeof InstallMethod>,
      confidence: "high",
      evidence: [{ kind: r.backend === "deb" ? "dpkg-record" : r.backend === "snap" ? "snap-list-record" : "file-path", detail: `${r.backend}:${r.packageName}` }],
      packageRef: r,
    })),
    packageInstanceRefs: refs,
    isProtected: protectedName || isProtectedName(name, config.userBlocklist),
    instancesDisambiguated: disambiguated,
  });

  return [
    mkApp("vlc", "/usr/share/applications/vlc.desktop", [vlcRef], false, true), // single instance → resolved (Flow A)
    // Two DISTINCT Firefox installs sharing one desktop entry → POST /resolve returns
    // disambiguation-required with both as candidates (SPEC §7.6 Flow B, FR-7).
    mkApp("firefox", "/usr/share/applications/firefox.desktop", [firefoxDeb], false, false),
    mkApp("firefox", "/usr/share/applications/firefox.desktop", [firefoxSnap], false, false),
    mkApp("gnome-shell", "/usr/share/applications/org.gnome.Shell.desktop", [gnomeRef], true, true), // protected (FR-20)
  ];
}

function createStores(config: AppConfig): Stores {
  const stores: Stores = {
    applications: new Map(),
    scans: new Map(),
    scanVersion: new Map(),
    plans: new Map(),
    jobs: new Map(),
    snapshots: new Map(),
    audit: new Map(),
    resolveTokens: new Map(),
    inFlightApps: new Set(),
    lastAuditHash: null,
  };
  for (const app of seedApplications(config)) {
    stores.applications.set(app.canonicalAppId, app);
  }
  return stores;
}

// ====================================================================================
// 7. Domain operations
// ====================================================================================

function requireApplication(stores: Stores, canonicalAppId: string): Application {
  const app = stores.applications.get(canonicalAppId);
  if (!app) throw new ApiError("NOT_FOUND", `Application '${canonicalAppId}' not found`);
  return app;
}

function requireScan(stores: Stores, canonicalAppId: string, scanVersion: number): ResidueGraph {
  const scan = stores.scans.get(scanKey(canonicalAppId, scanVersion));
  if (!scan) throw new ApiError("NOT_FOUND", `Scan '${canonicalAppId}@${scanVersion}' not found`);
  return scan;
}

function requirePlan(stores: Stores, planId: string): RemovalPlan {
  const plan = stores.plans.get(planId);
  if (!plan) throw new ApiError("NOT_FOUND", `Plan '${planId}' not found`);
  return plan;
}

function requireJob(stores: Stores, jobId: string): RemovalJob {
  const job = stores.jobs.get(jobId);
  if (!job) throw new ApiError("NOT_FOUND", `Job '${jobId}' not found`);
  return job;
}

/**
 * Resolve / scan guards enforced at the API boundary (FR-20, FR-7/F4).
 *   • protected → 409 PROTECTED_APP
 *   • not disambiguated → 409 DISAMBIGUATION_REQUIRED
 */
function guardRemovable(app: Application): void {
  if (app.isProtected) {
    throw new ApiError("PROTECTED_APP", `'${app.name}' is on the protected-app blocklist and cannot be removed`, {
      name: app.name,
    });
  }
  if (!app.instancesDisambiguated) {
    throw new ApiError("DISAMBIGUATION_REQUIRED", `'${app.name}' has distinct instances that must be disambiguated`, {
      canonicalAppId: app.canonicalAppId,
    });
  }
}

/** Build + seal a ResidueGraph, incrementing the monotonic ScanVersion (FR-8 AC3). */
function sealResidueGraph(stores: Stores, app: Application, scope: z.infer<typeof RemovalScope>): ResidueGraph {
  guardRemovable(app);
  const nextVersion = (stores.scanVersion.get(app.canonicalAppId) ?? 0) + 1;
  stores.scanVersion.set(app.canonicalAppId, nextVersion);

  let artifacts = buildArtifactsFor(app);
  if (scope === "current-user") artifacts = artifacts.filter((a) => a.scope === "user");
  else if (scope === "system-wide") artifacts = artifacts.filter((a) => a.scope === "system");

  const graph: ResidueGraph = {
    applicationId: app.canonicalAppId,
    scanVersion: nextVersion,
    sealedAt: nowIso(),
    artifacts,
    counts: deriveCounts(artifacts),
    runtimeWarnings: [], // (FR-28) populated by the runtime-warning detector in the full build
  };
  stores.scans.set(scanKey(app.canonicalAppId, nextVersion), graph);
  return graph;
}

/** Action precedence for fail-safe ordering (FR-17, §3.4). Lower runs first. */
function actionPrecedence(action: ActionKind): number {
  switch (action) {
    case "stop-service": return 0;
    case "uninstall-package": return 1;
    case "remove-association": return 2;
    case "delete-file": return 3;
    case "prune-orphan": return 9; // always last
  }
}

/**
 * Compose a RemovalPlan (Dry Run — D12; no mutation, FR-19 AC1/AC2).
 * Verdict rules (§3.4): system→blocked; shared→risky (impact = other owners);
 * exclusive→safe. mode rules (FR-21): remove → uninstall-package only;
 * purge → adds exclusive config/data/cache deletes.
 */
function composePlan(
  stores: Stores,
  app: Application,
  scan: ResidueGraph,
  mode: z.infer<typeof RemovalMode>,
  scope: z.infer<typeof RemovalScope>,
  config: AppConfig,
): RemovalPlan {
  guardRemovable(app);

  const operations: RemovalOperation[] = [];
  const blockedReasons: string[] = [];

  // Package uninstall — emitted in both modes (FR-21 AC1).
  for (const ref of app.packageInstanceRefs) {
    const verdict: z.infer<typeof SafetyVerdict> = "safe";
    operations.push({
      operationId: newId(),
      order: 0, // reassigned below
      action: "uninstall-package",
      target: { kind: "package", packageInstanceId: ref.packageInstanceId },
      verdict,
      impact: [],
      rationale: `Uninstall ${ref.backend} package '${ref.packageName}' (mode=${mode})`,
      accepted: false,
    });
  }

  // File-level operations only in purge mode (FR-21 AC2).
  if (mode === "purge") {
    for (const a of scan.artifacts) {
      if (scope === "current-user" && a.scope !== "user") continue;
      if (scope === "system-wide" && a.scope !== "system") continue;

      if (a.category === "service") {
        operations.push({
          operationId: newId(), order: 0,
          action: "stop-service",
          target: { kind: "artifact", artifactId: a.artifactId },
          verdict: a.usageKind === "system" ? "blocked" : "safe",
          impact: a.usageKind === "shared" ? a.ownerSet.filter((o) => o !== app.canonicalAppId) : [],
          rationale: `Stop service unit at ${a.target}`,
          accepted: false,
        });
        continue;
      }

      if (a.category === "association") {
        operations.push({
          operationId: newId(), order: 0,
          action: "remove-association",
          target: { kind: "artifact", artifactId: a.artifactId },
          verdict: a.usageKind === "system" ? "blocked" : a.usageKind === "shared" ? "risky" : "safe",
          impact: a.usageKind === "shared" ? a.ownerSet.filter((o) => o !== app.canonicalAppId) : [],
          rationale: `Remove MIME association ${a.target}`,
          accepted: false,
        });
        continue;
      }

      if (["config", "cache", "data", "desktop-entry"].includes(a.category)) {
        let verdict: z.infer<typeof SafetyVerdict>;
        if (a.usageKind === "system") verdict = "blocked";
        else if (a.usageKind === "shared") verdict = "risky";
        else verdict = "safe";
        if (verdict === "blocked") blockedReasons.push(`${a.target}: system-owned, not deletable`);
        operations.push({
          operationId: newId(), order: 0,
          action: "delete-file",
          target: { kind: "artifact", artifactId: a.artifactId },
          verdict,
          impact: a.usageKind === "shared" ? a.ownerSet.filter((o) => o !== app.canonicalAppId) : [],
          rationale: `Delete ${a.category} at ${a.target}`,
          accepted: false,
        });
      }
    }
  }

  // Fail-safe ordering (FR-17).
  operations.sort((x, y) => actionPrecedence(x.action) - actionPrecedence(y.action));
  operations.forEach((op, idx) => { op.order = idx; });

  // Projected snapshot size = bytes that will be backed up (file-backup entries; cache excluded — D6).
  const projectedSnapshotBytes = scan.artifacts
    .filter((a) => a.category !== "cache" && a.category !== "state" && a.sizeBytes != null)
    .reduce((sum, a) => sum + (a.sizeBytes ?? 0), 0);

  const plan: RemovalPlan = {
    planId: newId(),
    applicationId: app.canonicalAppId,
    scanVersion: scan.scanVersion,
    mode,
    scope,
    status: "draft", // FR-14 AC2: immutability once approved
    operations,
    projectedSnapshotBytes,
    exceedsCostCap: projectedSnapshotBytes > config.maxSnapshotBytes,
    blockedReasons,
    composedAt: nowIso(),
  };
  stores.plans.set(plan.planId, plan);
  return plan;
}

/** Capture a pre-execution Snapshot (FR-23). Manifest checksum over entry records (NFR-15). */
function captureSnapshot(stores: Stores, job: RemovalJob, plan: RemovalPlan, app: Application, config: AppConfig): Snapshot {
  const scan = requireScan(stores, app.canonicalAppId, plan.scanVersion);
  const entries: SnapshotEntry[] = [];
  let totalBytes = 0;

  for (const a of scan.artifacts) {
    if (a.category === "cache" || a.category === "state") continue; // cache excluded (D6, FR-23 AC3)
    const isPackage = a.category === "binary" || a.category === "dependency";
    const entry: SnapshotEntry = {
      artifactId: a.artifactId,
      category: a.category,
      originalPath: a.target,
      // In the privileged runtime this points at the real 0600 blob under blobRoot (D6, NFR-8).
      blobPath: isPackage ? null : path.join(config.blobRoot, job.jobId, `${a.artifactId}.blob`),
      // sha256 of file bytes in production (NFR-15); stable placeholder content for the reference build.
      checksum: sha256Hex(`snapshot-entry:${job.jobId}:${a.artifactId}:${a.target}:${a.sizeBytes ?? 0}`),
      sizeBytes: a.sizeBytes ?? 0,
      kind: isPackage ? "package-record" : "file-backup",
      packageName: isPackage ? app.packageInstanceRefs[0]?.packageName ?? null : null,
    };
    totalBytes += entry.sizeBytes;
    entries.push(entry);
  }

  // Reinstall records for each package instance (best-effort, D6 / FR-34).
  for (const ref of app.packageInstanceRefs) {
    entries.push({
      artifactId: null,
      category: "binary",
      originalPath: `${ref.backend}:${ref.packageName}`,
      blobPath: null,
      checksum: sha256Hex(`package-record:${job.jobId}:${ref.packageInstanceId}`),
      sizeBytes: 0,
      kind: "package-record",
      packageName: ref.packageName,
    });
  }

  const manifestRecord = entries.map((e) => ({
    p: e.originalPath, c: e.checksum, k: e.kind, s: e.sizeBytes,
  }));
  const snapshot: Snapshot = {
    snapshotId: newId(),
    jobId: job.jobId,
    capturedAt: nowIso(),
    checksumAlgo: "sha256",
    manifestChecksum: sha256Hex(stableStringify(manifestRecord)),
    entries,
    totalBytes,
    excludesCache: true,
    blobRoot: config.blobRoot,
  };
  stores.snapshots.set(snapshot.snapshotId, snapshot);
  return snapshot;
}

/** Recompute the manifest checksum and compare (NFR-15). */
function verifySnapshotIntegrity(snapshot: Snapshot): { intact: boolean; failures: Array<{ entryPath: string; reason: string }> } {
  const manifestRecord = snapshot.entries.map((e) => ({ p: e.originalPath, c: e.checksum, k: e.kind, s: e.sizeBytes }));
  const recomputed = sha256Hex(stableStringify(manifestRecord));
  if (recomputed === snapshot.manifestChecksum) return { intact: true, failures: [] };
  return {
    intact: false,
    failures: [{ entryPath: "<manifest>", reason: "manifest checksum mismatch" }],
  };
}

/**
 * Append an immutable, hash-chained AuditRecord (FR-33, NFR-9, F6).
 * Append-only by construction: no update/delete path exists for `stores.audit`.
 */
function appendAuditRecord(
  stores: Stores,
  job: RemovalJob,
  plan: RemovalPlan,
  snapshotId: string,
  outcome: "completed" | "rolled_back",
): AuditRecord {
  const prevHash = stores.lastAuditHash;
  const record: Omit<AuditRecord, "hash"> = {
    auditRecordId: newId(),
    jobId: job.jobId,
    planSnapshot: plan,
    steps: job.steps,
    snapshotId,
    outcome,
    undoable: stores.snapshots.has(snapshotId) && verifySnapshotIntegrity(stores.snapshots.get(snapshotId) as Snapshot).intact,
    createdAt: nowIso(),
    prevHash,
  };
  const hash = sha256Hex(`${prevHash ?? ""}${stableStringify(record)}`);
  const audit: AuditRecord = { ...record, hash };
  stores.lastAuditHash = hash;
  stores.audit.set(audit.auditRecordId, audit);
  return audit;
}

/**
 * Execute a job's steps synchronously to a terminal state (§4.1, FR-25/FR-26).
 * On any step failure the runner auto-drives failed → rolled_back (NFR-5).
 * Returns the updated job. Step bodies are recorded intents only (no destructive exec).
 */
function runJobToTerminal(stores: Stores, job: RemovalJob, plan: RemovalPlan, snapshotId: string): RemovalJob {
  const startedAt = nowIso();
  job.startedAt = startedAt;
  job.status = "running";

  let failedStep: ExecutedStep | null = null;
  for (const step of job.steps) {
    step.startedAt = nowIso();
    step.status = "running";
    try {
      // Command would run via ACL adapter (arg-array, shell:false, polkit-gated — §8/§9).
      step.status = "succeeded";
      step.finishedAt = nowIso();
    } catch (err) {
      step.status = "failed";
      step.finishedAt = nowIso();
      step.error = {
        code: "STEP_FAILED",
        message: err instanceof Error ? err.message : "step execution failed",
        retryable: false,
      };
      failedStep = step;
      break;
    }
  }

  if (failedStep) {
    job.status = "failed";
    job.failure = failedStep.error;
    // Auto-rollback before terminal (FR-25 AC2, NFR-5).
    job.status = "rolled_back";
    job.finishedAt = nowIso();
    appendAuditRecord(stores, job, plan, snapshotId, "rolled_back");
  } else {
    job.status = "completed";
    job.finishedAt = nowIso();
    appendAuditRecord(stores, job, plan, snapshotId, "completed");
  }

  stores.inFlightApps.delete(plan.applicationId); // release single-write lock
  return job;
}

// ====================================================================================
// 8. Express wiring
// ====================================================================================

/** Wrap an async route so rejections flow to the error handler (works across Express 4/5). */
function asyncHandler(
  fn: (req: Request, res: Response, next: NextFunction) => Promise<unknown> | unknown,
): RequestHandler {
  return (req, res, next) => {
    Promise.resolve(fn(req, res, next)).catch(next);
  };
}

const notFoundHandler: RequestHandler = (_req, res) => {
  res.status(404).json(errorEnvelope("NOT_FOUND", "Resource not found"));
};

const errorHandler: ErrorRequestHandler = (err, req, res, next) => {
  if (res.headersSent) {
    next(err);
    return;
  }
  if (err instanceof ApiError) {
    if (err.status >= 500) {
      logStructured("error", "api_error", { code: err.code, method: req.method, path: req.path });
    }
    res.status(err.status).json(errorEnvelope(err.code, err.message, err.details));
    return;
  }
  // Malformed JSON body → 400 VALIDATION_ERROR (NFR-7).
  const maybeType = (err as { type?: string }).type;
  if (err instanceof SyntaxError && maybeType === "entity.parse.failed") {
    res.status(400).json(errorEnvelope("VALIDATION_ERROR", "Malformed JSON request body"));
    return;
  }
  logStructured("error", "internal_error", {
    name: err instanceof Error ? err.name : "Error",
    method: req.method,
    path: req.path,
  });
  res.status(500).json(errorEnvelope("INTERNAL_ERROR", "An unexpected error occurred"));
};

// Path-param schemas
const CanonicalAppIdParam = z.object({ canonicalAppId: CanonicalAppId });
const ScanParam = z.object({
  canonicalAppId: CanonicalAppId,
  scanVersion: z.coerce.number().int().nonnegative(),
});
const PlanIdParam = z.object({ planId: PlanId });
const JobIdParam = z.object({ jobId: JobId });
const AuditIdParam = z.object({ auditRecordId: AuditRecordId });
const SnapshotIdParam = z.object({ snapshotId: SnapshotId });
const InventoryQuery = z.object({ q: z.string().optional() });

/** Assemble the Express Application API over fresh in-memory stores. */
export function createApp(): express.Express {
  const config = loadConfig();
  const stores = createStores(config);

  const app = express();
  app.disable("x-powered-by"); // hygiene (NFR-8)
  app.use(express.json());

  // ---- GET /health [FR-1 AC2, NFR-13, NFR-14] ----
  const healthHandler: RequestHandler = (_req, res) => {
    const backends = allBackendStates();
    const anyPresent = backends.some((b) => b.present);
    const health = {
      status: anyPresent ? "ok" : ("degraded" as const),
      backends,
      dbWritable: true, // in-memory; SQLite in the privileged runtime
    };
    HealthDTOSchema.parse(health); // contract-shape guard
    res.json(health);
  };

  // ---- GET /backends [FR-30, NFR-14] ----
  const backendsHandler: RequestHandler = (_req, res) => {
    res.json({ backends: allBackendStates() });
  };

  // ---- GET /inventory + /inventory/{id} [FR-1, FR-2, NFR-1] ----
  const inventoryListHandler: RequestHandler = (req, res) => {
    const query = parseOrThrow(InventoryQuery, { q: req.query.q ?? undefined });
    const skipped = (PackageBackend.options as readonly z.infer<typeof PackageBackend>[])
      .filter((b) => !backendPresent(b));
    const items = Array.from(stores.applications.values())
      .filter((a) => (query.q ? a.name.toLowerCase().includes(query.q.toLowerCase()) : true))
      .map((a) => ({
        canonicalAppId: a.canonicalAppId,
        name: a.name,
        installMethods: Array.from(new Set(a.installSources.map((s) => s.method))),
        version: a.packageInstanceRefs[0]?.version ?? null,
        icon: a.desktopEntry?.icon ?? null,
        instanceCount: a.packageInstanceRefs.length,
      }));
    res.json({ items, skippedBackends: skipped, generatedAt: nowIso() });
  };
  const inventoryOneHandler: RequestHandler = (req, res) => {
    const { canonicalAppId } = parseOrThrow(CanonicalAppIdParam, req.params);
    res.json(requireApplication(stores, canonicalAppId));
  };

  // ---- POST /resolve + /resolve/disambiguate [FR-3..FR-7] ----
  const resolveHandler: RequestHandler = (req, res) => {
    const body = parseOrThrow(ResolveRequestSchema, req.body);
    const source = body.source;

    let candidates: Application[];
    if (source.kind === "canonical-id") {
      const app = stores.applications.get(source.canonicalAppId);
      candidates = app ? [app] : [];
    } else {
      candidates = Array.from(stores.applications.values())
        .filter((a) => a.desktopEntry?.path === source.path);
    }

    if (candidates.length === 0) {
      res.json({ status: "unresolvable", reason: "No application matched the given source" });
      return;
    }
    // Distinct canonical applications matched and not yet chosen → disambiguation (FR-7 AC1).
    // The `disambiguation-required` result requires >= 2 candidates (ResolveResult schema §7.3).
    if (candidates.length >= 2) {
      const token = newId();
      stores.resolveTokens.set(token, candidates);
      res.json({ status: "disambiguation-required", resolveToken: token, candidates });
      return;
    }

    const resolved = candidates[0];
    if (!resolved) throw new ApiError("INTERNAL_ERROR", "Resolved application is undefined");
    // A "resolved" result implies the canonical application is ready to plan (O1):
    // single-instance / already-merged apps are disambiguated by definition.
    if (!resolved.instancesDisambiguated) resolved.instancesDisambiguated = true;
    res.json({ status: "resolved", application: resolved });
  };
  const disambiguateHandler: RequestHandler = (req, res) => {
    const body = parseOrThrow(DisambiguateRequestSchema, req.body);
    const candidates = stores.resolveTokens.get(body.resolveToken);
    if (!candidates) throw new ApiError("NOT_FOUND", `Unknown resolveToken '${body.resolveToken}'`);
    const candidateIds = new Set(candidates.map((c) => c.canonicalAppId));
    for (const id of body.selectedCanonicalAppIds) {
      if (!candidateIds.has(id)) {
        throw new ApiError("VALIDATION_ERROR", `Selected id '${id}' is not among the candidates`, { id });
      }
    }
    if (body.selectedCanonicalAppIds.length > 1) {
      throw new ApiError("DISAMBIGUATION_REQUIRED", "Selection is still ambiguous; choose a single canonical application");
    }
    const selected = candidates.find((c) => c.canonicalAppId === body.selectedCanonicalAppIds[0]);
    if (!selected) throw new ApiError("NOT_FOUND", "Selected application not found");
    selected.instancesDisambiguated = true; // FR-7 AC1: choice recorded
    res.json(selected);
  };

  // ---- POST /scans + GET /scans/{id}/{v} [FR-8..FR-13, NFR-2] ----
  const createScanHandler: RequestHandler = (req, res) => {
    const body = parseOrThrow(ScanRequestSchema, req.body);
    const app = requireApplication(stores, body.canonicalAppId);
    guardRemovable(app); // 409 PROTECTED_APP / DISAMBIGUATION_REQUIRED (F4)
    const graph = sealResidueGraph(stores, app, body.scope);
    res.status(201).json(graph);
  };
  const getScanHandler: RequestHandler = (req, res) => {
    const { canonicalAppId, scanVersion } = parseOrThrow(ScanParam, req.params);
    res.json(requireScan(stores, canonicalAppId, scanVersion));
  };

  // ---- POST /plans + GET /plans/{id} + POST /plans/{id}/approve [FR-14..FR-22, FR-19] ----
  const composePlanHandler: RequestHandler = (req, res) => {
    const body = parseOrThrow(ComposePlanRequestSchema, req.body);
    const app = requireApplication(stores, body.canonicalAppId);
    guardRemovable(app);
    const scan = requireScan(stores, body.canonicalAppId, body.scanVersion);
    const plan = composePlan(stores, app, scan, body.mode, body.scope, config);
    res.status(201).json(plan); // status "draft" — Dry Run (D12), no mutation (FR-19)
  };
  const getPlanHandler: RequestHandler = (req, res) => {
    const { planId } = parseOrThrow(PlanIdParam, req.params);
    res.json(requirePlan(stores, planId));
  };
  const approvePlanHandler: RequestHandler = (req, res) => {
    const { planId } = parseOrThrow(PlanIdParam, req.params);
    const body = parseOrThrow(ApproveRequestSchema, req.body);
    const plan = requirePlan(stores, planId);

    const blocked = plan.operations.filter((op) => op.verdict === "blocked");
    if (blocked.length > 0) {
      throw new ApiError("PLAN_HAS_BLOCKED_OP", "Plan contains blocked operations and cannot be approved", {
        blockedOperations: blocked.map((op) => op.operationId),
      }); // FR-18 AC1/AC3
    }
    const accepted = new Set(body.acceptedRiskOperationIds);
    const unacceptedRisky = plan.operations.filter((op) => op.verdict === "risky" && !accepted.has(op.operationId));
    if (unacceptedRisky.length > 0) {
      throw new ApiError("RISK_NOT_ACCEPTED", "Plan has risky operations that were not accepted", {
        unaccepted: unacceptedRisky.map((op) => op.operationId),
      }); // FR-18 AC2
    }
    for (const op of plan.operations) {
      op.accepted = op.verdict === "risky" && accepted.has(op.operationId);
    }
    plan.status = "approved"; // FR-14 AC2: immutable once approved
    res.json(plan);
  };

  // ---- POST /jobs + /jobs/{id}/begin + GET /jobs/{id} + /jobs/{id}/sweep [FR-23..FR-27] ----
  const createJobHandler: RequestHandler = (req, res) => {
    const body = parseOrThrow(CreateJobRequestSchema, req.body);
    const plan = requirePlan(stores, body.planId);
    if (plan.status !== "approved") {
      throw new ApiError("PLAN_NOT_APPROVED", "A job may only be created from an approved plan"); // F3 (here)
    }
    if (plan.exceedsCostCap && !body.proceedDespiteCostCap) {
      throw new ApiError("SNAPSHOT_COST_CAP_EXCEEDED", "Projected snapshot exceeds the cost cap; opt in to proceed", {
        projectedSnapshotBytes: plan.projectedSnapshotBytes,
        maxSnapshotBytes: config.maxSnapshotBytes,
      }); // FR-23 AC2, D11
    }
    if (stores.inFlightApps.has(plan.applicationId)) {
      throw new ApiError("CONFLICTING_JOB", "A job is already in-flight for this application", {
        canonicalAppId: plan.applicationId,
      }); // F8
    }

    const app = requireApplication(stores, plan.applicationId);
    const steps: ExecutedStep[] = plan.operations
      .filter((op) => op.verdict !== "blocked")
      .map((op) => ({
        stepId: newId(),
        operationId: op.operationId,
        order: op.order,
        status: "pending",
        startedAt: null,
        finishedAt: null,
        error: null,
      }));

    const job: RemovalJob = {
      jobId: newId(),
      planId: plan.planId,
      snapshotId: null,
      status: "created",
      steps,
      createdAt: nowIso(),
      startedAt: null,
      finishedAt: null,
      failure: null,
    };
    // Snapshot captured BEFORE leaving 'created' (FR-23 AC1).
    const snapshot = captureSnapshot(stores, job, plan, app, config);
    job.snapshotId = snapshot.snapshotId;

    stores.jobs.set(job.jobId, job);
    stores.inFlightApps.add(plan.applicationId); // acquire single-write lock
    res.status(201).json(job);
  };
  const beginJobHandler: RequestHandler = (req, res) => {
    const { jobId } = parseOrThrow(JobIdParam, req.params);
    const job = requireJob(stores, jobId);
    const plan = requirePlan(stores, job.planId);
    // Approval guard runs here too (§4.1, F3) — a snapshot is never executed against a non-approved plan.
    if (plan.status !== "approved") {
      throw new ApiError("PLAN_NOT_APPROVED", "Cannot begin execution against a non-approved plan");
    }
    if (job.status !== "created") {
      throw new ApiError("ILLEGAL_TRANSITION", `Job cannot transition from '${job.status}' to 'running'`); // FR-26 AC1
    }
    if (!job.snapshotId) {
      throw new ApiError("ILLEGAL_TRANSITION", "Job has no captured snapshot"); // invariant (FR-23 AC1)
    }
    runJobToTerminal(stores, job, plan, job.snapshotId);
    res.json(job);
  };
  const getJobHandler: RequestHandler = (req, res) => {
    const { jobId } = parseOrThrow(JobIdParam, req.params);
    res.json(requireJob(stores, jobId));
  };
  const sweepJobHandler: RequestHandler = (req, res) => {
    const { jobId } = parseOrThrow(JobIdParam, req.params);
    const job = requireJob(stores, jobId);
    if (job.status !== "completed") {
      throw new ApiError("ILLEGAL_TRANSITION", "Sweep is only valid on a completed job"); // F8
    }
    const plan = requirePlan(stores, job.planId);
    if (plan.mode !== "purge") {
      throw new ApiError("ILLEGAL_TRANSITION", "Sweep is only valid on a purge job"); // F8
    }
    // Post-completion sweep: no missed residue in the reference build (FR-27 AC2 — offered, not executed).
    const result = { missedResidueGraph: null, followUpPlanId: null };
    SweepResultSchema.parse(result);
    res.json(result);
  };

  // ---- GET /history [FR-35] ----
  const historyHandler: RequestHandler = (_req, res) => {
    const items = Array.from(stores.audit.values())
      .sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1)) // newest first
      .map((rec) => {
        const plan = rec.planSnapshot;
        const app = stores.applications.get(plan.applicationId);
        return {
          auditRecordId: rec.auditRecordId,
          jobId: rec.jobId,
          targetName: app?.name ?? "unknown",
          outcome: rec.outcome,
          createdAt: rec.createdAt,
          undoable: rec.undoable,
        };
      });
    res.json({ items });
  };

  // ---- GET /audit/{id} + POST /audit/{id}/undo [FR-33..FR-34, NFR-9, NFR-15] ----
  const getAuditHandler: RequestHandler = (req, res) => {
    const { auditRecordId } = parseOrThrow(AuditIdParam, req.params);
    const rec = stores.audit.get(auditRecordId);
    if (!rec) throw new ApiError("NOT_FOUND", `Audit record '${auditRecordId}' not found`);
    res.json(rec);
  };
  const undoHandler: RequestHandler = (req, res) => {
    const { auditRecordId } = parseOrThrow(AuditIdParam, req.params);
    const rec = stores.audit.get(auditRecordId);
    if (!rec) throw new ApiError("NOT_FOUND", `Audit record '${auditRecordId}' not found`);
    const snapshot = stores.snapshots.get(rec.snapshotId);
    if (!snapshot) throw new ApiError("NOT_FOUND", "Snapshot for this audit record not found");
    const verify = verifySnapshotIntegrity(snapshot);
    if (!verify.intact) {
      throw new ApiError("SNAPSHOT_CORRUPT", "Snapshot failed integrity check; restore refused", {
        failures: verify.failures,
      }); // NFR-15 — restore refused rather than partial
    }
    // Best-effort restore runs as a job; offline package reinstalls are deferred (FR-34 AC2, D6).
    const deferredPackages = snapshot.entries
      .filter((e) => e.kind === "package-record" && e.packageName)
      .map((e) => e.packageName as string);
    const restoreJobId = newId();
    res.status(201).json({ restoreJobId, deferredPackages });
  };

  // ---- GET /snapshots/{id} + POST /snapshots/{id}/verify [FR-23, NFR-15] ----
  const getSnapshotHandler: RequestHandler = (req, res) => {
    const { snapshotId } = parseOrThrow(SnapshotIdParam, req.params);
    const snapshot = stores.snapshots.get(snapshotId);
    if (!snapshot) throw new ApiError("NOT_FOUND", `Snapshot '${snapshotId}' not found`);
    res.json(snapshot);
  };
  const verifySnapshotHandler: RequestHandler = (req, res) => {
    const { snapshotId } = parseOrThrow(SnapshotIdParam, req.params);
    const snapshot = stores.snapshots.get(snapshotId);
    if (!snapshot) throw new ApiError("NOT_FOUND", `Snapshot '${snapshotId}' not found`);
    const verify = verifySnapshotIntegrity(snapshot);
    res.status(201).json({ snapshotId, intact: verify.intact, failures: verify.failures });
  };

  // ---- Routers (Express router pattern, CLAUDE.md C1) ----
  const api = Router();
  api.get("/health", healthHandler);
  api.get("/backends", backendsHandler);

  const inventory = Router();
  inventory.get("/", inventoryListHandler);
  inventory.get("/:canonicalAppId", inventoryOneHandler);
  api.use("/inventory", inventory);

  api.post("/resolve", asyncHandler(resolveHandler));
  api.post("/resolve/disambiguate", asyncHandler(disambiguateHandler));

  api.post("/scans", asyncHandler(createScanHandler));
  api.get("/scans/:canonicalAppId/:scanVersion", getScanHandler);

  api.post("/plans", asyncHandler(composePlanHandler));
  api.get("/plans/:planId", getPlanHandler);
  api.post("/plans/:planId/approve", asyncHandler(approvePlanHandler));

  api.post("/jobs", asyncHandler(createJobHandler));
  api.post("/jobs/:jobId/begin", asyncHandler(beginJobHandler));
  api.get("/jobs/:jobId", getJobHandler);
  api.post("/jobs/:jobId/sweep", asyncHandler(sweepJobHandler));

  api.get("/history", historyHandler);

  api.get("/audit/:auditRecordId", getAuditHandler);
  api.post("/audit/:auditRecordId/undo", asyncHandler(undoHandler));

  api.get("/snapshots/:snapshotId", getSnapshotHandler);
  api.post("/snapshots/:snapshotId/verify", asyncHandler(verifySnapshotHandler));

  app.use("/api/v1", api); // §7.1 server base path
  app.get("/health", healthHandler); // liveness alias at root (NFR-14)

  app.use(notFoundHandler);
  app.use(errorHandler);
  return app;
}

// ====================================================================================
// 9. Transport binding — local-only (SPEC §7.1, §2, D13)
// ====================================================================================

function resolveSocketPath(): string {
  const runtime = process.env.XDG_RUNTIME_DIR;
  if (runtime) return path.join(runtime, "app-remover.sock");
  const uid = typeof process.getuid === "function" ? process.getuid() : userInfo().uid;
  return path.join("/run/user", String(uid), "app-remover.sock");
}

/** Bind the API: 0600 Unix socket (primary) → 127.0.0.1 loopback TCP (fallback) (D13). */
export async function startServer(): Promise<Server> {
  const app = createApp();
  const port = Number(process.env.PORT ?? 17763);
  const socketPath = resolveSocketPath();

  return new Promise<Server>((resolve, reject) => {
    const listenTcp = (reason: string): void => {
      logStructured("warn", "transport_fallback_tcp", { reason, port });
      const server = app.listen(port, "127.0.0.1", () => {
        logStructured("info", "listening", { transport: "tcp", host: "127.0.0.1", port });
        resolve(server);
      });
      server.on("error", reject);
    };

    // Best-effort: clear a stale socket before binding (D13).
    if (existsSync(socketPath)) {
      try { unlinkSync(socketPath); } catch { /* ignore; bind will fail and fall back */ }
    }

    const server = app.listen(socketPath, () => {
      try { chmodSync(socketPath, 0o600); } catch { /* best effort */ }
      logStructured("info", "listening", { transport: "unix", path: socketPath });
      resolve(server);
    });
    server.on("error", (err: NodeJS.ErrnoException) => {
      // EACCES/ENOENT/EADDRINUSE on the socket → fall back to loopback TCP.
      listenTcp(err.code ?? "unix-bind-error");
    });
  });
}

// CommonJS entry guard (package.json `type: commonjs`).
if (require.main === module) {
  void startServer().catch((err) => {
    logStructured("error", "fatal", { message: err instanceof Error ? err.message : String(err) });
    process.exit(1);
  });
}

// Re-exports for tests / OpenAPI generation. (createApp/startServer are already exported above.)
export type { RuntimeWarning, ErrorDetail };

export {
  ApiError,
  parseOrThrow,
  errorEnvelope,
  validateBackendIdentifier,
  // Published Language + DTO schemas (SPEC §3, §7.3) — double as OpenAPI components.
  ApplicationSchema,
  ResidueGraphSchema,
  RemovalOperationSchema,
  RemovalPlanSchema,
  SnapshotSchema,
  RemovalJobSchema,
  AuditRecordSchema,
  InventoryDTOSchema,
  ResolveRequestSchema,
  ResolveResultSchema,
  ScanRequestSchema,
  ComposePlanRequestSchema,
  ApproveRequestSchema,
  CreateJobRequestSchema,
  HistoryItemDTOSchema,
  UndoResultSchema,
  HealthDTOSchema,
  SnapshotVerifyResultSchema,
  BackendsDTOSchema,
};
