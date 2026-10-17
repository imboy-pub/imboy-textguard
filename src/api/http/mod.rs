//! HTTP data plane (plan V2.1 §6 + §5, frozen).
//!
//! Routes:
//!
//! - `POST /v1/check` — content check, requires a check-scope credential
//! - `GET /v1/info` — runtime snapshot info, requires a check-scope credential
//! - `GET /health/live`, `GET /health/ready` — no credential; these are meant
//!   for the local/monitoring network only
//!
//! Publish endpoints belong to T14 and are intentionally absent here.
//!
//! Credentials travel in the `authorization` header: either the bare token or
//! `Bearer <token>` (see [`crate::api::auth`]).
//!
//! Every failure uses one stable JSON shape:
//! `{"error": {"code": "SCREAMING_SNAKE", "request_id": "...", "retryable": false}}`. Success on
//! `/v1/check` is `200` with the `CheckResponse` JSON.
//!
//! HTTP status mapping:
//!
//! | Error code family                              | Status |
//! |------------------------------------------------|--------|
//! | `INVALID_ARGUMENT`, `INVALID_UTF8`             | 400    |
//! | `TEXT_TOO_LARGE`                               | 413    |
//! | `NORMALIZATION_LIMIT`, `MATCH_BUDGET_EXCEEDED` | 422    |
//! | `UNAUTHENTICATED`                              | 401    |
//! | `PERMISSION_DENIED`                            | 403    |
//! | `OVERLOADED`                                   | 429    |
//! | `NOT_READY`, `CONNECTION_FAILED`, `UNSUPPORTED_RULESET` | 503 |
//! | `PROTOCOL_ERROR`                               | 502    |
//! | `CHECK_DEADLINE`                               | 504    |
//! | every other code                               | 500    |
//!
//! Body limits (AC-054): bodies larger than [`MAX_BODY_BYTES`] are rejected
//! twice — an up-front `Content-Length` check, then a bounded read of the
//! actual body (which covers chunked requests).
//! `Content-Encoding` other than absent/`identity` is a protocol error and the
//! service never decompresses.
//!
//! Every parsed check uses the shared runtime admission and cancellation budget.

pub mod manage;

mod error;
mod handlers;
mod validate;

use crate::api::auth::AuthTokens;
use crate::api::CheckCore;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use std::sync::Arc;

/// Maximum accepted request body: 256 KiB. Larger bodies are `TEXT_TOO_LARGE`.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// Build the HTTP data-plane router for assembly by `main` / T19.
pub fn router(core: Arc<dyn CheckCore>, tokens: Arc<AuthTokens>) -> axum::Router {
    router_with_runtime(core, tokens, crate::runtime::Admission::default())
}

pub fn router_with_runtime(
    core: Arc<dyn CheckCore>,
    tokens: Arc<AuthTokens>,
    admission: crate::runtime::Admission,
) -> axum::Router {
    router_observed(
        core,
        tokens,
        admission,
        Arc::new(crate::observability::Metrics::default()),
    )
}

pub fn router_observed(
    core: Arc<dyn CheckCore>,
    tokens: Arc<AuthTokens>,
    admission: crate::runtime::Admission,
    metrics: Arc<crate::observability::Metrics>,
) -> axum::Router {
    axum::Router::new()
        .route(
            "/v1/check",
            post(handlers::check_handler).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/v1/info", get(handlers::info_handler))
        .route("/health/live", get(handlers::live_handler))
        .route("/health/ready", get(handlers::ready_handler))
        .with_state(handlers::HttpState::new(core, tokens, admission, metrics))
}
