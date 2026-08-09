//! Error type for Tauri commands. Serializes to the SPEC §7.4 envelope:
//! `{ "error": { code, message, details } }` so the frontend (and any future
//! HTTP shim) sees an identical shape to the Node reference backend.

use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
pub enum AppError {
    Validation(String),
    NotFound(String),
    DisambiguationRequired {
        canonical_app_id: String,
        candidates: serde_json::Value,
    },
    ProtectedApp(String),
    PlanNotApproved(String),
    PlanHasBlockedOp(Vec<String>),
    RiskNotAccepted(Vec<String>),
    IllegalTransition(String),
    ConflictingJob(String),
    SnapshotCostCapExceeded {
        projected: u64,
        cap: u64,
    },
    SnapshotCorrupt(String),
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Validation(_) => "VALIDATION_ERROR",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::DisambiguationRequired { .. } => "DISAMBIGUATION_REQUIRED",
            AppError::ProtectedApp(_) => "PROTECTED_APP",
            AppError::PlanNotApproved(_) => "PLAN_NOT_APPROVED",
            AppError::PlanHasBlockedOp(_) => "PLAN_HAS_BLOCKED_OP",
            AppError::RiskNotAccepted(_) => "RISK_NOT_ACCEPTED",
            AppError::IllegalTransition(_) => "ILLEGAL_TRANSITION",
            AppError::ConflictingJob(_) => "CONFLICTING_JOB",
            AppError::SnapshotCostCapExceeded { .. } => "SNAPSHOT_COST_CAP_EXCEEDED",
            AppError::SnapshotCorrupt(_) => "SNAPSHOT_CORRUPT",
            AppError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub fn message(&self) -> String {
        match self {
            AppError::Validation(m) => format!("Request validation failed: {m}"),
            AppError::NotFound(m) => format!("Resource not found: {m}"),
            AppError::DisambiguationRequired { canonical_app_id, .. } => format!(
                "'{canonical_app_id}' has distinct instances that must be disambiguated"
            ),
            AppError::ProtectedApp(n) => format!("'{n}' is protected and cannot be removed"),
            AppError::PlanNotApproved(id) => format!("Plan {id} is not approved"),
            AppError::PlanHasBlockedOp(ops) => {
                format!("Plan has blocked operations: {}", ops.join(", "))
            }
            AppError::RiskNotAccepted(ops) => {
                format!("Risky operations not accepted: {}", ops.join(", "))
            }
            AppError::IllegalTransition(m) => format!("Illegal job transition: {m}"),
            AppError::ConflictingJob(id) => format!("A job is already in flight for {id}"),
            AppError::SnapshotCostCapExceeded { projected, cap } => {
                format!("Snapshot cost cap exceeded: {projected} > {cap} bytes")
            }
            AppError::SnapshotCorrupt(m) => format!("Snapshot corrupt: {m}"),
            AppError::Internal(m) => format!("Internal error: {m}"),
        }
    }

    fn details(&self) -> BTreeMap<&'static str, serde_json::Value> {
        let mut d = BTreeMap::new();
        match self {
            AppError::DisambiguationRequired {
                canonical_app_id,
                candidates,
            } => {
                d.insert("canonicalAppId", serde_json::json!(canonical_app_id));
                d.insert("candidates", candidates.clone());
            }
            AppError::SnapshotCostCapExceeded { projected, cap } => {
                d.insert("projectedBytes", serde_json::json!(projected));
                d.insert("capBytes", serde_json::json!(cap));
            }
            AppError::PlanHasBlockedOp(ops) | AppError::RiskNotAccepted(ops) => {
                d.insert("operationIds", serde_json::json!(ops));
            }
            _ => {}
        }
        d
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("AppError", 1)?;
        let inner = serde_json::json!({
            "code": self.code(),
            "message": self.message(),
            "details": self.details(),
        });
        st.serialize_field("error", &inner)?;
        st.end()
    }
}

// -------------------------- conversions --------------------------

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
impl From<uuid::Error> for AppError {
    fn from(e: uuid::Error) -> Self {
        AppError::Validation(format!("invalid id: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
