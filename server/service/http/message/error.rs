/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use axum::response::{IntoResponse, Response};
use error::TypeDBError;
use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    service::{
        http::{error::HttpServiceError, message::body::JsonBody},
        transaction_service::TransactionServiceError,
    },
    state::ServerStateError,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

impl IntoResponse for HttpServiceError {
    fn into_response(self) -> Response {
        let code = match &self {
            HttpServiceError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            HttpServiceError::JsonBodyExpected { .. } => StatusCode::BAD_REQUEST,
            HttpServiceError::RequestTimeout { .. } => StatusCode::REQUEST_TIMEOUT,
            HttpServiceError::NotFound { .. } => StatusCode::NOT_FOUND,
            HttpServiceError::UnknownVersion { .. } => StatusCode::NOT_FOUND,
            HttpServiceError::MissingPathParameter { .. } => StatusCode::NOT_FOUND,
            HttpServiceError::InvalidPathParameter { .. } => StatusCode::BAD_REQUEST,
            HttpServiceError::State { typedb_source } => match typedb_source {
                ServerStateError::Unimplemented { .. } => StatusCode::NOT_IMPLEMENTED,
                ServerStateError::OperationNotPermitted { .. } => StatusCode::FORBIDDEN,
                ServerStateError::DatabaseDoesNotExist { .. } => StatusCode::NOT_FOUND,
                ServerStateError::UserDoesNotExist { .. } => StatusCode::NOT_FOUND,
                ServerStateError::FailedToOpenPrerequisiteTransaction { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::ConceptReadError { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::FunctionReadError { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::UserCannotBeCreated { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::UserCannotBeRetrieved { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::UserCannotBeUpdated { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::UserCannotBeDeleted { .. } => StatusCode::BAD_REQUEST,
                ServerStateError::DatabaseExport { .. } => StatusCode::BAD_REQUEST,
            },
            HttpServiceError::Authentication { .. } => StatusCode::UNAUTHORIZED,
            HttpServiceError::DatabaseCreate { .. } => StatusCode::BAD_REQUEST,
            HttpServiceError::DatabaseDelete { .. } => StatusCode::BAD_REQUEST,
            HttpServiceError::Transaction { typedb_source } => match typedb_source {
                TransactionServiceError::DatabaseNotFound { .. } => StatusCode::NOT_FOUND,
                TransactionServiceError::CannotCommitReadTransaction { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::CannotRollbackReadTransaction { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::TransactionFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::DataCommitFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::SchemaCommitFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::QueryParseFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::SchemaQueryRequiresSchemaTransaction { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::WriteQueryRequiresSchemaOrWriteTransaction { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::TxnAbortSchemaQueryFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::QueryFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::AnalyseQueryFailed { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::AnalyseQueryExpectsPipeline { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::NoOpenTransaction { .. } => StatusCode::NOT_FOUND,
                TransactionServiceError::QueryInterrupted { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::QueryStreamNotFound { .. } => StatusCode::NOT_FOUND,
                TransactionServiceError::ServiceFailedQueueCleanup { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::PipelineExecution { .. } => StatusCode::BAD_REQUEST,
                TransactionServiceError::TransactionTimeout { .. } => StatusCode::REQUEST_TIMEOUT,
                TransactionServiceError::InvalidPrefetchSize { .. } => StatusCode::BAD_REQUEST,
            },
            HttpServiceError::QueryClose { .. } => StatusCode::BAD_REQUEST,
            HttpServiceError::QueryCommit { .. } => StatusCode::BAD_REQUEST,
        };
        (code, JsonBody(encode_error(self))).into_response()
    }
}

pub(crate) fn encode_error(error: HttpServiceError) -> ErrorResponse {
    ErrorResponse { code: error.root_source_typedb_error().code().to_string(), message: error.format_source_trace() }
}

#[cfg(test)]
mod tests {
    use http::StatusCode;

    use super::*;

    fn get_response_status(err: HttpServiceError) -> StatusCode {
        let response = err.into_response();
        response.status()
    }

    // --- Simple error variants status codes ---

    #[test]
    fn internal_error_returns_500() {
        assert_eq!(get_response_status(HttpServiceError::Internal { details: "boom".to_string() }), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn json_body_expected_returns_400() {
        assert_eq!(get_response_status(HttpServiceError::JsonBodyExpected { details: "bad json".to_string() }), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn request_timeout_returns_408() {
        assert_eq!(get_response_status(HttpServiceError::RequestTimeout {}), StatusCode::REQUEST_TIMEOUT);
    }

    #[test]
    fn not_found_returns_404() {
        assert_eq!(get_response_status(HttpServiceError::NotFound {}), StatusCode::NOT_FOUND);
    }

    #[test]
    fn unknown_version_returns_404() {
        assert_eq!(get_response_status(HttpServiceError::UnknownVersion { version: "v99".to_string() }), StatusCode::NOT_FOUND);
    }

    #[test]
    fn missing_path_parameter_returns_404() {
        assert_eq!(
            get_response_status(HttpServiceError::MissingPathParameter { parameter: "id".to_string() }),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn invalid_path_parameter_returns_400() {
        assert_eq!(
            get_response_status(HttpServiceError::InvalidPathParameter { parameter: "id".to_string() }),
            StatusCode::BAD_REQUEST
        );
    }

    // --- State error variants ---

    #[test]
    fn state_unimplemented_returns_501() {
        let err = HttpServiceError::State {
            typedb_source: ServerStateError::Unimplemented { description: "not done".to_string() },
        };
        assert_eq!(get_response_status(err), StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn state_operation_not_permitted_returns_403() {
        assert_eq!(get_response_status(HttpServiceError::operation_not_permitted()), StatusCode::FORBIDDEN);
    }

    #[test]
    fn state_database_not_exist_returns_404() {
        let err = HttpServiceError::State {
            typedb_source: ServerStateError::DatabaseDoesNotExist { name: "testdb".to_string() },
        };
        assert_eq!(get_response_status(err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn state_user_not_exist_returns_404() {
        let err = HttpServiceError::State { typedb_source: ServerStateError::UserDoesNotExist {} };
        assert_eq!(get_response_status(err), StatusCode::NOT_FOUND);
    }

    // --- Transaction error variants ---

    #[test]
    fn transaction_database_not_found_returns_404() {
        let err = HttpServiceError::Transaction {
            typedb_source: TransactionServiceError::DatabaseNotFound { name: "testdb".to_string() },
        };
        assert_eq!(get_response_status(err), StatusCode::NOT_FOUND);
    }

    #[test]
    fn transaction_cannot_commit_read_returns_400() {
        let err = HttpServiceError::Transaction {
            typedb_source: TransactionServiceError::CannotCommitReadTransaction {},
        };
        assert_eq!(get_response_status(err), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn transaction_no_open_returns_404() {
        assert_eq!(get_response_status(HttpServiceError::no_open_transaction()), StatusCode::NOT_FOUND);
    }

    #[test]
    fn transaction_timeout_returns_408() {
        assert_eq!(get_response_status(HttpServiceError::transaction_timeout()), StatusCode::REQUEST_TIMEOUT);
    }

    #[test]
    fn query_close_returns_400() {
        let err = HttpServiceError::QueryClose {
            typedb_source: TransactionServiceError::NoOpenTransaction {},
        };
        assert_eq!(get_response_status(err), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn query_commit_returns_400() {
        let err = HttpServiceError::QueryCommit {
            typedb_source: TransactionServiceError::CannotCommitReadTransaction {},
        };
        assert_eq!(get_response_status(err), StatusCode::BAD_REQUEST);
    }

    // --- encode_error ---

    #[test]
    fn encode_error_simple() {
        let err = HttpServiceError::Internal { details: "oops".to_string() };
        let resp = encode_error(err);
        assert_eq!(resp.code, "HSR1");
        assert!(resp.message.contains("Internal error"));
    }

    #[test]
    fn encode_error_nested_uses_root_source_code() {
        let err = HttpServiceError::operation_not_permitted();
        let resp = encode_error(err);
        // Root source is the ServerStateError, not the HttpServiceError wrapper
        assert_eq!(resp.code, "SRV2");
        assert!(resp.message.contains("not permitted"));
    }

    #[test]
    fn encode_error_transaction_uses_root_source_code() {
        let err = HttpServiceError::no_open_transaction();
        let resp = encode_error(err);
        assert_eq!(resp.code, "TSV12");
    }

    // --- ErrorResponse serde ---

    #[test]
    fn error_response_serialization() {
        let resp = ErrorResponse { code: "TST01".to_string(), message: "Something went wrong".to_string() };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"code\":\"TST01\""));
        assert!(json.contains("\"message\":\"Something went wrong\""));
    }

    #[test]
    fn error_response_deserialization() {
        let json = r#"{"code":"ERR42","message":"Bad input"}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, "ERR42");
        assert_eq!(resp.message, "Bad input");
    }

    #[test]
    fn error_response_roundtrip() {
        let resp = ErrorResponse { code: "HSR01".to_string(), message: "Internal error: test".to_string() };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.code, resp.code);
        assert_eq!(deserialized.message, resp.message);
    }
}
