//! Stable error envelope shared by every HTTP failure (plan 5).

use crate::model::ErrorCode;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// A transport-level failure: one error code plus its HTTP status.
pub(super) struct ApiError {
    code: ErrorCode,
    request_id: String,
}

impl ApiError {
    pub(super) fn new(code: ErrorCode) -> Self {
        Self {
            code,
            request_id: String::new(),
        }
    }

    pub(super) fn for_request(mut self, request_id: &str) -> Self {
        if crate::model::valid_identifier(request_id) {
            self.request_id = request_id.to_owned();
        }
        self
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
                "request_id": self.request_id,
                "retryable": retryable(self.code),
            }
        });
        (status_for(self.code), Json(body)).into_response()
    }
}

pub(super) fn retryable(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::Overloaded
            | ErrorCode::NotReady
            | ErrorCode::ConnectionFailed
            | ErrorCode::PublishBusy
            | ErrorCode::RetiredSnapshotBusy
    )
}

/// Frozen status table; see the module documentation in `super`.
pub(super) fn status_for(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::InvalidArgument | ErrorCode::InvalidUtf8 => StatusCode::BAD_REQUEST,
        ErrorCode::TextTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorCode::NormalizationLimit | ErrorCode::MatchBudgetExceeded => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::Overloaded => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::NotReady | ErrorCode::ConnectionFailed | ErrorCode::UnsupportedRuleset => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        ErrorCode::ProtocolError => StatusCode::BAD_GATEWAY,
        ErrorCode::CheckDeadline => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
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
            StatusCode::BAD_GATEWAY
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
            StatusCode::TOO_MANY_REQUESTS
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
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_for(ErrorCode::NormalizationLimit),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_for(ErrorCode::InternalError),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
