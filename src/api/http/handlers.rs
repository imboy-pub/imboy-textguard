//! Route handlers for the HTTP data plane.
//!
//! Handlers own transport concerns only: credential scope, encoding/size
//! bounds, strict parsing and shared runtime admission.

use super::error::ApiError;
use super::validate::parse_check_request;
use super::MAX_BODY_BYTES;
use crate::api::auth::{AuthTokens, Scope};
use crate::api::CheckCore;
use crate::model::ErrorCode;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

/// Message produced by the bounded body reader when the size cap is hit.
const LENGTH_LIMIT_MESSAGE: &str = "length limit exceeded";

#[derive(Clone)]
pub(super) struct HttpState {
    core: Arc<dyn CheckCore>,
    tokens: Arc<AuthTokens>,
    admission: crate::runtime::Admission,
    metrics: Arc<crate::observability::Metrics>,
}

impl HttpState {
    pub(super) fn new(
        core: Arc<dyn CheckCore>,
        tokens: Arc<AuthTokens>,
        admission: crate::runtime::Admission,
        metrics: Arc<crate::observability::Metrics>,
    ) -> Self {
        Self {
            core,
            tokens,
            admission,
            metrics,
        }
    }
}

pub(super) async fn check_handler(State(state): State<HttpState>, request: Request) -> Response {
    if let Err(error) = authorize(&state.tokens, request.headers()) {
        return failed(&state, error);
    }
    if let Err(error) = check_transport(request.headers()) {
        return failed(&state, error);
    }
    run_check(&state, request).await
}

async fn run_check(state: &HttpState, request: Request) -> Response {
    let reader = match state.admission.reader() {
        Ok(reader) => reader,
        Err(code) => return failed(state, ApiError::new(code)),
    };
    let bytes = match tokio::time::timeout(
        std::time::Duration::from_secs(1),
        read_bounded_body(request),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => return failed(state, error),
        Err(_) => return failed(state, ApiError::new(ErrorCode::CheckDeadline)),
    };
    drop(reader);
    let parsed = match parse_check_request(&bytes) {
        Ok(parsed) => parsed,
        Err(error) => return failed(state, error),
    };
    let request_id = parsed.request_id.clone();
    let parsed = Arc::new(parsed);
    let measured = parsed.clone();
    let started = std::time::Instant::now();
    let core = state.core.clone();
    let deadline = core.check_deadline(None);
    let result = state
        .admission
        .execute(deadline, move |token| {
            core.check_cancellable(&parsed, &|| token.is_cancelled())
        })
        .await;
    state.metrics.record_check(
        &measured,
        crate::runtime::Transport::Http,
        started.elapsed(),
        &result,
    );
    match result {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(code) => ApiError::new(code).for_request(&request_id).into_response(),
    }
}

fn failed(state: &HttpState, error: ApiError) -> Response {
    state
        .metrics
        .record_failure(crate::runtime::Transport::Http, error.code());
    error.into_response()
}

pub(super) async fn info_handler(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state.tokens, &headers) {
        return error.into_response();
    }
    let info = state.core.info();
    let mut body = json!({
        "instance_id": info.instance_id,
        "ready": info.ready,
        "build_id": info.build_id,
        "supported_semantics": info.supported_semantics,
    });
    if info.ready {
        body.as_object_mut().unwrap().extend(
            json!({
                "ruleset_revision": info.ruleset_revision,
                "generation": info.generation,
                "normalization_id": info.normalization_id,
                "t2s_table_id": info.t2s_table_id,
                "base_rules_digest": info.base_rules_digest,
                "canonical_count": info.canonical_count,
                "alias_count": info.alias_count,
                "regex_count": info.regex_count,
            })
            .as_object()
            .unwrap()
            .clone(),
        );
    }
    (StatusCode::OK, Json(body)).into_response()
}

pub(super) async fn live_handler() -> Response {
    (StatusCode::OK, Json(json!({"status": "live"}))).into_response()
}

pub(super) async fn ready_handler(State(state): State<HttpState>) -> Response {
    if state.core.ready() {
        (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not_ready"})),
        )
            .into_response()
    }
}

/// Verify exactly one check-scope credential; bare token or `Bearer <token>`.
fn authorize(tokens: &AuthTokens, headers: &HeaderMap) -> Result<(), ApiError> {
    let mut credentials = headers.get_all(header::AUTHORIZATION).iter();
    let presented = match (credentials.next(), credentials.next()) {
        (Some(single), None) => credential(single),
        _ => None,
    };
    tokens
        .verify(Scope::Check, presented)
        .map_err(ApiError::new)
}

fn credential(value: &HeaderValue) -> Option<&str> {
    let raw = value.to_str().ok()?;
    // scheme names are case-insensitive; exactly one space separates them
    if raw.len() >= 7 && raw.as_bytes()[..7].eq_ignore_ascii_case(b"bearer ") {
        Some(&raw[7..])
    } else {
        Some(raw)
    }
}

/// Reject unknown content encodings (never decompress) and oversize bodies
/// early via `Content-Length`.
fn check_transport(headers: &HeaderMap) -> Result<(), ApiError> {
    for value in headers.get_all(header::CONTENT_ENCODING).iter() {
        let encoding = value
            .to_str()
            .map_err(|_| ApiError::new(ErrorCode::ProtocolError))?;
        if !encoding.eq_ignore_ascii_case("identity") {
            return Err(ApiError::new(ErrorCode::ProtocolError));
        }
    }
    if let Some(length) = headers.get(header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .map_err(|_| ApiError::new(ErrorCode::ProtocolError))?;
        let length: u64 = length
            .parse()
            .map_err(|_| ApiError::new(ErrorCode::ProtocolError))?;
        if length > MAX_BODY_BYTES as u64 {
            return Err(ApiError::new(ErrorCode::TextTooLarge));
        }
    }
    Ok(())
}

async fn read_bounded_body(request: Request) -> Result<Bytes, ApiError> {
    let error = match axum::body::to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(bytes) => return Ok(bytes),
        Err(error) => error,
    };
    if error.to_string() == LENGTH_LIMIT_MESSAGE {
        Err(ApiError::new(ErrorCode::TextTooLarge))
    } else {
        Err(ApiError::new(ErrorCode::ProtocolError))
    }
}
