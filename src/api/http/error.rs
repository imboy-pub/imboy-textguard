//! Stable error envelope shared by every HTTP failure (plan 5).

use crate::model::ErrorCode;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// A transport-level failure: one error code plus its HTTP status.
pub(super) struct ApiError {
    code: ErrorCode,
}

impl ApiError {
    pub(super) fn new(code: ErrorCode) -> Self {
        Self { code }
    }

    #[cfg(test)]
    pub(super) fn code(&self) -> ErrorCode {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code,
                "message": message_for(self.code),
            }
        });
        (status_for(self.code), Json(body)).into_response()
    }
}

/// Frozen status table; see the module documentation in `super`.
/// NORMALIZATION_LIMIT and MATCH_BUDGET_EXCEEDED are input-driven limits
/// and map to 413, matching the gRPC RESOURCE_EXHAUSTED family (AC-058
/// cross-transport consistency).
pub(super) fn status_for(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::InvalidArgument | ErrorCode::InvalidUtf8 | ErrorCode::ProtocolError => {
            StatusCode::BAD_REQUEST
        }
        ErrorCode::TextTooLarge
        | ErrorCode::NormalizationLimit
        | ErrorCode::MatchBudgetExceeded => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::Overloaded | ErrorCode::NotReady => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::CheckDeadline => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn message_for(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidArgument => "request violates the check request contract",
        ErrorCode::InvalidUtf8 => "request body is not valid UTF-8",
        ErrorCode::TextTooLarge => "request body exceeds the accepted size limit",
        ErrorCode::Unauthenticated => "missing or unknown credential",
        ErrorCode::PermissionDenied => "credential is not authorized for this operation",
        ErrorCode::ProtocolError => "request violates the transport protocol contract",
        ErrorCode::Overloaded => "service is overloaded",
        ErrorCode::NotReady => "service is not ready",
        ErrorCode::CheckDeadline => "check deadline exceeded",
        ErrorCode::MatchBudgetExceeded => "match budget exceeded",
        ErrorCode::NormalizationLimit => "normalization limit exceeded",
        _ => "internal error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_table_is_frozen() {
        assert_eq!(
            status_for(ErrorCode::InvalidArgument),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(status_for(ErrorCode::InvalidUtf8), StatusCode::BAD_REQUEST);
        assert_eq!(
            status_for(ErrorCode::ProtocolError),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for(ErrorCode::TextTooLarge),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            status_for(ErrorCode::Unauthenticated),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_for(ErrorCode::PermissionDenied),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_for(ErrorCode::Overloaded),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_for(ErrorCode::NotReady),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            status_for(ErrorCode::CheckDeadline),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            status_for(ErrorCode::MatchBudgetExceeded),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            status_for(ErrorCode::NormalizationLimit),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            status_for(ErrorCode::InternalError),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
