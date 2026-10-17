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
//! `{"error": {"code": "SCREAMING_SNAKE", "message": "..."}}`. Success on
//! `/v1/check` is `200` with the `CheckResponse` JSON.
//!
//! HTTP status mapping:
//!
//! | Error code family                              | Status |
//! |------------------------------------------------|--------|
//! | `INVALID_ARGUMENT`, `INVALID_UTF8`, `PROTOCOL_ERROR` | 400 |
//! | `TEXT_TOO_LARGE`                               | 413    |
//! | `UNAUTHENTICATED`                              | 401    |
//! | `PERMISSION_DENIED`                            | 403    |
//! | `OVERLOADED`, `NOT_READY`                      | 503    |
//! | `CHECK_DEADLINE`                               | 504    |
//! | every other code                               | 500    |
//!
//! Body limits (AC-054): bodies larger than [`MAX_BODY_BYTES`] are rejected
//! twice — an up-front `Content-Length` check, then a bounded read of the
//! actual body (which covers chunked requests).
//! `Content-Encoding` other than absent/`identity` is a protocol error and the
//! service never decompresses.
//!
//! Wiring: handlers call [`crate::api::CheckCore::check`] directly. Runtime
//! admission (semaphores, deadlines) is T19's assembly job; this module only
//! owns transport parsing, size bounds and credential scope checks.

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
    axum::Router::new()
        .route(
            "/v1/check",
            post(handlers::check_handler).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route("/v1/info", get(handlers::info_handler))
        .route("/health/live", get(handlers::live_handler))
        .route("/health/ready", get(handlers::ready_handler))
        .with_state(handlers::HttpState::new(core, tokens))
}
