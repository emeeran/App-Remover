-- app-remover SQLite schema (SPEC §6). Idempotent (CREATE IF NOT EXISTS).
-- Enums are stored as TEXT with CHECK constraints; nested aggregates as JSON TEXT.
-- Timestamps are INTEGER epoch-millis. audit_records is append-only (no UPDATE/DELETE
-- is ever issued by the repository — enforced in code, NFR-9).

CREATE TABLE IF NOT EXISTS applications (
  canonical_app_id    TEXT PRIMARY KEY,
  name                TEXT NOT NULL,
  desktop_entry_path  TEXT,
  desktop_app_id      TEXT,
  is_protected        INTEGER NOT NULL DEFAULT 0 CHECK (is_protected IN (0,1)),
  disambiguated       INTEGER NOT NULL DEFAULT 0 CHECK (disambiguated IN (0,1)),
  first_seen_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS install_sources (
  install_source_id   INTEGER PRIMARY KEY AUTOINCREMENT,
  canonical_app_id    TEXT NOT NULL REFERENCES applications(canonical_app_id) ON DELETE RESTRICT,
  method              TEXT NOT NULL,
  confidence          TEXT NOT NULL CHECK (confidence IN ('high','medium','low')),
  evidence            TEXT NOT NULL,                       -- JSON Evidence[]
  created_at          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_install_sources_app ON install_sources(canonical_app_id);

CREATE TABLE IF NOT EXISTS package_instances (
  package_instance_id TEXT PRIMARY KEY,
  canonical_app_id    TEXT NOT NULL REFERENCES applications(canonical_app_id) ON DELETE RESTRICT,
  backend             TEXT NOT NULL,
  package_name        TEXT NOT NULL,
  version             TEXT,
  scope               TEXT CHECK (scope IN ('system','user')),
  UNIQUE (backend, package_name, scope)
);
CREATE INDEX IF NOT EXISTS ix_package_instances_app ON package_instances(canonical_app_id);

CREATE TABLE IF NOT EXISTS scans (
  canonical_app_id    TEXT NOT NULL REFERENCES applications(canonical_app_id) ON DELETE RESTRICT,
  scan_version        INTEGER NOT NULL,
  sealed_at           INTEGER NOT NULL,
  PRIMARY KEY (canonical_app_id, scan_version)
);

CREATE TABLE IF NOT EXISTS artifacts (
  artifact_id         TEXT PRIMARY KEY,
  canonical_app_id    TEXT NOT NULL REFERENCES applications(canonical_app_id) ON DELETE RESTRICT,
  scan_version        INTEGER NOT NULL,
  category            TEXT NOT NULL CHECK (category IN ('binary','config','cache','data','state','service','desktop-entry','association','dependency')),
  target              TEXT NOT NULL,
  owner_set           TEXT NOT NULL,                       -- JSON UUID[]
  usage_kind          TEXT NOT NULL CHECK (usage_kind IN ('exclusive','shared','system')),
  size_bytes          INTEGER,
  deletable           INTEGER NOT NULL CHECK (deletable IN (0,1)),
  confidence          TEXT NOT NULL CHECK (confidence IN ('high','medium','low')),
  discovered_by       TEXT NOT NULL CHECK (discovered_by IN ('manifest','knowledge-base','heuristic')),
  scope               TEXT CHECK (scope IN ('system','user'))
);
CREATE INDEX IF NOT EXISTS ix_artifacts_scan ON artifacts(canonical_app_id, scan_version);

CREATE TABLE IF NOT EXISTS plans (
  plan_id                 TEXT PRIMARY KEY,
  canonical_app_id        TEXT NOT NULL REFERENCES applications(canonical_app_id) ON DELETE RESTRICT,
  scan_version            INTEGER NOT NULL,
  mode                    TEXT NOT NULL CHECK (mode IN ('remove','purge')),
  scope                   TEXT NOT NULL CHECK (scope IN ('system-wide','current-user','both')),
  status                  TEXT NOT NULL CHECK (status IN ('draft','approved','superseded')),
  projected_snapshot_bytes INTEGER NOT NULL,
  exceeds_cost_cap        INTEGER NOT NULL CHECK (exceeds_cost_cap IN (0,1)),
  composed_at             INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS plan_operations (
  operation_id        TEXT PRIMARY KEY,
  plan_id             TEXT NOT NULL REFERENCES plans(plan_id) ON DELETE RESTRICT,
  "order"             INTEGER NOT NULL,
  action              TEXT NOT NULL CHECK (action IN ('uninstall-package','stop-service','delete-file','remove-association','prune-orphan')),
  target_kind         TEXT NOT NULL CHECK (target_kind IN ('artifact','package')),
  target_ref          TEXT NOT NULL,
  verdict             TEXT NOT NULL CHECK (verdict IN ('safe','risky','blocked','manual-review')),
  impact              TEXT NOT NULL,                       -- JSON UUID[]
  rationale           TEXT NOT NULL,
  accepted            INTEGER NOT NULL DEFAULT 0 CHECK (accepted IN (0,1))
);
CREATE INDEX IF NOT EXISTS ix_plan_ops_order ON plan_operations(plan_id, "order");

CREATE TABLE IF NOT EXISTS jobs (
  job_id          TEXT PRIMARY KEY,
  plan_id         TEXT NOT NULL REFERENCES plans(plan_id) ON DELETE RESTRICT,
  snapshot_id     TEXT,
  status          TEXT NOT NULL CHECK (status IN ('created','running','completed','failed','rolled-back')),
  created_at      INTEGER NOT NULL,
  started_at      INTEGER,
  finished_at     INTEGER,
  failure         TEXT                                     -- JSON ErrorDetail
);
CREATE INDEX IF NOT EXISTS ix_jobs_status ON jobs(status);

CREATE TABLE IF NOT EXISTS executed_steps (
  step_id         TEXT PRIMARY KEY,
  job_id          TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE RESTRICT,
  operation_id    TEXT NOT NULL,
  "order"         INTEGER NOT NULL,
  status          TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed','skipped')),
  started_at      INTEGER,
  finished_at     INTEGER,
  error           TEXT
);
CREATE INDEX IF NOT EXISTS ix_steps_job ON executed_steps(job_id, "order");

CREATE TABLE IF NOT EXISTS snapshots (
  snapshot_id        TEXT PRIMARY KEY,
  job_id             TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE RESTRICT,
  captured_at        INTEGER NOT NULL,
  checksum_algo      TEXT NOT NULL DEFAULT 'sha256',
  manifest_checksum  TEXT NOT NULL,
  total_bytes        INTEGER NOT NULL,
  excludes_cache     INTEGER NOT NULL DEFAULT 1 CHECK (excludes_cache IN (0,1)),
  blob_root          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshot_entries (
  entry_row_id    INTEGER PRIMARY KEY AUTOINCREMENT,
  snapshot_id     TEXT NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE RESTRICT,
  artifact_id     TEXT,
  category        TEXT NOT NULL,
  original_path   TEXT NOT NULL,
  blob_path       TEXT,
  checksum        TEXT NOT NULL,
  size_bytes      INTEGER NOT NULL,
  kind            TEXT NOT NULL CHECK (kind IN ('file-backup','package-record','unit-record')),
  package_name    TEXT
);
CREATE INDEX IF NOT EXISTS ix_snapshot_entries ON snapshot_entries(snapshot_id);

CREATE TABLE IF NOT EXISTS audit_records (
  audit_record_id   TEXT PRIMARY KEY,
  job_id            TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE RESTRICT,
  plan_snapshot     TEXT NOT NULL,    -- JSON RemovalPlan (immutable deep copy)
  steps             TEXT NOT NULL,    -- JSON ExecutedStep[]
  snapshot_id       TEXT NOT NULL,
  outcome           TEXT NOT NULL CHECK (outcome IN ('completed','rolled_back')),
  undoable          INTEGER NOT NULL CHECK (undoable IN (0,1)),
  created_at        INTEGER NOT NULL,
  hash              TEXT NOT NULL,    -- SHA256(prevHash || canonical(record))
  prev_hash         TEXT              -- NULL for genesis
);
CREATE INDEX IF NOT EXISTS ix_audit_job ON audit_records(job_id);
CREATE INDEX IF NOT EXISTS ix_audit_created ON audit_records(created_at);

CREATE TABLE IF NOT EXISTS event_log (
  event_id      INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id        TEXT,
  event_type    TEXT NOT NULL,
  payload       TEXT NOT NULL,        -- JSON
  created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_event_log_job ON event_log(job_id);
CREATE INDEX IF NOT EXISTS ix_event_log_type ON event_log(event_type);
