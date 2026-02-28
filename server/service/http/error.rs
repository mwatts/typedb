/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use database::{database::DatabaseCreateError, DatabaseDeleteError};
use error::{typedb_error, TypeDBError};

use crate::{
    authentication::AuthenticationError, service::transaction_service::TransactionServiceError, state::ServerStateError,
};

typedb_error!(
    pub HttpServiceError(component = "HTTP Service", prefix = "HSR") {
        Internal(1, "Internal error: {details}", details: String),
        JsonBodyExpected(2, "Cannot parse expected JSON body: {details}", details: String),
        RequestTimeout(3, "Request timeout."),
        NotFound(4, "Requested resource not found."),
        UnknownVersion(5, "Unknown API version '{version}'.", version: String),
        MissingPathParameter(6, "Requested resource not found: missing path parameter {parameter}.", parameter: String),
        InvalidPathParameter(7, "Requested resource not found: invalid path parameter {parameter}.", parameter: String),
        State(8, "State error.", typedb_source: ServerStateError),
        Authentication(9, "Authentication error.", typedb_source: AuthenticationError),
        DatabaseCreate(10, "Database create error.", typedb_source: DatabaseCreateError),
        DatabaseDelete(11, "Database delete error.", typedb_source: DatabaseDeleteError),
        Transaction(16, "Transaction error.", typedb_source: TransactionServiceError),
        QueryClose(17, "Error while closing single-query transaction.", typedb_source: TransactionServiceError),
        QueryCommit(18, "Error while committing single-query transaction.", typedb_source: TransactionServiceError),
    }
);

impl HttpServiceError {
    pub(crate) fn source(&self) -> &(dyn TypeDBError + Sync + '_) {
        self.source_typedb_error().unwrap_or(self)
    }

    pub(crate) fn format_source_trace(&self) -> String {
        self.stack_trace().join("\n").to_string()
    }

    pub(crate) fn operation_not_permitted() -> Self {
        Self::State { typedb_source: ServerStateError::OperationNotPermitted {} }
    }

    pub(crate) fn no_open_transaction() -> Self {
        Self::Transaction { typedb_source: TransactionServiceError::NoOpenTransaction {} }
    }

    pub(crate) fn transaction_timeout() -> Self {
        Self::Transaction { typedb_source: TransactionServiceError::TransactionTimeout {} }
    }
}

#[cfg(test)]
mod tests {
    use error::TypeDBError;

    use super::*;

    // --- Simple variant construction and properties ---

    #[test]
    fn internal_error_code() {
        let err = HttpServiceError::Internal { details: "oops".to_string() };
        assert_eq!(err.code().to_string(), "HSR1");
    }

    #[test]
    fn internal_error_component() {
        let err = HttpServiceError::Internal { details: "oops".to_string() };
        assert_eq!(err.component(), "HTTP Service");
    }

    #[test]
    fn internal_error_description() {
        let err = HttpServiceError::Internal { details: "oops".to_string() };
        let desc = error::TypeDBError::format_description(&err);
        assert!(desc.contains("Internal error"));
        assert!(desc.contains("oops"));
    }

    #[test]
    fn json_body_expected_error() {
        let err = HttpServiceError::JsonBodyExpected { details: "missing field".to_string() };
        assert_eq!(err.code().to_string(), "HSR2");
        let desc = error::TypeDBError::format_description(&err);
        assert!(desc.contains("Cannot parse expected JSON body"));
    }

    #[test]
    fn request_timeout_error() {
        let err = HttpServiceError::RequestTimeout {};
        assert_eq!(err.code().to_string(), "HSR3");
    }

    #[test]
    fn not_found_error() {
        let err = HttpServiceError::NotFound {};
        assert_eq!(err.code().to_string(), "HSR4");
    }

    #[test]
    fn unknown_version_error() {
        let err = HttpServiceError::UnknownVersion { version: "v99".to_string() };
        assert_eq!(err.code().to_string(), "HSR5");
        let desc = error::TypeDBError::format_description(&err);
        assert!(desc.contains("v99"));
    }

    #[test]
    fn missing_path_parameter_error() {
        let err = HttpServiceError::MissingPathParameter { parameter: "db_name".to_string() };
        assert_eq!(err.code().to_string(), "HSR6");
        let desc = error::TypeDBError::format_description(&err);
        assert!(desc.contains("db_name"));
    }

    #[test]
    fn invalid_path_parameter_error() {
        let err = HttpServiceError::InvalidPathParameter { parameter: "id".to_string() };
        assert_eq!(err.code().to_string(), "HSR7");
    }

    // --- Convenience constructors ---

    #[test]
    fn operation_not_permitted_wraps_state_error() {
        let err = HttpServiceError::operation_not_permitted();
        assert!(matches!(err, HttpServiceError::State { .. }));
        assert_eq!(err.code().to_string(), "HSR8");
    }

    #[test]
    fn no_open_transaction_wraps_transaction_error() {
        let err = HttpServiceError::no_open_transaction();
        assert!(matches!(err, HttpServiceError::Transaction { .. }));
        assert_eq!(err.code().to_string(), "HSR16");
    }

    #[test]
    fn transaction_timeout_wraps_transaction_error() {
        let err = HttpServiceError::transaction_timeout();
        assert!(matches!(err, HttpServiceError::Transaction { .. }));
        assert_eq!(err.code().to_string(), "HSR16");
    }

    // --- source() method ---

    #[test]
    fn source_returns_self_for_simple_error() {
        let err = HttpServiceError::Internal { details: "test".to_string() };
        let source = err.source();
        assert_eq!(source.code().to_string(), "HSR1");
    }

    #[test]
    fn source_returns_inner_for_nested_error() {
        let err = HttpServiceError::operation_not_permitted();
        let source = err.source();
        // The source should be the inner ServerStateError
        assert_eq!(source.code().to_string(), "SRV2");
    }

    // --- format_source_trace ---

    #[test]
    fn format_source_trace_simple() {
        let err = HttpServiceError::Internal { details: "test".to_string() };
        let trace = err.format_source_trace();
        assert!(!trace.is_empty());
        assert!(trace.contains("Internal error"));
    }

    #[test]
    fn format_source_trace_nested() {
        let err = HttpServiceError::operation_not_permitted();
        let trace = err.format_source_trace();
        assert!(trace.contains("State error"));
    }

    // --- root_source for nested errors ---

    #[test]
    fn root_source_of_state_error() {
        let err = HttpServiceError::operation_not_permitted();
        let root = err.root_source_typedb_error();
        assert_eq!(root.code().to_string(), "SRV2");
    }

    #[test]
    fn root_source_of_transaction_error() {
        let err = HttpServiceError::no_open_transaction();
        let root = err.root_source_typedb_error();
        assert_eq!(root.code().to_string(), "TSV12");
    }
}
