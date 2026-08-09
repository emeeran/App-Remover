//! Tauri commands — the frontend-facing API (SPEC §7 endpoints → invoke names).

use tauri::State;

use crate::backends;
use crate::db::Db;
use crate::desktop;
use crate::domain::{
    CanonicalApplication, DesktopEntryRef, InstallMethod, RemovalJob, RemovalMode, RemovalPlan,
    RemovalScope, ResolveSource, ResidueGraph, UndoResult,
};
use crate::dto::{BackendStatus, HealthResponse, HistoryResponse, InventoryItem, InventoryResponse};
use crate::error::{AppError, AppResult};
use crate::executor::SystemExecutor;
use crate::{audit, planner, runner, scanner};

#[tauri::command]
pub fn health(db: State<'_, Db>) -> AppResult<HealthResponse> {
    // The connection opened in WAL mode; a lock succeeding means it's usable.
    let db_writable = db.0.lock().map(|_| true).unwrap_or(false);
    Ok(HealthResponse {
        status: "ok".into(),
        backends: backends::detect_backends(),
        db_writable,
    })
}

#[tauri::command]
pub fn list_backends() -> AppResult<Vec<BackendStatus>> {
    Ok(backends::detect_backends())
}

#[tauri::command]
pub fn list_inventory(q: Option<String>) -> AppResult<InventoryResponse> {
    let mut items: Vec<InventoryItem> = backends::inventory()?.into_iter().map(app_to_item).collect();
    if let Some(ref query) = q {
        let ql = query.to_lowercase();
        items.retain(|i| i.name.to_lowercase().contains(&ql));
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(InventoryResponse { items })
}

#[tauri::command]
pub fn get_app(canonical_app_id: String) -> AppResult<CanonicalApplication> {
    // M2: re-derive from apt inventory. Persistence + O(1) lookup lands with scans.
    backends::inventory()?
        .into_iter()
        .find(|a| a.canonical_app_id == canonical_app_id)
        .ok_or_else(|| AppError::NotFound(format!("application {canonical_app_id}")))
}

fn method_str(m: &InstallMethod) -> String {
    serde_json::to_value(m)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn app_to_item(a: CanonicalApplication) -> InventoryItem {
    let version = a
        .package_instance_refs
        .iter()
        .find_map(|r| r.version.clone());
    let install_methods = a
        .install_sources
        .iter()
        .map(|s| method_str(&s.method))
        .collect();
    InventoryItem {
        canonical_app_id: a.canonical_app_id,
        name: a.name,
        install_methods,
        version,
        icon: a.desktop_entry.as_ref().and_then(|d| d.icon.clone()),
        instance_count: a.package_instance_refs.len() as u32,
        is_protected: a.is_protected,
    }
}

// ------------------------------- resolve -------------------------------

#[tauri::command]
pub fn resolve_app(source: ResolveSource) -> AppResult<CanonicalApplication> {
    match source {
        ResolveSource::DesktopEntry { path } => resolve_desktop(&path),
        ResolveSource::CanonicalId { canonical_app_id } => backends::inventory()?
            .into_iter()
            .find(|a| a.canonical_app_id == canonical_app_id)
            .ok_or_else(|| AppError::NotFound(format!("application {canonical_app_id}"))),
    }
}

/// Resolve a `.desktop` launcher to its owning package: parse Exec → locate
/// the binary on PATH → `dpkg -S` → package. Disambiguation across backends
/// is dormant until multiple adapters exist (M7).
fn resolve_desktop(path: &str) -> AppResult<CanonicalApplication> {
    let entry = desktop::parse(path)?;
    let exec = entry
        .exec
        .as_deref()
        .and_then(desktop::exec_binary)
        .ok_or_else(|| AppError::Validation(format!("desktop entry {path} has no Exec")))?;
    let bin_path = which_exec(&exec)
        .ok_or_else(|| AppError::NotFound(format!("binary '{exec}' not on PATH")))?;
    let pkg = backends::owning_package(&bin_path)
        .ok_or_else(|| AppError::NotFound(format!("no owning package for {bin_path}")))?;
    let mut app = backends::inventory()?
        .into_iter()
        .find(|a| a.name == pkg)
        .ok_or_else(|| AppError::NotFound(format!("package {pkg} not installed")))?;
    app.desktop_entry = Some(DesktopEntryRef {
        path: path.to_string(),
        app_id: entry.app_id,
        icon: entry.icon,
        mimetypes: Vec::new(),
    });
    Ok(app)
}

/// Resolve an Exec token (absolute path or bare name) to an absolute path.
fn which_exec(token: &str) -> Option<String> {
    if token.starts_with('/') {
        return Some(token.to_string());
    }
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = std::path::Path::new(dir).join(token);
        if candidate.is_file() {
            return candidate.to_str().map(str::to_owned);
        }
    }
    None
}

// -------------------------------- scan ---------------------------------

#[tauri::command]
pub fn create_scan(canonical_app_id: String, db: State<'_, Db>) -> AppResult<ResidueGraph> {
    scanner::create_scan(canonical_app_id, &db)
}

#[tauri::command]
pub fn get_scan(
    canonical_app_id: String,
    scan_version: u32,
    db: State<'_, Db>,
) -> AppResult<ResidueGraph> {
    scanner::get_scan(canonical_app_id, scan_version, &db)
}

// -------------------------------- plan --------------------------------

#[tauri::command]
pub fn create_plan(
    canonical_app_id: String,
    scan_version: u32,
    mode: Option<RemovalMode>,
    scope: Option<RemovalScope>,
    db: State<'_, Db>,
) -> AppResult<RemovalPlan> {
    let app = backends::inventory()?
        .into_iter()
        .find(|a| a.canonical_app_id == canonical_app_id)
        .ok_or_else(|| AppError::NotFound(format!("application {canonical_app_id}")))?;
    if app.is_protected {
        return Err(AppError::ProtectedApp(app.name.clone()));
    }
    let graph = scanner::get_scan(canonical_app_id, scan_version, &db)?;
    planner::compose(
        &app,
        &graph,
        mode.unwrap_or(RemovalMode::Remove),
        scope.unwrap_or(RemovalScope::SystemWide),
        &db,
    )
}

#[tauri::command]
pub fn get_plan(plan_id: String, db: State<'_, Db>) -> AppResult<RemovalPlan> {
    let conn = db.0.lock().unwrap();
    planner::load_plan(&conn, &plan_id)
}

#[tauri::command]
pub fn approve_plan(
    plan_id: String,
    accepted_risk_operation_ids: Vec<String>,
    db: State<'_, Db>,
) -> AppResult<RemovalPlan> {
    planner::approve(&plan_id, &accepted_risk_operation_ids, &db)
}

// -------------------------------- job --------------------------------

#[tauri::command]
pub fn create_job(
    plan_id: String,
    proceed_despite_cost_cap: Option<bool>,
    db: State<'_, Db>,
) -> AppResult<RemovalJob> {
    runner::create_job(&plan_id, proceed_despite_cost_cap.unwrap_or(false), &db)
}

#[tauri::command]
pub fn begin_job(job_id: String, db: State<'_, Db>) -> AppResult<RemovalJob> {
    runner::begin_job(&job_id, &SystemExecutor, &db)
}

#[tauri::command]
pub fn get_job(job_id: String, db: State<'_, Db>) -> AppResult<RemovalJob> {
    let conn = db.0.lock().unwrap();
    runner::load_job(&conn, &job_id)
}

// ------------------------------- undo -------------------------------

#[tauri::command]
pub fn undo_removal(audit_record_id: String, db: State<'_, Db>) -> AppResult<UndoResult> {
    runner::undo(&audit_record_id, &SystemExecutor, &db)
}

#[tauri::command]
pub fn get_history(db: State<'_, Db>) -> AppResult<HistoryResponse> {
    let conn = db.0.lock().unwrap();
    Ok(HistoryResponse {
        items: audit::recent(&conn, 50)?,
    })
}
