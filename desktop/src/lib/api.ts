// Typed wrappers around Tauri commands (Rust serializes camelCase).
import { invoke } from "@tauri-apps/api/core";

export type PackageBackend =
  | "deb" | "snap" | "flatpak" | "appimage"
  | "pip" | "npm" | "manual" | "systemd";

export interface BackendStatus {
  backend: PackageBackend;
  present: boolean;
}

export interface HealthResponse {
  status: string;
  backends: BackendStatus[];
  dbWritable: boolean;
}

export interface InventoryItem {
  canonicalAppId: string;
  name: string;
  installMethods: string[];
  version?: string;
  icon: string | null;
  instanceCount: number;
  isProtected: boolean;
}

export interface InventoryResponse {
  items: InventoryItem[];
}

export interface Artifact {
  artifactId: string;
  category: string;
  target: string;
  ownerSet: string[];
  usageKind: "exclusive" | "shared" | "system";
  sizeBytes?: number;
  deletable: boolean;
  confidence: "high" | "medium" | "low";
  discoveredBy: string;
  scope?: "system" | "user";
}

export interface ResidueCounts {
  exclusive: number;
  shared: number;
  system: number;
}

export interface RuntimeWarning {
  kind: "process-running" | "lock-held" | "service-active";
  detail: string;
}

export interface ResidueGraph {
  applicationId: string;
  scanVersion: number;
  sealedAt: number;
  artifacts: Artifact[];
  counts: ResidueCounts;
  runtimeWarnings: RuntimeWarning[];
}

export type ResolveSource =
  | { kind: "desktop-entry"; path: string }
  | { kind: "canonical-id"; canonicalAppId: string };

export interface RemovalOperation {
  operationId: string;
  order: number;
  action: string;
  targetKind: string;
  targetRef: string;
  verdict: "safe" | "risky" | "blocked" | "manual-review";
  impact: string[];
  rationale: string;
  accepted: boolean;
}

export interface RemovalPlan {
  planId: string;
  applicationId: string;
  scanVersion: number;
  mode: string;
  scope: string;
  status: "draft" | "approved" | "superseded";
  operations: RemovalOperation[];
  projectedSnapshotBytes: number;
  exceedsCostCap: boolean;
  blockedReasons: string[];
  composedAt: number;
}

export interface ErrorDetail {
  code: string;
  message: string;
}

export interface ExecutedStep {
  stepId: string;
  operationId: string;
  order: number;
  status: "pending" | "running" | "succeeded" | "failed" | "skipped";
  startedAt?: number;
  finishedAt?: number;
  error?: ErrorDetail;
}

export interface RemovalJob {
  jobId: string;
  planId: string;
  snapshotId?: string;
  status: "created" | "running" | "completed" | "failed" | "rolled-back";
  steps: ExecutedStep[];
  createdAt: number;
  startedAt?: number;
  finishedAt?: number;
  failure?: ErrorDetail;
}

export interface UndoResult {
  auditRecordId: string;
  restoredFiles: number;
  reinstalledPackages: string[];
  deferredPackages: string[];
}

export interface HistoryItem {
  auditRecordId: string;
  jobId: string;
  outcome: string;
  undoable: boolean;
  createdAt: number;
  appName: string | null;
}

export interface HistoryResponse {
  items: HistoryItem[];
}

export interface AppError {
  error: { code: string; message: string; details?: unknown };
}

export const api = {
  health: () => invoke<HealthResponse>("health"),
  listBackends: () => invoke<BackendStatus[]>("list_backends"),
  listInventory: () => invoke<InventoryResponse>("list_inventory"),
  resolveApp: (source: ResolveSource) => invoke<unknown>("resolve_app", { source }),
  createScan: (canonicalAppId: string) =>
    invoke<ResidueGraph>("create_scan", { canonicalAppId }),
  getScan: (canonicalAppId: string, scanVersion: number) =>
    invoke<ResidueGraph>("get_scan", { canonicalAppId, scanVersion }),
  createPlan: (
    canonicalAppId: string,
    scanVersion: number,
    mode?: "remove" | "purge",
    scope?: "system-wide" | "current-user" | "both"
  ) =>
    invoke<RemovalPlan>("create_plan", { canonicalAppId, scanVersion, mode, scope }),
  getPlan: (planId: string) => invoke<RemovalPlan>("get_plan", { planId }),
  approvePlan: (planId: string, acceptedRiskOperationIds: string[]) =>
    invoke<RemovalPlan>("approve_plan", { planId, acceptedRiskOperationIds }),
  createJob: (planId: string, proceedDespiteCostCap?: boolean) =>
    invoke<RemovalJob>("create_job", { planId, proceedDespiteCostCap }),
  beginJob: (jobId: string) => invoke<RemovalJob>("begin_job", { jobId }),
  getJob: (jobId: string) => invoke<RemovalJob>("get_job", { jobId }),
  undoRemoval: (auditRecordId: string) =>
    invoke<UndoResult>("undo_removal", { auditRecordId }),
  getHistory: () => invoke<HistoryResponse>("get_history"),
};

export function describeError(e: unknown): string {
  const ae = e as AppError | undefined;
  if (ae?.error?.message) return `${ae.error.code}: ${ae.error.message}`;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return `${e}`;
  }
}
