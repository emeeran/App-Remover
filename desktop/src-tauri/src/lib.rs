// Domain types/db are consumed incrementally across milestones M2–M5; allow
// dead_code during scaffold to keep the build signal clean.
#![allow(dead_code)]

mod audit;
mod backends;
mod commands;
mod config;
mod db;
mod desktop;
mod domain;
mod dto;
mod error;
mod exec;
mod executor;
mod policy;
mod planner;
mod residue;
mod runner;
mod scanner;
mod snapshot;

use std::path::PathBuf;

pub use db::Db;
pub use error::{AppError, AppResult};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Resolve the SQLite path under the user's XDG data dir:
/// `${XDG_DATA_HOME:-~/.local/share}/app-remover/app-remover.db`
fn db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("app-remover")
        .join("app-remover.db")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = Db::open(&db_path()).expect("failed to open app-remover database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::health,
            commands::list_backends,
            commands::list_inventory,
            commands::get_app,
            commands::resolve_app,
            commands::create_scan,
            commands::get_scan,
            commands::create_plan,
            commands::get_plan,
            commands::approve_plan,
            commands::create_job,
            commands::begin_job,
            commands::get_job,
            commands::undo_removal,
            commands::get_history,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
