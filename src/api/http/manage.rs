//! Management (publish) plane: `GET`/`PUT /internal/v1/rulesets/active`
//! (T14, plan §9).
//!
//! File-location note: this file lives under `src/api/http/` per the T14
//! task map, but the T12 HTTP module (`api/http/mod.rs`) is frozen and
//! cannot declare new submodules, so it is compiled as
//! [`crate::rules::manage`] via an explicit `#[path]` include from
//! `src/rules/mod.rs`. It reuses [`crate::api::auth`] credentials and the
//! data-plane envelope shape, but keeps its own error mapping because
//! `api::http::error::ApiError` is private to the frozen T12 module.
//!
//! Credential: `authorization` with the **publish** scope (bare token or
//! `Bearer <token>`). A check-scope credential is PERMISSION_DENIED.
//!
//! `PUT` publishes a ruleset package (exact bytes are stored):
//!
//! * bodies over [`MAX_PACKAGE_BYTES`] (64 MiB) are rejected up front via
//!   `Content-Length` and again by a bounded body read; any
//!   `Content-Encoding` other than absent/`identity` is a protocol error
//! * `If-Match` carries the CAS token (the revision/ETag of the current
//!   active ruleset); it may be omitted only for the initial publish into
//!   a store with no active ruleset — an omitted header while a ruleset is
//!   active answers **428 Precondition Required**, a mismatched token
//!   **409 Conflict** (both carry the machine code `CAS_CONFLICT`); the
//!   token may be quoted or bare; `*` and duplicated headers are
//!   `INVALID_ARGUMENT`
//! * republishing identical bytes while already active returns
//!   `already_active = true` with the generation unchanged
//!
//! `GET` returns the active pointer: revision, generation, digest,
//! instance_id, canonical/alias/regex counts, timestamps and
//! `base_rules_digest`; the `ETag` response header is the (quoted)
//! revision on both endpoints.
//!
//! Status mapping (management plane):
//!
//! | Condition                              | Status |
//! |----------------------------------------|--------|
//! | `UNAUTHENTICATED`                      | 401    |
//! | `PERMISSION_DENIED`                    | 403    |
//! | `INVALID_ARGUMENT`, `INVALID_UTF8`, `PROTOCOL_ERROR`, `INVALID_RULESET`, `UNSUPPORTED_RULESET` | 400 |
//! | `TEXT_TOO_LARGE`                       | 413    |
//! | `NOT_READY`, `PUBLISH_BUSY`            | 503    |
//! | `CAS_CONFLICT` with no `If-Match`      | 428    |
//! | `CAS_CONFLICT` with a mismatched token | 409    |
//! | everything else (`INTERNAL_ERROR`, `PUBLISH_STATE_UNKNOWN`, ...) | 500 |
//!
//! A `PUBLISH_STATE_UNKNOWN` response means the manifest was renamed but
//! its directory fsync failed: the publication may or may not be durable
//! — re-check with `GET`; never assume "not published" (AC-068 semantics,
//! full injection in T22).

use crate::api::auth::{AuthTokens, Scope};
use crate::model::ErrorCode;
use crate::rules::{PublishError, RulesState, MAX_PACKAGE_BYTES};
use axum::body::to_bytes;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

/// Message produced by the bounded body reader when the size cap is hit.
const LENGTH_LIMIT_MESSAGE: &str = "length limit exceeded";

const ACTIVE_PATH: &str = "/internal/v1/rulesets/active";

#[derive(Clone)]
struct ManageState {
    state: Arc<RulesState>,
    tokens: Arc<AuthTokens>,
}

/// Build the management router for assembly by `main` / T19. Mount it on
/// the internal network only.
pub fn router(state: Arc<RulesState>, tokens: Arc<AuthTokens>) -> axum::Router {
    axum::Router::new()
        .route(
            ACTIVE_PATH,
            get(get_active)
                .put(put_active)
                .layer(DefaultBodyLimit::max(MAX_PACKAGE_BYTES)),
        )
        .with_state(ManageState { state, tokens })
}

async fn get_active(State(state): State<ManageState>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state.tokens, &headers) {
        return error_response(error, false);
    }
    let Some(active) = state.state.current() else {
        return error_response(ErrorCode::NotReady, false);
    };
    let body = json!({
        "ready": true,
        "schema_version": crate::rules::MANIFEST_SCHEMA_VERSION,
        "revision": active.revision(),
        "generation": active.generation(),
        "digest": active.digest(),
        "base_rules_digest": active.snapshot().digest(),
        "instance_id": state.state.instance_id(),
        "published_by": active.published_by(),
        "canonical_count": active.counts().canonical,
        "alias_count": active.counts().alias,
        "regex_count": active.counts().regex,
        "updated_at_unix": active.updated_at_unix(),
        "updated_at_rfc3339": active.updated_at_rfc3339(),
    });
    with_etag(StatusCode::OK, active.revision(), body)
}

async fn put_active(State(state): State<ManageState>, request: Request) -> Response {
    if let Err(error) = authorize(&state.tokens, request.headers()) {
        return error_response(error, false);
    }
    if let Err(error) = check_transport(request.headers()) {
        return error_response(error, false);
    }
    let if_match = match if_match_token(request.headers()) {
        Ok(if_match) => if_match,
        Err(error) => return error_response(error, false),
    };
    let bytes = match to_bytes(request.into_body(), MAX_PACKAGE_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let code = if error.to_string() == LENGTH_LIMIT_MESSAGE {
                ErrorCode::TextTooLarge
            } else {
                ErrorCode::ProtocolError
            };
            return error_response(code, false);
        }
    };
    match state.state.publish(&bytes, if_match.as_deref()) {
        Ok(receipt) => {
            let body = json!({
                "revision": receipt.revision,
                "generation": receipt.generation,
                "digest": receipt.digest,
                "instance_id": receipt.instance_id,
                "already_active": receipt.already_active,
            });
            with_etag(StatusCode::OK, &receipt.revision, body)
        }
        Err(error) => publish_error_response(&error, if_match.is_none()),
    }
}

/// Verify exactly one publish-scope credential; bare token or Bearer form.
fn authorize(tokens: &AuthTokens, headers: &HeaderMap) -> Result<(), ErrorCode> {
    let mut credentials = headers.get_all(header::AUTHORIZATION).iter();
    let presented = match (credentials.next(), credentials.next()) {
        (Some(single), None) => credential(single),
        _ => None,
    };
    tokens.verify(Scope::Publish, presented)
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
fn check_transport(headers: &HeaderMap) -> Result<(), ErrorCode> {
    for value in headers.get_all(header::CONTENT_ENCODING).iter() {
        let encoding = value.to_str().map_err(|_| ErrorCode::ProtocolError)?;
        if !encoding.eq_ignore_ascii_case("identity") {
            return Err(ErrorCode::ProtocolError);
        }
    }
    if let Some(length) = headers.get(header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .map_err(|_| ErrorCode::ProtocolError)?
            .parse::<u64>()
            .map_err(|_| ErrorCode::ProtocolError)?;
        if length > MAX_PACKAGE_BYTES as u64 {
            return Err(ErrorCode::TextTooLarge);
        }
    }
    Ok(())
}

/// Exactly one `If-Match` value, quoted or bare. `*` and empty tokens are
/// rejected: wildcard matching would bypass the CAS the plane exists to
/// enforce.
fn if_match_token(headers: &HeaderMap) -> Result<Option<String>, ErrorCode> {
    let mut values = headers.get_all(header::IF_MATCH).iter();
    let first = match (values.next(), values.next()) {
        (None, None) => return Ok(None),
        (Some(single), None) => single,
        _ => return Err(ErrorCode::InvalidArgument),
    };
    let raw = first.to_str().map_err(|_| ErrorCode::InvalidArgument)?;
    let token = raw
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(raw);
    if token.is_empty() || token == "*" {
        return Err(ErrorCode::InvalidArgument);
    }
    Ok(Some(token.to_owned()))
}

fn publish_error_response(error: &PublishError, precondition_missing: bool) -> Response {
    let code = error.error_code();
    let message = match error {
        PublishError::UnknownState(_) => {
            // honest guidance for the unknown-durability outcome
            "publication durability is unknown: the manifest was replaced but its fsync failed; re-check the active state with GET before retrying"
        }
        _ => message_for(code),
    };
    let body = json!({
        "error": {
            "code": code,
            "message": message,
        }
    });
    (status_for(code, precondition_missing), Json(body)).into_response()
}

fn error_response(code: ErrorCode, precondition_missing: bool) -> Response {
    let body = json!({
        "error": {
            "code": code,
            "message": message_for(code),
        }
    });
    (status_for(code, precondition_missing), Json(body)).into_response()
}

/// Management-plane status table; see the module documentation.
fn status_for(code: ErrorCode, precondition_missing: bool) -> StatusCode {
    match code {
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::TextTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::InvalidArgument
        | ErrorCode::InvalidUtf8
        | ErrorCode::ProtocolError
        | ErrorCode::InvalidRuleset
        | ErrorCode::UnsupportedRuleset => StatusCode::BAD_REQUEST,
        ErrorCode::NotReady | ErrorCode::PublishBusy => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::CasConflict if precondition_missing => StatusCode::PRECONDITION_REQUIRED,
        ErrorCode::CasConflict => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn message_for(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Unauthenticated => "missing or unknown credential",
        ErrorCode::PermissionDenied => "credential is not authorized for this operation",
        ErrorCode::ProtocolError => "request violates the transport protocol contract",
        ErrorCode::TextTooLarge => "request body exceeds the accepted size limit",
        ErrorCode::InvalidArgument => "request violates the publish request contract",
        ErrorCode::InvalidUtf8 => "request body is not valid UTF-8",
        ErrorCode::InvalidRuleset => "ruleset failed validation",
        ErrorCode::UnsupportedRuleset => "ruleset schema or table is not supported",
        ErrorCode::NotReady => "no active ruleset is loaded",
        ErrorCode::PublishBusy => "another publication is in progress",
        ErrorCode::CasConflict => "If-Match does not match the active ruleset revision",
        ErrorCode::PublishStateUnknown => "publication durability is unknown",
        ErrorCode::InternalError => "internal error",
        _ => "internal error",
    }
}

fn with_etag(status: StatusCode, revision: &str, body: Value) -> Response {
    let etag = HeaderValue::from_str(&format!("\"{revision}\""))
        .expect("hex revision is a valid header value");
    (status, [(header::ETAG, etag)], Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(
                axum::http::HeaderName::from_static(name),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn if_match_accepts_quoted_and_bare_tokens_only() {
        assert_eq!(
            if_match_token(&headers(&[("if-match", "\"abc\"")])).unwrap(),
            Some("abc".to_owned())
        );
        assert_eq!(
            if_match_token(&headers(&[("if-match", "abc")])).unwrap(),
            Some("abc".to_owned())
        );
        assert_eq!(if_match_token(&headers(&[])).unwrap(), None);
        for invalid in [
            headers(&[("if-match", "*")]),
            headers(&[("if-match", "\"\"")]),
            headers(&[("if-match", "abc"), ("if-match", "def")]),
        ] {
            assert!(if_match_token(&invalid).is_err());
        }
    }

    #[test]
    fn management_status_table_splits_cas_failure_kinds() {
        assert_eq!(
            status_for(ErrorCode::CasConflict, true),
            StatusCode::PRECONDITION_REQUIRED
        );
        assert_eq!(
            status_for(ErrorCode::CasConflict, false),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_for(ErrorCode::PublishBusy, false),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_for(ErrorCode::InvalidRuleset, false),
            StatusCode::BAD_REQUEST
        );
    }
}
