# REQUIREMENTS.md — App-Remover

> **Stage 1 (Requirements).** Source of truth for *what* App-Remover must do and *how well*.
> *Inputs:* [DOMAIN.md](../00-domain/DOMAIN.md), [CONTEXT_MAP.md](../00-domain/CONTEXT_MAP.md), `raw_idea.txt`.
> *Downstream consumer:* `docs/02-spec/SPEC.md` (references FR/NFR IDs), gated by `docs/03-review/VERDICT.md`.
> *Glossary:* the Ubiquitous Language in [DOMAIN.md §3](../00-domain/DOMAIN.md) is normative here. This document
> reuses those terms verbatim and adds none.
>
> **Traceability convention:** the ID scheme below is *pinned*. `FR-n`, `NFR-n`, and `D-n` IDs are referenced
> verbatim by `SPEC.md` and the review gate; they must not be renumbered. Open Questions are cited as
> `OQ1`–`OQ14` against [DOMAIN.md §6](../00-domain/DOMAIN.md).

---

## 1. Purpose

Translate the conceptual domain model into **testable, traceable** requirements. Every Functional Requirement
(FR) and Non-Functional Requirement (NFR) below:

- has a stable ID referenced by later stages,
- is verifiable by an explicit acceptance criterion (Given/When/Then or a measurable threshold),
- names its **originating sub-domain(s)** from [DOMAIN.md §2](../00-domain/DOMAIN.md), and
- links, where relevant, to a **Requirements Decision (`D-n`)** that resolves an open question from
  [DOMAIN.md §6](../00-domain/DOMAIN.md).

### 1.1 In Scope

- Detecting how an installed Ubuntu desktop app was installed, computing all of its residue, planning a
  safe removal, executing it reversibly, and undoing it — across apt/deb, snap, flatpak, AppImage,
  pip/npm, manual/script installs, and user/system `systemd` services.
- A local desktop-facing application API that a client ("click an icon") drives.

### 1.2 Out of Scope (for this version)

- Non-Ubuntu / non-Debian distributions and package formats (RPM, pacman, Homebrew).
- Remote/multi-host fleet management; App-Remover operates on the local host only.
- macOS / Windows. Mobile. GUI widget toolkit choice (a spec-stage decision).
- Antivirus / malware remediation.

### 1.3 End-to-end Requirement Flow (traceability spine)

The capability areas below form a strict pipeline that mirrors the [Core Domain Rules (DOMAIN §5)](../00-domain/DOMAIN.md):

discover (`FR-1`) → select (`FR-2` / `FR-3`) → **resolve & canonicalize (`FR-4`–`FR-7`)** [Rule 1] → map residue (`FR-8`–`FR-13`) → plan (`FR-14`–`FR-22`) [Rules 2–4] → dry-run (`FR-19`) → **snapshot (`FR-23`)** [Rule 5] → execute (`FR-24`–`FR-29`) [Rule 6] → sweep (`FR-27`) → audit/undo (`FR-33`–`FR-35`).

No stage may begin before its predecessor's invariant holds (e.g., planning never precedes canonicalization;
execution never precedes an approved plan **and** a Snapshot). This ordering is normative for `SPEC.md`.

---

## 2. Assumptions & Constraints

| # | Assumption / Constraint |
|---|---|
| A1 | Target platform is **Ubuntu 22.04 LTS and 24.04 LTS** on x86-64 (see NFR-10). |
| A2 | App-Remover runs **locally** with a single interactive user on the desktop; multi-user systems are supported but treated as an explicit scope choice (FR-22). |
| A3 | "Reference machine" for performance NFRs = a modern Ubuntu LTS desktop with an SSD and ≥ 8 GB RAM. |
| A4 | System removals require elevated privilege; the user is a sudoer or can authorize via polkit (FR-32). |
| A5 | Package-manager tooling (`dpkg`, `snap`, `flatpak`, `pip`, `npm`) is present and functional; absent backends degrade gracefully (NFR-14), they do not crash the product. |
| C1 | **Constraint (from CLAUDE.md):** backend is Node.js + Express + TypeScript (strict), Zod for validation. Requirements are technology-neutral; this binds the *implementation*, not the requirements. |
| C2 | **Constraint:** requirements must be **traceable** to a sub-domain and **testable**; unverifiable or invented requirements are rejected at review. |

---

## 3. Requirements Decisions (Resolving Domain Open Questions)

[DOMAIN.md §6](../00-domain/DOMAIN.md) defers 14 open questions to Stage 1. Each is resolved below as a
**Decision (`D-n`)** and realized by one or more FRs. These decisions are normative; the spec must implement
them and the reviewer must verify coverage.

| Decision | Resolves (OQ) | Resolution | Realized by |
|---|---|---|---|
| **D1** | OQ1 — "associated residue" | A **Residue Policy** defines artifact locations by category. Default policy covers: `binary` (`/usr/bin`, `/usr/lib`, `/opt`), `config` (`/etc/<app>`, `~/.config/<app>`), `cache` (`~/.cache/<app>`, `/var/cache/<app>`), `data` (`~/.local/share/<app>`, `/var/lib/<app>`), `state` (dconf/gsettings schemas, `/run`, `/var/run`), `service` (systemd unit files — user and system), `desktop-entry` (`~/.local/share/applications`, `/usr/share/applications`), `association` (mimetype handlers, default-apps), `dependency` (package reverse-deps). Policy is configurable; additions/exceptions do not change the category model. | FR-8, FR-9 |
| **D2** | OQ2 — "without breaking any other" source of truth | The **Owner Set** is the single source of truth for "who uses an artifact," computed by **blending three signals**: (a) package file-ownership from the PM ACL (authoritative where it exists), (b) package reverse-dependencies, (c) heuristic shared-path detection for unpackaged files. Breakage = an artifact/package whose Owner Set contains an app other than the Removal Target. | FR-10, FR-15 |
| **D3** | OQ3 — cross-system collisions | Canonicalization **merges** installs of the same app across systems into one Canonical Application. When *distinct* install instances remain (e.g. apt **and** snap Firefox), the user is shown each instance with its provenance and must **explicitly disambiguate** (no silent default removal of the wrong instance). | FR-6, FR-7 |
| **D4** | OQ4 — residue discovery strategy | A **blend**: (1) package manifests (authoritative for packaged apps), (2) a curated **knowledge base** of known residues (seeded, growable), (3) naming-heuristic fallback. Heuristic-only matches are flagged lower-confidence. | FR-12 |
| **D5** | OQ5 — privilege model | Privilege is acquired and **scoped per session** via **polkit/pkexec** (primary on Ubuntu), with **least privilege** — never a full-root shell. Sensitive operations elevate per-operation. | FR-32, NFR-7 |
| **D6** | OQ6 — undo completeness | A Snapshot captures, for to-be-deleted items: **file-level backup of `config`/`data`** artifacts, the **package list** for reinstall, and **service-unit contents**. `cache` artifacts are **not** backed up (rebuildable). Package reinstall is **best-effort** and may require network; offline undo restores files and unit state but cannot guarantee package re-download. | FR-23, FR-34, NFR-6 |
| **D7** | OQ7 — Protected Apps | A **blocklist** forbids removal of system-critical components: the desktop environment, `glibc`/core libs, the display server (X/Wayland), `systemd`, the package managers (`dpkg`/`apt`, `snapd`, `flatpak`), and **App-Remover itself**. Core blocklist entries are **non-overridable**. | FR-20 |
| **D8** | OQ8 — user vs system scope | The user selects scope per removal: **system-wide** (package + global config/data, affects all users), **current-user** (user-scope data/config/units only), or **both** (default). Multi-user behavior is defined by this choice, not assumed. | FR-22 |
| **D9** | OQ9 — non-packaged apps | Detection by signature/pattern: AppImage (AppImageType magic + integration data), extracted tarballs & source builds (bin-path + shebang heuristics), pip/npm globals (their own listing commands), container images (OCI manifest). Their residue is modeled via D1 policy + D4 KB; low-confidence matches are flagged. | FR-13 |
| **D10** | OQ10 — concurrency / running app | Before removal, App-Remover **detects running processes and held locks** for the target. It attempts a **graceful stop of app-owned services**, then warns the user; a confirmed-running, locked target can be refused or user-overridden at `risky` verdict — never silently removed. | FR-28 |
| **D11** | OQ11 — snapshot disk cost | A **snapshot cost cap** (default configurable) is enforced; if the projected snapshot exceeds the cap, the job **warns and aborts before any deletion** unless the user explicitly opts to proceed. `cache` is excluded by default (D6). | FR-23, NFR-2 |
| **D12** | OQ12 — dry-run UX | **Dry Run is mandatory and first-class**: no destructive operation may execute before the computed RemovalPlan has been computed and presented. Dry Run is the default path; execution is an explicit subsequent step. | FR-19 |
| **D13** | OQ13 — interaction surface | App-Remover is a **local Node service exposing an application API** (an OHS/PL per [CONTEXT_MAP.md §6](../00-domain/CONTEXT_MAP.md)) consumed by a desktop client. The client handles "click an icon"; the backend never assumes a GUI. | FR-36 |
| **D14** | OQ14 — confirmation & risk | `risky` and `manual-review` verdicts are **surfaced with their concrete Impact** (which apps break) and require **recorded user acceptance** before approval. `blocked` verdicts **cannot be overridden**. | FR-18 |

---

## 4. Functional Requirements

Priority uses **MoSCoW** — **M**ust (required for v1 acceptance), **S**hould (high-value, scheduled if time
permits), **C**ould (desirable, may be deferred). FRs are grouped by capability area; each names its
originating sub-domain(s).

### 4.1 Application Discovery & Selection — *origins: Application Inventory, Desktop Integration*

#### FR-1: Enumerate installed applications across all package systems — **M**
*Statement:* App-Remover enumerates every user-facing installed application across all supported package
systems into a single inventory read model.
- **AC1:** *Given* a system with deb, snap, and flatpak apps installed, *When* inventory is enumerated,
  *Then* the result contains apps from **all three** systems, each with a canonical name, icon (where
  available), version, and detected install method.
- **AC2:** *Given* a package backend is **not installed** (e.g. flatpak absent), *When* inventory is
  enumerated, *Then* the product skips that backend and still returns the others without error.
- **AC3:** *Given* two installs of the same app from different systems, *When* enumerated, *Then* they are
  presented in a form that supports later canonicalization (FR-6).

#### FR-2: Present a selectable application list — **M**
*Statement:* The user can browse/search the inventory and select a Removal Target by canonical identity.
- **AC1:** *Given* an enumerated inventory, *When* the user searches by name, *Then* matching apps are
  returned within the latency bound in NFR-1.
- **AC2:** *Given* a selected app, *When* the selection is committed, *Then* exactly one Canonical
  Application (FR-4) becomes the Removal Target — never an ambiguous raw package.

#### FR-3: Initiate removal from a desktop entry / icon — **M**
*Statement:* A user can initiate removal by activating the app's desktop entry (the "click an icon" path).
- **AC1:** *Given* a `.desktop` entry for an app, *When* the user invokes "remove" from that entry via the
  client, *Then* the backend resolves the entry to a Canonical Application (FR-4) and begins the planning
  flow — no manual package-name entry required.

### 4.2 Provenance & Canonicalization — *origin: Provenance Detection*

#### FR-4: Resolve a selection to a Canonical Application — **M**
*Statement:* A desktop entry or inventory selection is resolved to one disambiguated Canonical Application
with provenance **before** any planning begins (Domain Rule 1).
- **AC1:** *Given* a desktop entry with a matching app, *When* resolved, *Then* the result is a Canonical
  Application with **≥ 1** `InstallSource` carrying evidence, **or** is explicitly classified `manual`.
- **AC2:** *Given* an unresolvable entry, *When* resolved, *Then* planning is **not started** and the user
  receives a clear "could not identify how this was installed" result.

#### FR-5: Detect install method(s) with evidence and confidence — **M**
*Statement:* Each `InstallSource` records its method, supporting evidence, and a confidence value.
- **AC1:** *Given* a snap app, *When* provenance is detected, *Then* an `InstallSource` of method `snap`
  is produced with evidence (e.g. `snap list` record).
- **AC2:** *Given* a heuristic-only match (e.g. inferred manual install), *When* provenance is detected,
  *Then* confidence is marked lower than a manifest-backed match.

#### FR-6: Merge duplicates into one Canonical Application — **M** *(D3)*
*Statement:* Installs of the same app across package systems are collapsed into one Canonical Application.
- **AC1:** *Given* the same app installed via apt **and** flatpak, *When* canonicalized, *Then* they share
  one `CanonicalAppId` and the Application references **multiple** `PackageInstanceRef`s.
- **AC2:** *Given* a canonicalized application, *When* used downstream, *Then* all residue, planning, and
  execution refer to the single canonical identity.

#### FR-7: Disambiguate distinct install instances — **M** *(D3)*
*Statement:* When genuinely separate install instances exist, the user explicitly chooses the target; no
silent default removal of the wrong instance.
- **AC1:** *Given* apt Firefox and snap Firefox as distinct instances, *When* the user initiates removal,
  *Then* both instances are shown with provenance and the user must select which (or both) to target.
- **AC2:** *Given* the user has **not** disambiguated, *When* planning is requested, *Then* planning is
  **blocked** until disambiguation is recorded.

### 4.3 Residue Mapping — *origin: Residue Mapping*

#### FR-8: Build a complete ResidueGraph by residue policy — **M** *(D1)*
*Statement:* App-Remover computes the complete set of artifacts for the Removal Target across all policy
categories (D1) into a `ResidueGraph`, each artifact annotated with its Owner Set.
- **AC1:** *Given* an app with config in `~/.config/<app>` and a system service unit, *When* scanned,
  *Then* both appear as distinct `Artifact`s with categories `config` and `service`.
- **AC2:** *Given* a completed scan, *When* the graph is sealed, *Then* every `Artifact`'s Owner Set
  **includes** the owning Application (Domain invariant), and the graph is immutable thereafter.
- **AC3:** *Given* a re-scan, *When* it completes, *Then* it yields a **new `ScanVersion`** rather than
  mutating the prior graph.

#### FR-9: Classify each artifact by category — **M**
*Statement:* Every artifact is classified into exactly one of: `binary`, `config`, `cache`, `data`,
`state`, `service`, `desktop-entry`, `association`, `dependency`.
- **AC1:** *Given* a scanned artifact, *When* classified, *Then* it carries exactly one valid category
  value; unclassifiable items are flagged for review rather than silently dropped.

#### FR-10: Compute the Owner Set per artifact — **M** *(D2)*
*Statement:* Each artifact's Owner Set is computed by blending package file-ownership, reverse-dependency,
and shared-path heuristic signals (D2).
- **AC1:** *Given* a config file owned only by the target's deb, *When* the Owner Set is computed, *Then*
  it contains exactly {Removal Target}.
- **AC2:** *Given* a shared library used by another app, *When* the Owner Set is computed, *Then* it
  contains that other app, marking the artifact `shared`.
- **AC3:** *Given* an OS-owned path (e.g. under `/usr/lib` belonging to a base package), *When* the Owner
  Set is computed, *Then* it is classified `system` and never deletable.

#### FR-11: Derive UsageKind from the Owner Set — **M**
*Statement:* `UsageKind` is a pure function of the Owner Set: size 1 → `exclusive`; >1 → `shared`;
OS-owned → `system`.
- **AC1:** *Given* an artifact whose Owner Set is exactly {Removal Target}, *When* classified, *Then*
  `UsageKind = exclusive`.
- **AC2:** *Given* a `system` artifact, *When* a plan is composed, *Then* no delete operation is generated
  for it.

#### FR-12: Blend manifest + knowledge-base + heuristic discovery — **S** *(D4)*
*Statement:* Residue discovery combines package manifests, a curated knowledge base, and naming heuristics,
with confidence reflecting the source.
- **AC1:** *Given* a deb app with a manifest, *When* discovered, *Then* manifest-listed files are
  included at high confidence.
- **AC2:** *Given* an app with a known KB residue entry, *When* discovered, *Then* that entry is included
  even if absent from the manifest.
- **AC3:** *Given* a heuristic-only `~/.config/<name>` match, *When* discovered, *Then* it is included but
  flagged lower-confidence than manifest/KB matches.

#### FR-13: Detect and map non-packaged apps — **S** *(D9)*
*Statement:* AppImage, extracted tarballs/source builds, pip/npm globals, and container images are detected
and given a residue model via D1+D4.
- **AC1:** *Given* an AppImage with integration data, *When* detected, *Then* it is identified as
  `AppImage` provenance and its desktop entry + integration files enter the ResidueGraph.
- **AC2:** *Given* a pip-global package, *When* detected, *Then* its files are listed via the pip backend
  and mapped as artifacts. *(Could)* — full source-build residue may be heuristic-only.

### 4.4 Removal Safety & Planning — *origin: Removal Safety*

#### FR-14: Compose a RemovalPlan with operations, verdicts, and impact — **M**
*Statement:* From a `ResidueGraph` and the dependency graph, App-Remover composes an ordered
`RemovalPlan` of `RemovalOperation`s, each with an action, order, safety verdict, and impact.
- **AC1:** *Given* a sealed ResidueGraph, *When* a plan is composed, *Then* every exclusive artifact
  yields an operation and every shared/system artifact yields a `blocked` or omitted operation per verdict
  rules.
- **AC2:** *Given* a composed plan, *When* approved inputs change (e.g. new scan), *Then* a **new** plan
  is produced — an approved plan is immutable.

#### FR-15: Compute Impact per operation — **M** *(D2)*
*Statement:* Each operation records the set of **other** applications that would break if it executed
(Domain: `Impact`).
- **AC1:** *Given* an operation on a shared artifact used by App B, *When* impact is computed, *Then*
  `Impact = {App B}`.
- **AC2:** *Given* an operation with non-empty Impact, *When* verdict is assigned, *Then* the verdict is
  **never** `safe`.

#### FR-16: Assign a SafetyVerdict per operation — **M**
*Statement:* Each operation receives exactly one verdict: `safe`, `risky`, `blocked`, or `manual-review`.
- **AC1:** *Given* an exclusive artifact with empty impact, *When* verdicted, *Then* verdict = `safe`.
- **AC2:** *Given* a system artifact, *When* verdicted, *Then* verdict = `blocked`.
- **AC3:** *Given* a heuristic-only, low-confidence residue, *When* verdicted, *Then* verdict =
  `manual-review`.

#### FR-17: Enforce plan ordering rules — **M**
*Statement:* Operations are ordered: dependents before dependencies; services stopped before their files
deleted; packages removed before the residue sweep; orphans pruned last.
- **AC1:** *Given* a service and its binary, *When* ordered, *Then* the stop-service operation precedes
  the delete-file operation.
- **AC2:** *Given* a package and a now-orphaned dependency, *When* ordered, *Then* the package removal
  precedes orphan pruning, which is last.

#### FR-18: Gate approval by verdict and recorded risk acceptance — **M** *(D14)*
*Statement:* A plan cannot reach `approved` while any `blocked` operation remains; `risky` operations
require recorded user acceptance; `blocked` cannot be overridden.
- **AC1:** *Given* a plan containing a `blocked` operation, *When* approval is attempted, *Then* approval
  is **refused**.
- **AC2:** *Given* a plan containing a `risky` operation, *When* the user accepts its Impact, *Then* the
  acceptance is recorded and the operation may proceed; without acceptance, approval is refused.
- **AC3:** *Given* a `blocked` operation, *When* the user attempts to force it, *Then* it remains blocked.

#### FR-19: Dry Run — compute and present the plan without executing — **M** *(D12)*
*Statement:* The user can request a non-destructive preview of the full RemovalPlan before any destructive
step.
- **AC1:** *Given* a Removal Target, *When* Dry Run is invoked, *Then* the complete plan (operations,
  verdicts, impact, ordering) is returned and **no** artifact, package, or service is modified.
- **AC2:** *Given* a system under Dry Run, *When* it completes, *Then* filesystem/package state is
  byte-for-byte unchanged (verifiable by checksum before/after).

#### FR-20: Enforce the Protected Apps blocklist — **M** *(D7)*
*Statement:* System-critical apps in the blocklist cannot be planned or executed for removal; core entries
are non-overridable.
- **AC1:** *Given* App-Remover itself, or `glibc`, or the desktop environment as a target, *When* removal
  is attempted, *Then* it is **refused** with a "protected app" reason.
- **AC2:** *Given* a core blocklist entry, *When* the user attempts to override, *Then* removal remains
  refused.

#### FR-21: Removal-mode selection — Remove (shallow) vs Purge (deep) — **M**
*Statement:* The user selects between `Remove` (uninstall packages only) and `Purge` (packages + exclusive
residue).
- **AC1:** *Given* mode = `Remove`, *When* a plan is composed, *Then* only package-uninstall operations
  are generated — no config/data deletion.
- **AC2:** *Given* mode = `Purge`, *When* a plan is composed, *Then* exclusive `config`/`data`/`cache`
  artifacts are also included.

#### FR-22: User/system scope selection — **S** *(D8)*
*Statement:* The user selects `system-wide`, `current-user`, or `both`; behavior follows the choice.
- **AC1:** *Given* scope = `current-user`, *When* a plan is composed, *Then* only user-scope artifacts and
  user units are included; system packages are untouched.
- **AC2:** *Given* scope = `system-wide`, *When* executed, *Then* the removal affects all users.

### 4.5 Removal Execution — *origin: Removal Execution*

#### FR-23: Snapshot before any destructive step — **M** *(D6, D11)*
*Statement:* A `Snapshot` of restorable state is captured **before** a job leaves `created`, subject to the
cost cap; no destructive operation runs without one.
- **AC1:** *Given* an approved plan, *When* a job is created, *Then* a Snapshot exists before the job
  transitions to `running`.
- **AC2:** *Given* a projected Snapshot exceeding the cost cap, *When* the job is created, *Then* it warns
  and **aborts before deletion** unless the user explicitly proceeds.
- **AC3:** *Given* `cache` artifacts in scope, *When* snapshotted, *Then* they are excluded from the backup
  by default (rebuildable).

#### FR-24: Execute the approved plan in dependency-correct order — **M**
*Statement:* The job executes operations in the plan's order; a job cannot start unless its plan is
`approved`.
- **AC1:** *Given* a job whose plan is not `approved`, *When* `begin()` is called, *Then* it is refused.
- **AC2:** *Given* an approved plan, *When* executed, *Then* operations run in the order produced by FR-17.

#### FR-25: Automatic rollback on step failure — **M**
*Statement:* On any step failure, the job rolls back using its Snapshot before reaching a terminal state;
a partial removal is never a final state.
- **AC1:** *Given* a job `running` and a step that errors, *When* the failure occurs, *Then* the job
  transitions to `failed` and then to `rolled_back` using the Snapshot.
- **AC2:** *Given* a `failed` job, *When* it reaches a terminal state, *Then* that state is `rolled_back`,
  never `failed` (no half-removed terminal state).

#### FR-26: Removal Job state machine — **M**
*Statement:* A job follows `created → running → completed | failed → rolled_back` strictly; illegal
transitions are rejected.
- **AC1:** *Given* a `completed` job, *When* a transition to `running` is attempted, *Then* it is rejected.
- **AC2:** *Given* any job, *When* transitions occur, *Then* they conform exactly to the state machine in
  [DOMAIN.md §4.4](../00-domain/DOMAIN.md).

#### FR-27: Leftover Sweep after removal — **S**
*Statement:* After completion, a pass detects residue the plan missed and may yield a fresh ResidueGraph
for a follow-up plan.
- **AC1:** *Given* a `completed` Purge job, *When* the sweep runs, *Then* any still-present exclusive
  residue is reported.
- **AC2:** *Given* sweep findings, *When* the user opts to act, *Then* a follow-up plan is offered (not
  auto-executed).

#### FR-28: Handle running / locked apps — **S** *(D10)*
*Statement:* Before removal, running processes and held locks are detected; app-owned services are stopped
gracefully; a locked target is surfaced, never silently removed.
- **AC1:** *Given* the target's process is running, *When* removal is initiated, *Then* the user is warned
  and a graceful stop of app-owned services is attempted.
- **AC2:** *Given* a held lock that cannot be released, *When* the user does not override, *Then* the
  operation is verdicted `risky`/`manual-review` and not auto-executed.

#### FR-29: Per-step status, error capture, and reporting — **C**
*Statement:* Each `ExecutedStep` records its status, timestamp, and any error; job progress is queryable.
- **AC1:** *Given* a running job, *When* progress is queried, *Then* the current step and completed steps
  with statuses are returned.
- **AC2:** *Given* a failed step, *When* its record is inspected, *Then* the error detail and timestamp are
  present.

### 4.6 Package Manager Integration — *origin: Package Manager Integration (ACL)*

#### FR-30: Uniform query surface across backends — **M**
*Statement:* A single uniform query API abstracts apt/dpkg, snap, flatpak, AppImage, pip/npm, manual, and
systemd so the core never invokes backend-specific tooling directly.
- **AC1:** *Given* any supported backend, *When* "list installed packages" is queried, *Then* a uniform
  result shape is returned regardless of backend.
- **AC2:** *Given* the core domain, *When* inspecting code paths, *Then* no `dpkg`/`snap`/`flatpak` shell
  commands appear outside the ACL adapters.

#### FR-31: Uniform command surface — **M**
*Statement:* The ACL exposes uniform commands: `uninstall-package`, `stop-service`, `delete-file`,
`prune-orphan`, plus queries for reverse-dependencies and file-ownership.
- **AC1:** *Given* a deb and a snap to uninstall, *When* `uninstall-package` is invoked for each, *Then*
  both succeed through their respective adapters under one command name.
- **AC2:** *Given* a reverse-dependency query, *When* issued for a deb package, *Then* it returns the list
  of packages depending on it.

### 4.7 Privilege & OS Access — *origin: Privilege & OS Access*

#### FR-32: Acquire and scope elevated privileges — **M** *(D5)*
*Statement:* Privileged operations acquire scoped, least-privilege elevation via polkit/pkexec, per
session, never via a full-root shell.
- **AC1:** *Given* an operation needing root, *When* it runs, *Then* privilege is acquired via the scoped
  mechanism and released when the operation completes.
- **AC2:** *Given* any privileged path, *When* audited, *Then* no unrestricted interactive root shell is
  spawned.

### 4.8 Audit & Undo — *origin: Audit & Undo Store*

#### FR-33: Persist an immutable AuditRecord per job — **M**
*Statement:* Every completed or failed/rolled-back job produces one immutable AuditRecord (Snapshot ref,
plan, steps, outcome).
- **AC1:** *Given* a terminal job, *When* it finishes, *Then* exactly one immutable AuditRecord exists for
  it and cannot be altered after write.

#### FR-34: Undo / Restore a completed removal — **M** *(D6)*
*Statement:* The user can reverse a previously completed removal using its AuditRecord + Snapshot, restoring
backed-up files, unit state, and reinstalling packages (best-effort per D6).
- **AC1:** *Given* a completed removal with a Snapshot, *When* Undo is invoked, *Then* backed-up
  `config`/`data` files and unit state are restored.
- **AC2:** *Given* a package removed offline, *When* Undo attempts reinstall and no network is available,
  *Then* file/unit state is restored and the package reinstall is reported as deferred/best-effort, not
  silently failed.

#### FR-35: View removal history — **S**
*Statement:* The user can view past removals and their outcomes from the Audit store.
- **AC1:** *Given* prior jobs, *When* history is opened, *Then* each is listed with target, timestamp, and
  outcome, and an Undo action where restorable.

### 4.9 Client / Interaction Surface — *origin: Removal Execution boundary (D13)*

#### FR-36: Application API for the desktop client — **M** *(D13)*
*Statement:* The backend exposes an application API (OHS) that a desktop client uses to enumerate, resolve,
dry-run, approve, execute, and undo — the backend does not assume a GUI.
- **AC1:** *Given* the client, *When* it invokes the API, *Then* discovery (FR-1), planning (FR-14/19),
  execution (FR-24), and undo (FR-34) are each reachable as discrete operations.
- **AC2:** *Given* the backend with no client connected, *When* the API is exercised directly, *Then* all
  capabilities function without a GUI present.

---

## 5. Non-Functional Requirements

Each NFR states a measurable threshold. "Reference machine" is defined in §2 (A3).

| ID | Category | Requirement | Acceptance Criterion (testable) |
|---|---|---|---|
| **NFR-1** | Performance | Inventory enumeration is responsive. | On the reference machine, enumerating **≤ 300** installed apps returns in **≤ 8 s**. |
| **NFR-2** | Performance | Residue scan is bounded. | A full ResidueGraph scan for a typical app completes in **≤ 30 s**; a projected Snapshot exceeding the cost cap (FR-23 AC2) is reported **before** scan work that would breach it. |
| **NFR-3** | Performance | Planning / Dry Run is responsive. | Dry Run (FR-19) for a typical app returns the full plan in **≤ 15 s** on the reference machine. |
| **NFR-4** | Safety / Correctness | No unintended modification of shared/system artifacts. | Across the regression suite, **zero** operations modify an artifact whose `UsageKind ∈ {shared, system}`; verifiable by checksumming protected paths before/after. |
| **NFR-5** | Reliability | No half-removed terminal state. | **≥ 99%** of injected-step-failure jobs reach `rolled_back` without manual intervention (FR-25). |
| **NFR-6** | Reversibility | Undo restores snapshotted state. | In a no-network test, Undo restores **≥ 99%** of backed-up `config`/`data` artifacts and unit state (D6). |
| **NFR-7** | Security | Least-privilege, injection-safe privilege use. | No unrestricted root shell (FR-32); all PM-ACL inputs are validated (Zod) so package/app identifiers cannot inject shell arguments; verified by fuzz/injection tests. |
| **NFR-8** | Security | No secret / sensitive-data leakage. | Logs and AuditRecords never persist raw credentials; file contents of backed-up artifacts are stored with restricted permissions and are not echoed to logs. |
| **NFR-9** | Auditability | Complete, immutable audit trail. | Every destructive operation is recorded in an immutable AuditRecord with before-state (FR-33); a tamper/append-only test confirms records cannot be modified after write. |
| **NFR-10** | Compatibility | Supported Ubuntu stack. | Verified to operate on **Ubuntu 22.04 LTS and 24.04 LTS**, on **GNOME, KDE Plasma, and XFCE**, across **apt/dpkg, snap, and flatpak** backends. |
| **NFR-11** | Usability | Safe-by-default interaction. | Removal is initiable in **≤ 2 user actions** from a desktop entry; **no** destructive step executes before the Dry Run plan is presented (FR-19); risk labels use plain language with concrete Impact. |
| **NFR-12** | Maintainability | Backends are isolated behind the ACL. | Adding a new package backend touches **only** its adapter and registration — no change to Residue/Safety/Execution core logic; verified by a dependency test showing core modules import no backend symbols. |
| **NFR-13** | Observability | Progress and structured logs. | Job progress is queryable (FR-29) and the backend emits structured (machine-parseable) logs for every operation with operation id, target, verdict, and outcome. |
| **NFR-14** | Robustness | Graceful degradation for absent backends. | With one or more backends missing, the product enumerates/operates on the remaining backends without crashing (FR-1 AC2). |
| **NFR-15** | Data Integrity | Snapshots are integrity-verifiable. | Every Snapshot carries checksums; corruption is detected **before** restore; a corrupted-Snapshot test refuses Undo rather than restoring partial state. |

---

## 6. Traceability

### 6.1 Requirements → Originating Sub-Domains

| Sub-Domain (DOMAIN §2) | Originating FRs | NFRs |
|---|---|---|
| Residue Mapping | FR-8, FR-9, FR-10, FR-11, FR-12, FR-13 | NFR-2, NFR-4 |
| Removal Safety | FR-14, FR-15, FR-16, FR-17, FR-18, FR-19, FR-20, FR-21, FR-22 | NFR-3, NFR-4, NFR-11 |
| Removal Execution | FR-23, FR-24, FR-25, FR-26, FR-27, FR-28, FR-29, FR-36 | NFR-5, NFR-6, NFR-13, NFR-15 |
| Application Inventory | FR-1, FR-2 | NFR-1, NFR-14 |
| Provenance Detection | FR-4, FR-5, FR-6, FR-7 | — |
| Desktop Integration | FR-2, FR-3 | NFR-11 |
| Package Manager Integration (ACL) | FR-13, FR-30, FR-31 | NFR-12, NFR-14 |
| Privilege & OS Access | FR-32 | NFR-7 |
| Audit & Undo Store | FR-33, FR-34, FR-35 | NFR-8, NFR-9, NFR-15 |

### 6.2 Open Questions → Resolving Decisions → Requirements

| OQ (DOMAIN §6) | Decision | Realizing FR(s) / NFR(s) |
|---|---|---|
| OQ1 residue definition | D1 | FR-8, FR-9 |
| OQ2 breakage source of truth | D2 | FR-10, FR-15 |
| OQ3 cross-system collisions | D3 | FR-6, FR-7 |
| OQ4 discovery strategy | D4 | FR-12 |
| OQ5 privilege model | D5 | FR-32, NFR-7 |
| OQ6 undo completeness | D6 | FR-23, FR-34, NFR-6 |
| OQ7 Protected Apps | D7 | FR-20 |
| OQ8 user vs system scope | D8 | FR-22 |
| OQ9 non-packaged apps | D9 | FR-13 |
| OQ10 concurrency | D10 | FR-28 |
| OQ11 snapshot cost | D11 | FR-23, NFR-2 |
| OQ12 dry-run UX | D12 | FR-19 |
| OQ13 interaction surface | D13 | FR-36 |
| OQ14 confirmation/risk | D14 | FR-18 |

---

## 7. Requirements Coverage of the Core Domain Rules

Mapping the [Core Domain Rules (DOMAIN §5)](../00-domain/DOMAIN.md) to the requirements that enforce them —
so the reviewer can confirm every invariant is owned.

| Domain Rule | Enforced by |
|---|---|
| 1. Resolve before remove | FR-4, FR-5 |
| 2. Owner-aware removal | FR-10, FR-11, FR-15, NFR-4 |
| 3. Impact-gated approval | FR-15, FR-16, FR-18 |
| 4. Protected Apps | FR-20 |
| 5. Snapshot-before-execute | FR-23, FR-25, NFR-15 |
| 6. Fail-safe ordering | FR-17, FR-24, FR-25, FR-26, NFR-5 |
