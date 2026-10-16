//! Credential verification shared by both transports (plan 6.1).
//!
//! Missing or malformed credentials are UNAUTHENTICATED; a valid check
//! credential presented to a publish operation is PERMISSION_DENIED. An
//! unconfigured service fails closed: every request is rejected.

use crate::model::ErrorCode;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Check,
    Publish,
}

#[derive(Clone)]
pub struct AuthTokens {
    check: Arc<str>,
    publish: Arc<str>,
}

impl AuthTokens {
    /// Tokens must be non-empty; an empty configuration rejects everything.
    pub fn new(check: &str, publish: &str) -> Self {
        Self {
            check: Arc::from(check),
            publish: Arc::from(publish),
        }
    }

    pub fn configured(&self, scope: Scope) -> bool {
        !self.select(scope).is_empty()
    }

    fn select(&self, scope: Scope) -> &str {
        match scope {
            Scope::Check => &self.check,
            Scope::Publish => &self.publish,
        }
    }

    /// Constant-time comparison of two non-empty secrets.
    fn same(left: &str, right: &str) -> bool {
        if left.is_empty() || right.is_empty() {
            return false;
        }
        let left = left.as_bytes();
        let right = right.as_bytes();
        if left.len() != right.len() {
            return false;
        }
        left.iter()
            .zip(right)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }

    pub fn verify(&self, scope: Scope, presented: Option<&str>) -> Result<(), ErrorCode> {
        let Some(presented) = presented else {
            return Err(ErrorCode::Unauthenticated);
        };
        if presented.is_empty() {
            return Err(ErrorCode::Unauthenticated);
        }
        if Self::same(self.select(scope), presented) {
            return Ok(());
        }
        // wrong value for this scope: either a foreign token (denied) or an
        // unknown token (unauthenticated)
        if (scope == Scope::Publish && Self::same(&self.check, presented))
            || (scope == Scope::Check && Self::same(&self.publish, presented))
        {
            return Err(ErrorCode::PermissionDenied);
        }
        Err(ErrorCode::Unauthenticated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_empty_credentials_are_unauthenticated() {
        let tokens = AuthTokens::new("check-secret", "publish-secret");
        assert_eq!(
            tokens.verify(Scope::Check, None),
            Err(ErrorCode::Unauthenticated)
        );
        assert_eq!(
            tokens.verify(Scope::Check, Some("")),
            Err(ErrorCode::Unauthenticated)
        );
    }

    #[test]
    fn wrong_value_and_wrong_scope_split_401_and_403() {
        let tokens = AuthTokens::new("check-secret", "publish-secret");
        assert!(tokens.verify(Scope::Check, Some("check-secret")).is_ok());
        assert_eq!(
            tokens.verify(Scope::Check, Some("publish-secret")),
            Err(ErrorCode::PermissionDenied)
        );
        assert_eq!(
            tokens.verify(Scope::Publish, Some("check-secret")),
            Err(ErrorCode::PermissionDenied)
        );
        assert_eq!(
            tokens.verify(Scope::Check, Some("garbage")),
            Err(ErrorCode::Unauthenticated)
        );
    }

    #[test]
    fn empty_configuration_fails_closed() {
        let tokens = AuthTokens::new("", "");
        assert!(!tokens.configured(Scope::Check));
        assert_eq!(
            tokens.verify(Scope::Check, Some("")),
            Err(ErrorCode::Unauthenticated)
        );
    }
}
