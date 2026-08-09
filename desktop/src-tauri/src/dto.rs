//! Response DTOs for the Tauri commands (SPEC §7 response shapes, camelCase).

use serde::{Deserialize, Serialize};

use crate::domain::PackageBackend;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub backend: PackageBackend,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItem {
    pub canonical_app_id: String,
    pub name: String,
    pub install_methods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub icon: Option<String>,
    pub instance_count: u32,
    pub is_protected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryResponse {
    pub items: Vec<InventoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub backends: Vec<BackendStatus>,
    pub db_writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub audit_record_id: String,
    pub job_id: String,
    pub outcome: String,
    pub undoable: bool,
    pub created_at: i64,
    pub app_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryResponse {
    pub items: Vec<HistoryItem>,
}
