//! Transport-agnostic service surface shared by HTTP and gRPC (plan 6).

pub mod auth;
pub mod grpc;
pub mod http;

use crate::model::{CheckRequest, CheckResponse, ErrorCode};
use std::time::Instant;

/// Runtime information mirrored into `GET /v1/info` and `TextGuard.GetInfo`.
#[derive(Debug, Clone)]
pub struct InfoResponse {
    pub instance_id: String,
    pub ready: bool,
    pub build_id: String,
    pub ruleset_revision: String,
    pub generation: String,
    pub normalization_id: String,
    pub t2s_table_id: String,
    pub base_rules_digest: String,
    pub canonical_count: u32,
    pub alias_count: u32,
    pub regex_count: u32,
    pub supported_semantics: Vec<String>,
}

/// The single core every transport talks to. Implementations must serve a
/// whole request from one frozen snapshot and must never turn an error
/// into an allow decision.
pub trait CheckCore: Send + Sync {
    fn check(&self, request: &CheckRequest) -> Result<CheckResponse, ErrorCode>;
    fn info(&self) -> InfoResponse;
    fn ready(&self) -> bool;

    /// Deadline for one check: admission + core scan included (plan 5).
    fn check_deadline(&self, requested: Option<Instant>) -> Instant {
        let default = Instant::now() + crate::runtime::CHECK_BUDGET;
        requested.unwrap_or(default).min(default)
    }
}
