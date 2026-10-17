//! gRPC data plane (T13): the `imboy.textguard.v1.TextGuard` service.
//!
//! # Scope
//!
//! Implements `CheckText` and `GetInfo` on top of the transport-agnostic
//! [`crate::api::CheckCore`]. The standard `grpc.health.v1.Health` service is
//! **not** implemented here; it belongs to the T17/T19 server assembly and is
//! intentionally out of scope for this module.
//!
//! # Credentials
//!
//! Both RPCs require a check-scope credential in the `authorization` request
//! metadata key (lowercase, as HTTP/2 headers are lowercase). The raw token is
//! accepted, and a `Bearer <token>` prefix (ASCII case-insensitive) is stripped
//! for parity with the HTTP plane. Missing/wrong credentials surface as
//! `UNAUTHENTICATED`; a publish token presented to this check plane surfaces as
//! `PERMISSION_DENIED` (see [`crate::api::auth`]).
//!
//! # Presence (AC-056)
//!
//! Every proto3 `optional` request field is treated with explicit presence:
//! `request_id`, `scene`, `fields` (1..=8), and per-field `field_id`/`text`
//! that are `None` are `INVALID_ARGUMENT` — they are never defaulted. An
//! explicit empty-string `text` is valid and business-equivalent to the HTTP
//! plane. Duplicate `field_id` values are rejected. After conversion the
//! [`model::CheckRequest::validate`] guard also runs (belt and braces) and the
//! first failure is mapped through the error contract below.
//!
//! # Unknown enums (AC-057)
//!
//! Prost 0.13 decodes `optional` enum fields as raw `i32` without validating
//! the variant, so a wire value of `99` reaches the handler as `scene ==
//! Some(99)`. The handler therefore validates the raw value itself: anything
//! outside `1..=3` (including `SCENE_UNSPECIFIED = 0`) is `INVALID_ARGUMENT`.
//! Unknown *field numbers* are skipped by the protobuf decoder, which is the
//! correct forwards-compatible behavior and is not an error. Response mapping
//! sets every response field — including `scan_complete` and `decision` — so a
//! decoded response never relies on proto3 defaults.
//!
//! # Error contract (AC-058)
//!
//! Every [`model::ErrorCode`] carries its `SCREAMING_SNAKE` code string both in
//! the tonic status message and in the `x-textguard-error-code` response
//! metadata key, so an Erlang client can tell e.g. `OVERLOADED` apart from
//! `TEXT_TOO_LARGE` even though both map to `RESOURCE_EXHAUSTED`:
//!
//! | model::ErrorCode | tonic Code |
//! |---|---|
//! | `InvalidArgument`, `InvalidUtf8` | `INVALID_ARGUMENT` |
//! | `ProtocolError` | `DATA_LOSS` |
//! | `TextTooLarge`, `NormalizationLimit`, `MatchBudgetExceeded`, `Overloaded` | `RESOURCE_EXHAUSTED` |
//! | `Unauthenticated` | `UNAUTHENTICATED` |
//! | `PermissionDenied` | `PERMISSION_DENIED` |
//! | `NotReady`, `ConnectionFailed` | `UNAVAILABLE` |
//! | `CheckDeadline` | `DEADLINE_EXCEEDED` |
//! | `InternalError`, `NormalizerIncompatible` | `INTERNAL` |
//! | `UnsupportedRuleset`, `CasConflict`, `InvalidRuleset`, `PublishDeadline`, `PublishStateUnknown`, `PublishBusy`, `RetiredSnapshotBusy` | `FAILED_PRECONDITION` |
//!
//! The publish-family codes describe an operator-resolvable state machine
//! (stale ruleset/CAS or in-flight publish), which is why they map to
//! `FAILED_PRECONDITION` rather than `INTERNAL`.
//!
//! # Limits and deadline (AC-059)
//!
//! [`service_with_limits`] is the single wiring used by tests, `main` and the
//! T19 assembly: 256 KiB `max_decoding_message_size`/`max_encoding_message_size`.
//! A larger message is rejected by the tonic framing layer before any handler
//! runs with `OUT_OF_RANGE` ("decoded message length too large ...").
//!
//! The `grpc-timeout` request metadata is parsed per the gRPC spec
//! (`1..=8` ASCII digits plus `H`/`M`/`S`/`m`/`u`/`n`; values beyond 24h are
//! clamped to keep `Instant` arithmetic overflow-free; unparseable values are
//! ignored, matching tonic's own client behavior). The effective deadline is
//! `core.check_deadline(requested)`, i.e. never longer than the requested one
//! and bounded by the core's check budget. `core.check` runs on
//! `tokio::task::spawn_blocking` and is raced against
//! `tokio::time::timeout_at(deadline)`, so the RPC aborts with
//! `DEADLINE_EXCEEDED` (`CHECK_DEADLINE`) while the core keeps working.
//!
//! # Compression
//!
//! No compression feature is compiled into tonic (the crate pins
//! `tonic = "=0.12.3"` without `gzip`/`zstd`). A request with
//! `grpc-encoding: gzip` is therefore rejected by the tonic codec before any
//! handler runs with `UNIMPLEMENTED` ("Content is compressed with `gzip`
//! which isn't supported") and a `grpc-accept-encoding: identity` response
//! metadata; nothing is ever decompressed.
//!
//! [model::CheckRequest::validate]: crate::model::CheckRequest::validate

use crate::api::auth::{AuthTokens, Scope};
use crate::api::{CheckCore, InfoResponse};
use crate::model::{self, ErrorCode};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tonic::metadata::MetadataMap;
use tonic::{Code, Request, Response, Status};

/// Maximum decoded/encoded gRPC message size applied by [`service_with_limits`].
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Metadata key carrying the `SCREAMING_SNAKE` error code on every error status.
pub const ERROR_CODE_METADATA: &str = "x-textguard-error-code";
pub const REQUEST_ID_METADATA: &str = "x-textguard-request-id";

/// gRPC deadline values are clamped to this so `Instant + Duration` cannot overflow.
const MAX_GRPC_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// The `imboy.textguard.v1.TextGuard` service over one [`CheckCore`].
pub struct TextGuardSvc {
    core: Arc<dyn CheckCore>,
    tokens: Arc<AuthTokens>,
}

impl TextGuardSvc {
    pub fn new(core: Arc<dyn CheckCore>, tokens: Arc<AuthTokens>) -> Self {
        Self { core, tokens }
    }

    /// Returns the small [`ErrorCode`] so call sites map it through
    /// [`status_from_error`] themselves (keeps `Status` out of the hot path).
    fn authorize(&self, metadata: &MetadataMap) -> Result<(), ErrorCode> {
        let mut presented = metadata
            .get_all("authorization")
            .iter()
            .filter_map(|value| value.to_str().ok().map(strip_bearer));
        let first = presented.next();
        if presented.next().is_some() {
            // parity with the HTTP plane: repeated credentials are treated
            // as absent rather than first-wins
            return Err(ErrorCode::Unauthenticated);
        }
        self.tokens.verify(Scope::Check, first)
    }
}

/// Shared wiring for `main`, the T19 assembly and the contract tests: the
/// service plus the frozen 256 KiB message limits.
pub fn service_with_limits(
    core: Arc<dyn CheckCore>,
    tokens: Arc<AuthTokens>,
) -> crate::protocol::text_guard_server::TextGuardServer<TextGuardSvc> {
    crate::protocol::text_guard_server::TextGuardServer::new(TextGuardSvc::new(core, tokens))
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
}

#[tonic::async_trait]
impl crate::protocol::text_guard_server::TextGuard for TextGuardSvc {
    async fn check_text(
        &self,
        request: Request<crate::protocol::CheckTextRequest>,
    ) -> Result<Response<crate::protocol::CheckTextResponse>, Status> {
        self.authorize(request.metadata())
            .map_err(status_from_error)?;
        let requested = grpc_timeout_deadline(request.metadata());
        let request_id = request
            .get_ref()
            .request_id
            .as_deref()
            .filter(|value| crate::model::valid_identifier(value))
            .unwrap_or("")
            .to_owned();
        let core_request = to_model_request(request.into_inner())
            .map_err(|code| status_for_request(code, &request_id))?;
        let deadline = self.core.check_deadline(requested);
        let core = Arc::clone(&self.core);
        let task = tokio::task::spawn_blocking(move || core.check(&core_request));
        match tokio::time::timeout_at(deadline.into(), task).await {
            Ok(Ok(Ok(response))) => Ok(Response::new(to_proto_response(response))),
            Ok(Ok(Err(code))) => Err(status_for_request(code, &request_id)),
            // the blocking task panicked: the core never produced a decision
            Ok(Err(_join_error)) => Err(status_for_request(ErrorCode::InternalError, &request_id)),
            Err(_elapsed) => Err(status_for_request(ErrorCode::CheckDeadline, &request_id)),
        }
    }

    async fn get_info(
        &self,
        request: Request<crate::protocol::GetInfoRequest>,
    ) -> Result<Response<crate::protocol::GetInfoResponse>, Status> {
        self.authorize(request.metadata())
            .map_err(status_from_error)?;
        Ok(Response::new(to_proto_info(self.core.info())))
    }
}

/// Strip an ASCII case-insensitive `Bearer ` prefix; everything else is the raw token.
fn strip_bearer(value: &str) -> &str {
    if value.len() >= 7 && value[..7].eq_ignore_ascii_case("bearer ") {
        &value[7..]
    } else {
        value
    }
}

/// Convert a proto request into a model request, enforcing presence first.
fn to_model_request(
    proto: crate::protocol::CheckTextRequest,
) -> Result<model::CheckRequest, ErrorCode> {
    let request_id = proto.request_id.ok_or(ErrorCode::InvalidArgument)?;
    let scene = match proto.scene.ok_or(ErrorCode::InvalidArgument)? {
        1 => model::Scene::ChannelMessage,
        2 => model::Scene::MomentPost,
        3 => model::Scene::ProfileField,
        // 0 (SCENE_UNSPECIFIED), out-of-range and negative wire values
        _ => return Err(ErrorCode::InvalidArgument),
    };
    if proto.fields.is_empty() || proto.fields.len() > 8 {
        return Err(ErrorCode::InvalidArgument);
    }
    let mut identities = HashSet::with_capacity(proto.fields.len());
    let mut fields = Vec::with_capacity(proto.fields.len());
    for field in proto.fields {
        let field_id = field.field_id.ok_or(ErrorCode::InvalidArgument)?;
        let text = field.text.ok_or(ErrorCode::InvalidArgument)?;
        if !identities.insert(field_id.clone()) {
            return Err(ErrorCode::InvalidArgument);
        }
        fields.push(model::TextField { field_id, text });
    }
    let request = model::CheckRequest {
        request_id,
        scene,
        fields,
    };
    request.validate()?;
    Ok(request)
}

fn to_proto_response(response: model::CheckResponse) -> crate::protocol::CheckTextResponse {
    crate::protocol::CheckTextResponse {
        request_id: Some(response.request_id),
        ruleset_revision: Some(response.ruleset_revision),
        generation: Some(response.generation),
        normalization_id: Some(response.normalization_id),
        t2s_table_id: Some(response.t2s_table_id),
        semantics_id: Some(response.semantics_id),
        base_rules_digest: Some(response.base_rules_digest),
        decision: Some(proto_decision(response.decision) as i32),
        scan_complete: Some(response.scan_complete),
        hits_truncated: Some(response.hits_truncated),
        hits: response.hits.into_iter().map(to_proto_hit).collect(),
        fields_checked: Some(response.fields_checked),
        hit_semantics: Some(response.hit_semantics),
        build_id: Some(response.build_id),
    }
}

fn to_proto_hit(hit: model::Hit) -> crate::protocol::Hit {
    crate::protocol::Hit {
        field_id: Some(hit.field_id),
        rule_id: Some(hit.rule_id),
        variant_id: Some(hit.variant_id),
        variant_type: Some(proto_variant_type(hit.variant_type) as i32),
        word: Some(hit.word),
        category: Some(hit.category),
        severity: Some(proto_severity(hit.severity) as i32),
        effective_action: Some(proto_decision(hit.effective_action.into()) as i32),
        start_byte: Some(hit.start_byte),
        end_byte: Some(hit.end_byte),
        match_methods: hit
            .match_methods
            .into_iter()
            .map(|method| proto_match_method(method) as i32)
            .collect(),
    }
}

fn to_proto_info(info: InfoResponse) -> crate::protocol::GetInfoResponse {
    crate::protocol::GetInfoResponse {
        instance_id: Some(info.instance_id),
        ready: Some(info.ready),
        build_id: Some(info.build_id),
        ruleset_revision: info.ready.then_some(info.ruleset_revision),
        generation: info.ready.then_some(info.generation),
        normalization_id: info.ready.then_some(info.normalization_id),
        t2s_table_id: info.ready.then_some(info.t2s_table_id),
        base_rules_digest: info.ready.then_some(info.base_rules_digest),
        canonical_count: info.ready.then_some(info.canonical_count),
        alias_count: info.ready.then_some(info.alias_count),
        regex_count: info.ready.then_some(info.regex_count),
        supported_semantics: info.supported_semantics,
    }
}

fn proto_decision(action: model::Action) -> crate::protocol::Decision {
    match action {
        model::Action::Allow => crate::protocol::Decision::Allow,
        model::Action::Review => crate::protocol::Decision::Review,
        model::Action::Block => crate::protocol::Decision::Block,
    }
}

fn proto_severity(severity: model::Severity) -> crate::protocol::Severity {
    match severity {
        model::Severity::Low => crate::protocol::Severity::Low,
        model::Severity::Medium => crate::protocol::Severity::Medium,
        model::Severity::High => crate::protocol::Severity::High,
    }
}

fn proto_variant_type(variant: model::VariantType) -> crate::protocol::VariantType {
    match variant {
        model::VariantType::Canonical => crate::protocol::VariantType::Canonical,
        model::VariantType::Pinyin => crate::protocol::VariantType::Pinyin,
        model::VariantType::Homophone => crate::protocol::VariantType::Homophone,
        model::VariantType::Lookalike => crate::protocol::VariantType::Lookalike,
        model::VariantType::Decomposed => crate::protocol::VariantType::Decomposed,
        model::VariantType::RegexRule => crate::protocol::VariantType::RegexRule,
    }
}

fn proto_match_method(method: model::MatchMethod) -> crate::protocol::MatchMethod {
    match method {
        model::MatchMethod::Literal => crate::protocol::MatchMethod::Literal,
        model::MatchMethod::T2s => crate::protocol::MatchMethod::T2s,
        model::MatchMethod::Gap => crate::protocol::MatchMethod::Gap,
        model::MatchMethod::Alias => crate::protocol::MatchMethod::Alias,
        model::MatchMethod::Regex => crate::protocol::MatchMethod::Regex,
    }
}

/// The `SCREAMING_SNAKE` wire name of an error code, shared by the status
/// message and the `x-textguard-error-code` metadata.
fn error_code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
        ErrorCode::InvalidUtf8 => "INVALID_UTF8",
        ErrorCode::ProtocolError => "PROTOCOL_ERROR",
        ErrorCode::TextTooLarge => "TEXT_TOO_LARGE",
        ErrorCode::NormalizationLimit => "NORMALIZATION_LIMIT",
        ErrorCode::MatchBudgetExceeded => "MATCH_BUDGET_EXCEEDED",
        ErrorCode::Overloaded => "OVERLOADED",
        ErrorCode::NotReady => "NOT_READY",
        ErrorCode::ConnectionFailed => "CONNECTION_FAILED",
        ErrorCode::CheckDeadline => "CHECK_DEADLINE",
        ErrorCode::Unauthenticated => "UNAUTHENTICATED",
        ErrorCode::PermissionDenied => "PERMISSION_DENIED",
        ErrorCode::NormalizerIncompatible => "NORMALIZER_INCOMPATIBLE",
        ErrorCode::InternalError => "INTERNAL_ERROR",
        ErrorCode::UnsupportedRuleset => "UNSUPPORTED_RULESET",
        ErrorCode::CasConflict => "CAS_CONFLICT",
        ErrorCode::InvalidRuleset => "INVALID_RULESET",
        ErrorCode::PublishDeadline => "PUBLISH_DEADLINE",
        ErrorCode::PublishStateUnknown => "PUBLISH_STATE_UNKNOWN",
        ErrorCode::PublishBusy => "PUBLISH_BUSY",
        ErrorCode::RetiredSnapshotBusy => "RETIRED_SNAPSHOT_BUSY",
    }
}

/// The full AC-058 mapping; see the module doc table for the rationale.
fn status_from_error(code: ErrorCode) -> Status {
    status_for_request(code, "")
}

fn status_for_request(code: ErrorCode, request_id: &str) -> Status {
    let name = error_code_name(code);
    let grpc_code = match code {
        ErrorCode::InvalidArgument | ErrorCode::InvalidUtf8 => Code::InvalidArgument,
        ErrorCode::ProtocolError => Code::DataLoss,
        ErrorCode::TextTooLarge
        | ErrorCode::NormalizationLimit
        | ErrorCode::MatchBudgetExceeded
        | ErrorCode::Overloaded => Code::ResourceExhausted,
        ErrorCode::Unauthenticated => Code::Unauthenticated,
        ErrorCode::PermissionDenied => Code::PermissionDenied,
        ErrorCode::NotReady | ErrorCode::ConnectionFailed => Code::Unavailable,
        ErrorCode::CheckDeadline => Code::DeadlineExceeded,
        ErrorCode::InternalError | ErrorCode::NormalizerIncompatible => Code::Internal,
        ErrorCode::UnsupportedRuleset
        | ErrorCode::CasConflict
        | ErrorCode::InvalidRuleset
        | ErrorCode::PublishDeadline
        | ErrorCode::PublishStateUnknown
        | ErrorCode::PublishBusy
        | ErrorCode::RetiredSnapshotBusy => Code::FailedPrecondition,
    };
    let mut metadata = MetadataMap::new();
    metadata.insert(
        ERROR_CODE_METADATA,
        name.parse().expect("valid header value"),
    );
    metadata.insert(
        REQUEST_ID_METADATA,
        request_id.parse().expect("validated request identity"),
    );
    Status::with_metadata(grpc_code, name, metadata)
}

/// Parse a `grpc-timeout` value per the gRPC HTTP/2 spec: 1..=8 ASCII digits
/// followed by `H`, `M`, `S`, `m`, `u` or `n`. Unparseable values yield `None`
/// (treated as absent, like tonic's own transport), and the duration is
/// clamped to 24h to keep `Instant` arithmetic overflow-free.
fn parse_grpc_timeout(value: &str) -> Option<Duration> {
    let (digits, unit) = value.split_at(value.len().checked_sub(1)?);
    if digits.is_empty() || digits.len() > 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let amount: u64 = digits.parse().ok()?;
    let duration = match unit {
        "H" => Duration::from_secs(amount.checked_mul(60 * 60)?),
        "M" => Duration::from_secs(amount.checked_mul(60)?),
        "S" => Duration::from_secs(amount),
        "m" => Duration::from_millis(amount),
        "u" => Duration::from_micros(amount),
        "n" => Duration::from_nanos(amount),
        _ => return None,
    };
    Some(duration.min(MAX_GRPC_TIMEOUT))
}

fn grpc_timeout_deadline(metadata: &MetadataMap) -> Option<Instant> {
    let value = metadata.get("grpc-timeout")?.to_str().ok()?;
    parse_grpc_timeout(value).map(|duration| Instant::now() + duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_timeout_formats() {
        assert_eq!(
            parse_grpc_timeout("3H"),
            Some(Duration::from_secs(3 * 3600))
        );
        assert_eq!(parse_grpc_timeout("42S"), Some(Duration::from_secs(42)));
        assert_eq!(parse_grpc_timeout("150m"), Some(Duration::from_millis(150)));
        assert_eq!(parse_grpc_timeout("2u"), Some(Duration::from_micros(2)));
        assert_eq!(parse_grpc_timeout("82n"), Some(Duration::from_nanos(82)));
        // spec violations are ignored, not fatal
        for invalid in ["", "H", "oneH", "82f", "123456789H", "1.5S"] {
            assert_eq!(parse_grpc_timeout(invalid), None, "{invalid:?}");
        }
        // 24h clamp keeps Instant arithmetic safe
        assert_eq!(
            parse_grpc_timeout("99999999H"),
            Some(Duration::from_secs(24 * 3600))
        );
    }

    #[test]
    fn bearer_prefix_is_case_insensitive() {
        assert_eq!(strip_bearer("tok"), "tok");
        assert_eq!(strip_bearer("Bearer tok"), "tok");
        assert_eq!(strip_bearer("bearer tok"), "tok");
        assert_eq!(strip_bearer("BEARER tok"), "tok");
        assert_eq!(strip_bearer("Bearer "), "");
    }

    #[test]
    fn error_names_cover_every_code() {
        let codes = [
            ErrorCode::InvalidArgument,
            ErrorCode::InvalidUtf8,
            ErrorCode::ProtocolError,
            ErrorCode::TextTooLarge,
            ErrorCode::NormalizationLimit,
            ErrorCode::MatchBudgetExceeded,
            ErrorCode::Overloaded,
            ErrorCode::NotReady,
            ErrorCode::ConnectionFailed,
            ErrorCode::CheckDeadline,
            ErrorCode::Unauthenticated,
            ErrorCode::PermissionDenied,
            ErrorCode::NormalizerIncompatible,
            ErrorCode::InternalError,
            ErrorCode::UnsupportedRuleset,
            ErrorCode::CasConflict,
            ErrorCode::InvalidRuleset,
            ErrorCode::PublishDeadline,
            ErrorCode::PublishStateUnknown,
            ErrorCode::PublishBusy,
            ErrorCode::RetiredSnapshotBusy,
        ];
        for code in codes {
            let status = status_from_error(code);
            assert_eq!(status.message(), error_code_name(code));
            assert_eq!(
                status.metadata().get(ERROR_CODE_METADATA).unwrap(),
                error_code_name(code)
            );
        }
    }
}
