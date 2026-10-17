//! Authenticated management HTTP, using the frozen publication-state Schema.
//! ETag binds instance, generation and revision; every PUT supplies If-Match.
//! Publish work remains locked if the caller times out; query state before retrying.

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
    receiving: Arc<tokio::sync::Semaphore>,
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
        .with_state(ManageState {
            state,
            tokens,
            receiving: Arc::new(tokio::sync::Semaphore::new(1)),
        })
}

async fn get_active(State(state): State<ManageState>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state.tokens, &headers) {
        return error_response(error, false);
    }
    let (active, etag) = match state.state.active_state() {
        Ok(value) => value,
        Err(error) => return publish_error_response(&error, false),
    };
    let body = publication_body(
        state.state.instance_id(),
        active.as_ref().map_or(0, |value| value.generation()),
        active.as_ref().map_or("", |value| value.revision()),
        &etag,
        if active.is_some() {
            "active"
        } else {
            "uninitialized"
        },
        active.as_ref().map_or([0; 3], |value| value.scene_counts()),
    );
    with_etag(StatusCode::OK, &etag, body)
}

async fn put_active(State(state): State<ManageState>, request: Request) -> Response {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
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
    if if_match.is_none() {
        return error_response(ErrorCode::CasConflict, true);
    }
    let permit = match state.receiving.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return error_response(ErrorCode::PublishBusy, false),
    };
    let confirmed_empty = request
        .headers()
        .get("x-textguard-confirm-empty")
        .is_some_and(|value| value == "true");
    let bytes = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        to_bytes(request.into_body(), MAX_PACKAGE_BYTES),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            let code = if error.to_string() == LENGTH_LIMIT_MESSAGE {
                ErrorCode::TextTooLarge
            } else {
                ErrorCode::ProtocolError
            };
            return error_response(code, false);
        }
        Err(_) => return error_response(ErrorCode::PublishDeadline, false),
    };
    if serde_json::from_slice::<Value>(&bytes).is_ok_and(|value| {
        value
            .get("literals")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
            && value
                .get("regex_rules")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
    }) && !confirmed_empty
    {
        return error_response(ErrorCode::InvalidRuleset, false);
    }
    let publication = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.state.publish(&bytes, if_match.as_deref())
    });
    match tokio::time::timeout_at(deadline, publication).await {
        Ok(Ok(Ok(receipt))) => {
            let body = publication_body(
                &receipt.instance_id,
                receipt.generation,
                &receipt.revision,
                &receipt.state_etag,
                if receipt.already_active {
                    "already_active"
                } else {
                    "committed"
                },
                receipt.scene_counts,
            );
            with_etag(StatusCode::OK, &receipt.state_etag, body)
        }
        Ok(Ok(Err(error))) => publish_error_response(&error, false),
        Ok(Err(_)) => error_response(ErrorCode::InternalError, false),
        Err(_) => error_response(ErrorCode::PublishDeadline, false),
    }
}

fn publication_body(
    instance: &str,
    generation: u64,
    revision: &str,
    etag: &str,
    status: &str,
    counts: [usize; 3],
) -> Value {
    json!({"instance_id": instance, "ready": generation > 0, "generation": generation.to_string(),
        "ruleset_revision": revision, "state_etag": etag, "status": status,
        "scene_counts": {"channel_message": counts[0], "moment_post": counts[1], "profile_field": counts[2]}})
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
    let body = json!({
        "error": {
            "code": code,
            "request_id": "",
            "retryable": super::error::retryable(code),
        }
    });
    (status_for(code, precondition_missing), Json(body)).into_response()
}

fn error_response(code: ErrorCode, precondition_missing: bool) -> Response {
    let body = json!({
        "error": {
            "code": code,
            "request_id": "",
            "retryable": super::error::retryable(code),
        }
    });
    (status_for(code, precondition_missing), Json(body)).into_response()
}

/// Management-plane status table; see the module documentation.
fn status_for(code: ErrorCode, _precondition_missing: bool) -> StatusCode {
    match code {
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::TextTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::InvalidArgument
        | ErrorCode::InvalidUtf8
        | ErrorCode::ProtocolError
        | ErrorCode::InvalidRuleset
        | ErrorCode::UnsupportedRuleset => StatusCode::BAD_REQUEST,
        ErrorCode::NotReady => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::PublishBusy | ErrorCode::RetiredSnapshotBusy => StatusCode::CONFLICT,
        ErrorCode::CasConflict => StatusCode::PRECONDITION_FAILED,
        ErrorCode::PublishDeadline => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
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
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            status_for(ErrorCode::CasConflict, false),
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            status_for(ErrorCode::PublishBusy, false),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_for(ErrorCode::InvalidRuleset, false),
            StatusCode::BAD_REQUEST
        );
    }
}
