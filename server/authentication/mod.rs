/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use std::sync::Arc;

use axum::RequestPartsExt;
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use error::typedb_error;
use http::Extensions;
use tonic::metadata::MetadataMap;

use crate::state::BoxServerState;

pub(crate) mod credential_verifier;
pub(crate) mod token_manager;

pub const HTTP_AUTHORIZATION_FIELD: &str = "authorization";
pub const HTTP_BEARER_PREFIX: &str = "Bearer ";

pub(crate) async fn extract_parts_authorization_token(mut parts: http::request::Parts) -> Option<String> {
    parts
        .extract::<TypedHeader<Authorization<Bearer>>>()
        .await
        .map(|header| {
            let TypedHeader(Authorization(bearer)) = header;
            bearer.token().to_string()
        })
        .ok()
}

pub(crate) fn extract_metadata_authorization_token(metadata: &MetadataMap) -> Option<String> {
    let Some(Ok(authorization)) = metadata.get(HTTP_AUTHORIZATION_FIELD).map(|value| value.to_str()) else {
        return None;
    };
    authorization.strip_prefix(HTTP_BEARER_PREFIX).map(|token| token.to_string())
}

pub(crate) fn extract_metadata_accessor(metadata: &MetadataMap) -> Option<String> {
    let Some(Ok(authorization)) = metadata.get(HTTP_AUTHORIZATION_FIELD).map(|value| value.to_str()) else {
        return None;
    };
    authorization.strip_prefix(HTTP_BEARER_PREFIX).map(|token| token.to_string())
}

pub(crate) async fn authenticate<T>(
    server_state: Arc<BoxServerState>,
    request: http::Request<T>,
) -> Result<http::Request<T>, AuthenticationError> {
    let (mut parts, body) = request.into_parts();

    match extract_parts_authorization_token(parts.clone()).await {
        Some(token) => {
            let accessor = server_state.token_get_owner(&token).await.ok_or(AuthenticationError::InvalidToken {})?;
            parts.extensions.insert(Accessor(accessor));
            Ok(http::Request::from_parts(parts, body))
        }
        None => Err(AuthenticationError::MissingToken {}),
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct Accessor(pub String);

impl Accessor {
    pub fn from_extensions(extensions: &Extensions) -> Result<Self, AuthenticationError> {
        extensions.get::<Self>().cloned().ok_or_else(|| AuthenticationError::CorruptedAccessor {})
    }
}

// CAREFUL: Do not reorder these errors as we depend on errors codes in drivers.
typedb_error! {
    pub AuthenticationError(component = "Authentication", prefix = "AUT") {
        InvalidCredential(1, "Invalid credential supplied."),
        MissingToken(2, "Missing token (expected as the authorization bearer)."),
        InvalidToken(3, "Invalid token supplied."),
        CorruptedAccessor(4, "Could not identify the mandatory request's accessor. This might be an authentication bug."),
    }
}

#[cfg(test)]
mod tests {
    use tonic::metadata::MetadataMap;

    use super::*;

    #[test]
    fn extract_metadata_token_valid_bearer() {
        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Bearer my-token-123".parse().unwrap());
        let token = extract_metadata_authorization_token(&metadata);
        assert_eq!(token, Some("my-token-123".to_string()));
    }

    #[test]
    fn extract_metadata_token_missing_header() {
        let metadata = MetadataMap::new();
        let token = extract_metadata_authorization_token(&metadata);
        assert_eq!(token, None);
    }

    #[test]
    fn extract_metadata_token_no_bearer_prefix() {
        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Basic abc123".parse().unwrap());
        let token = extract_metadata_authorization_token(&metadata);
        assert_eq!(token, None);
    }

    #[test]
    fn extract_metadata_token_empty_bearer() {
        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Bearer ".parse().unwrap());
        let token = extract_metadata_authorization_token(&metadata);
        assert_eq!(token, Some("".to_string()));
    }

    #[test]
    fn extract_metadata_accessor_valid() {
        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Bearer token-value".parse().unwrap());
        let token = extract_metadata_accessor(&metadata);
        assert_eq!(token, Some("token-value".to_string()));
    }

    #[test]
    fn extract_metadata_accessor_missing() {
        let metadata = MetadataMap::new();
        let token = extract_metadata_accessor(&metadata);
        assert_eq!(token, None);
    }

    #[test]
    fn accessor_from_extensions_present() {
        let mut extensions = Extensions::new();
        extensions.insert(Accessor("admin".to_string()));
        let accessor = Accessor::from_extensions(&extensions);
        assert!(accessor.is_ok());
        assert_eq!(accessor.unwrap().0, "admin");
    }

    #[test]
    fn accessor_from_extensions_missing() {
        let extensions = Extensions::new();
        let accessor = Accessor::from_extensions(&extensions);
        assert!(accessor.is_err());
    }

    #[test]
    fn accessor_equality() {
        let a = Accessor("admin".to_string());
        let b = Accessor("admin".to_string());
        assert_eq!(a, b);
    }

    #[test]
    fn accessor_inequality() {
        let a = Accessor("admin".to_string());
        let b = Accessor("user".to_string());
        assert_ne!(a, b);
    }

    #[test]
    fn accessor_ordering() {
        let a = Accessor("admin".to_string());
        let b = Accessor("user".to_string());
        assert!(a < b);
    }

    #[test]
    fn accessor_clone() {
        let a = Accessor("admin".to_string());
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn http_constants() {
        assert_eq!(HTTP_AUTHORIZATION_FIELD, "authorization");
        assert_eq!(HTTP_BEARER_PREFIX, "Bearer ");
    }

    #[test]
    fn authentication_error_codes() {
        use error::TypeDBError;
        let err = AuthenticationError::InvalidCredential {};
        assert_eq!(err.code(), "AUT1");
        let err = AuthenticationError::MissingToken {};
        assert_eq!(err.code(), "AUT2");
        let err = AuthenticationError::InvalidToken {};
        assert_eq!(err.code(), "AUT3");
        let err = AuthenticationError::CorruptedAccessor {};
        assert_eq!(err.code(), "AUT4");
    }
}
