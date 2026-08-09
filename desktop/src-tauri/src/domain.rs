//! Domain model — port of SPEC §3 (Zod schemas) to Rust types.
//! Structs serialize camelCase for the frontend; enums are kebab-case.

use serde::{Deserialize, Serialize};

// ----------------------------- enums -----------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageBackend {
    Deb,
    Snap,
    Flatpak,
    Appimage,
    Pip,
    Npm,
    Manual,
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMethod {
    Deb,
    Snap,
    Flatpak,
    Appimage,
    Manual,
    LanguagePackage,
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceKind {
    DpkgRecord,
    SnapList,
    FlatpakList,
    AppimageMagic,
    PipRecord,
    NpmRecord,
    OciManifest,
    DesktopEntry,
    FilePath,
    ReverseDependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactCategory {
    Binary,
    Config,
    Cache,
    Data,
    State,
    Service,
    DesktopEntry,
    Association,
    Dependency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageKind {
    /// ownerSet.len() == 1
    Exclusive,
    /// ownerSet.len() > 1
    Shared,
    /// OS-owned; never deletable
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScopeTag {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoverySource {
    Manifest,
    KnowledgeBase,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyVerdict {
    Safe,
    Risky,
    Blocked,
    ManualReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    UninstallPackage,
    StopService,
    DeleteFile,
    RemoveAssociation,
    PruneOrphan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemovalMode {
    Remove,
    Purge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemovalScope {
    SystemWide,
    CurrentUser,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanStatus {
    Draft,
    Approved,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    Created,
    Running,
    Completed,
    Failed,
    RolledBack,
}

// ----------------------------- structs -----------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInstanceRef {
    pub backend: PackageBackend,
    pub package_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeTag>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopEntryRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mimetypes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSource {
    pub method: InstallMethod,
    pub confidence: ConfidenceLevel,
    pub evidence: Vec<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_ref: Option<PackageInstanceRef>,
}

/// Canonical application aggregate (SPEC §3.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalApplication {
    pub canonical_app_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<DesktopEntryRef>,
    pub install_sources: Vec<InstallSource>,
    pub package_instance_refs: Vec<PackageInstanceRef>,
    pub is_protected: bool,
    pub instances_disambiguated: bool,
}

// --------------------------- scan graph ---------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeWarningKind {
    ProcessRunning,
    LockHeld,
    ServiceActive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeWarning {
    pub kind: RuntimeWarningKind,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidueCounts {
    pub exclusive: u32,
    pub shared: u32,
    pub system: u32,
}

/// A filesystem/system object associated with an application (SPEC §3.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub category: ArtifactCategory,
    pub target: String,
    pub owner_set: Vec<String>,
    pub usage_kind: UsageKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub deletable: bool,
    pub confidence: ConfidenceLevel,
    pub discovered_by: DiscoverySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeTag>,
}

/// A sealed, immutable residue graph for one application+version (SPEC §3.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidueGraph {
    pub application_id: String,
    pub scan_version: u32,
    pub sealed_at: i64,
    pub artifacts: Vec<Artifact>,
    pub counts: ResidueCounts,
    pub runtime_warnings: Vec<RuntimeWarning>,
}

// ----------------------------- resolve -----------------------------

/// Input to `resolve_app`. Tagged by `kind` (kebab); the canonical-id field is
/// camelCase so the frontend sends `{kind:"canonical-id", canonicalAppId}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ResolveSource {
    DesktopEntry {
        path: String,
    },
    CanonicalId {
        #[serde(rename = "canonicalAppId")]
        canonical_app_id: String,
    },
}

// ------------------------------ plan -------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalOperation {
    pub operation_id: String,
    pub order: u32,
    pub action: Action,
    /// "artifact" (a residue path) or "package" (a package instance).
    pub target_kind: String,
    /// artifact_id, or package_name for package targets.
    pub target_ref: String,
    pub verdict: SafetyVerdict,
    /// Canonical app ids that break if this op runs.
    pub impact: Vec<String>,
    pub rationale: String,
    pub accepted: bool,
}

/// A dry-run removal plan over a sealed residue graph (SPEC §3.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    pub plan_id: String,
    pub application_id: String,
    pub scan_version: u32,
    pub mode: RemovalMode,
    pub scope: RemovalScope,
    pub status: PlanStatus,
    pub operations: Vec<RemovalOperation>,
    pub projected_snapshot_bytes: u64,
    pub exceeds_cost_cap: bool,
    pub blocked_reasons: Vec<String>,
    pub composed_at: i64,
}

// ------------------------- job / snapshot -------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedStep {
    pub step_id: String,
    pub operation_id: String,
    pub order: u32,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDetail>,
}

/// A removal job executing an approved plan (SPEC §3.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalJob {
    pub job_id: String,
    pub plan_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub status: JobStatus,
    pub steps: Vec<ExecutedStep>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<ErrorDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotEntryKind {
    FileBackup,
    PackageRecord,
    UnitRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    pub category: String,
    pub original_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_path: Option<String>,
    pub checksum: String,
    pub size_bytes: u64,
    pub kind: SnapshotEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
}

/// A pre-removal backup enabling undo (SPEC §3.6, NFR-15).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub snapshot_id: String,
    pub job_id: String,
    pub captured_at: i64,
    pub checksum_algo: String,
    pub manifest_checksum: String,
    pub total_bytes: u64,
    pub excludes_cache: bool,
    pub blob_root: String,
    pub entries: Vec<SnapshotEntry>,
}

/// Append-only, hash-chained audit record (SPEC §3.7, NFR-9).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub audit_record_id: String,
    pub job_id: String,
    pub plan_snapshot: serde_json::Value,
    pub steps: Vec<ExecutedStep>,
    pub snapshot_id: String,
    pub outcome: String,
    pub undoable: bool,
    pub created_at: i64,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
}

/// Result of undoing a completed removal (SPEC §3.7 UndoResult).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub audit_record_id: String,
    pub restored_files: u32,
    pub reinstalled_packages: Vec<String>,
    pub deferred_packages: Vec<String>,
}


