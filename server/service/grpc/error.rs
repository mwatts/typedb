/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{cmp::Ordering, collections::HashMap};

use error::{typedb_error, TypeDBError};
use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};

// Errors caused by incorrect implementation or usage of the network protocol.
// Note: NOT a typedb_error!(), since we want go directly to Status
#[derive(Debug)]
pub(crate) enum ProtocolError {
    MissingField {
        name: &'static str,
        description: &'static str,
    },
    TransactionAlreadyOpen {},
    TransactionClosed {},
    UnrecognisedTransactionType {
        enum_variant: i32,
    },
    IncompatibleProtocolVersion {
        server_protocol_version: i32,
        driver_protocol_version: i32,
        driver_lang: String,
        driver_version: String,
    },
    ErrorCompletingWrite {},
    FailedQueryResponse {},
}

impl IntoGrpcStatus for ProtocolError {
    fn into_status(self) -> Status {
        match self {
            Self::MissingField { name, description } => Status::with_error_details(
                Code::InvalidArgument,
                "Bad request",
                ErrorDetails::with_bad_request_violation(
                    name,
                    format!("{}. Check client-server compatibility?", description),
                ),
            ),
            Self::TransactionAlreadyOpen {} => Status::already_exists("Transaction already open."),
            Self::TransactionClosed {} => {
                Status::new(Code::InvalidArgument, "Transaction already closed, no further operations possible.")
            }
            Self::IncompatibleProtocolVersion {
                server_protocol_version,
                driver_protocol_version,
                driver_version,
                driver_lang,
            } => {
                let required_driver_age = match server_protocol_version.cmp(&driver_protocol_version) {
                    Ordering::Less => "an older",
                    Ordering::Equal => unreachable!("Incompatible protocol version should only be thrown "),
                    Ordering::Greater => "a newer",
                };

                Status::failed_precondition(format!(
                    r#"
                    Incompatible driver version. This '{driver_lang}' driver version '{driver_version}' implements protocol version {driver_protocol_version},
                    while the server supports network protocol version {server_protocol_version}. Please use {required_driver_age} driver that is compatible with this server.
                    "#
                ))
            }
            Self::UnrecognisedTransactionType { enum_variant, .. } => Status::with_error_details(
                Code::InvalidArgument,
                "Bad request",
                ErrorDetails::with_bad_request_violation(
                    "transaction_type",
                    format!(
                        "Unrecognised transaction type variant: {enum_variant}. Check client-server compatibility?"
                    ),
                ),
            ),
            Self::ErrorCompletingWrite {} => {
                Status::new(Code::Internal, "Error completing currently executing write query.")
            }
            Self::FailedQueryResponse {} => Status::internal("Failed to send response"),
        }
    }
}

pub(crate) trait IntoProtocolErrorMessage {
    fn into_error_message(self) -> typedb_protocol::Error;
}

impl<T: TypeDBError + Sync> IntoProtocolErrorMessage for T {
    fn into_error_message(self) -> typedb_protocol::Error {
        let root_source = self.root_source_typedb_error();
        typedb_protocol::Error {
            error_code: root_source.code().to_string(),
            domain: root_source.component().to_string(),
            stack_trace: self.stack_trace(),
        }
    }
}

pub(crate) trait IntoGrpcStatus {
    fn into_status(self) -> Status;
}

impl IntoGrpcStatus for typedb_protocol::Error {
    fn into_status(self) -> Status {
        let mut details = ErrorDetails::with_error_info(self.error_code, self.domain, HashMap::new());
        details.set_debug_info(self.stack_trace, "");
        Status::with_error_details(Code::InvalidArgument, "Request generated error", details)
    }
}

typedb_error! {
    pub(crate) GrpcServiceError(component = "GRPC Service", prefix = "GSR") {
        UnexpectedMissingField(1, "Invalid request: missing field '{field}'.", field: String),
    }
}

#[cfg(test)]
mod tests {
    use error::TypeDBError;
    use tonic::Code;

    use super::*;

    // --- ProtocolError into_status ---

    #[test]
    fn missing_field_produces_invalid_argument() {
        let err = ProtocolError::MissingField { name: "database_name", description: "Required field" };
        let status = err.into_status();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), "Bad request");
    }

    #[test]
    fn transaction_already_open_produces_already_exists() {
        let err = ProtocolError::TransactionAlreadyOpen {};
        let status = err.into_status();
        assert_eq!(status.code(), Code::AlreadyExists);
        assert!(status.message().contains("Transaction already open"));
    }

    #[test]
    fn transaction_closed_produces_invalid_argument() {
        let err = ProtocolError::TransactionClosed {};
        let status = err.into_status();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("Transaction already closed"));
    }

    #[test]
    fn unrecognised_transaction_type_produces_invalid_argument() {
        let err = ProtocolError::UnrecognisedTransactionType { enum_variant: 99 };
        let status = err.into_status();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), "Bad request");
    }

    #[test]
    fn incompatible_protocol_newer_server() {
        let err = ProtocolError::IncompatibleProtocolVersion {
            server_protocol_version: 5,
            driver_protocol_version: 3,
            driver_lang: "rust".to_string(),
            driver_version: "1.0".to_string(),
        };
        let status = err.into_status();
        assert_eq!(status.code(), Code::FailedPrecondition);
        let msg = status.message().to_string();
        assert!(msg.contains("a newer"));
        assert!(msg.contains("rust"));
        assert!(msg.contains("1.0"));
    }

    #[test]
    fn incompatible_protocol_older_server() {
        let err = ProtocolError::IncompatibleProtocolVersion {
            server_protocol_version: 2,
            driver_protocol_version: 5,
            driver_lang: "python".to_string(),
            driver_version: "2.0".to_string(),
        };
        let status = err.into_status();
        assert_eq!(status.code(), Code::FailedPrecondition);
        let msg = status.message().to_string();
        assert!(msg.contains("an older"));
        assert!(msg.contains("python"));
    }

    #[test]
    fn error_completing_write_produces_internal() {
        let err = ProtocolError::ErrorCompletingWrite {};
        let status = err.into_status();
        assert_eq!(status.code(), Code::Internal);
        assert!(status.message().contains("Error completing"));
    }

    #[test]
    fn failed_query_response_produces_internal() {
        let err = ProtocolError::FailedQueryResponse {};
        let status = err.into_status();
        assert_eq!(status.code(), Code::Internal);
        assert!(status.message().contains("Failed to send response"));
    }

    // --- typedb_protocol::Error into_status ---

    #[test]
    fn protocol_error_message_into_status() {
        let err = typedb_protocol::Error {
            error_code: "TST01".to_string(),
            domain: "Test".to_string(),
            stack_trace: vec!["error line 1".to_string(), "error line 2".to_string()],
        };
        let status = err.into_status();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), "Request generated error");
    }

    // --- IntoProtocolErrorMessage ---

    #[test]
    fn grpc_service_error_into_error_message() {
        let err = GrpcServiceError::UnexpectedMissingField { field: "name".to_string() };
        let msg = err.into_error_message();
        assert_eq!(msg.error_code, "GSR1");
        assert_eq!(msg.domain, "GRPC Service");
        assert!(!msg.stack_trace.is_empty());
    }

    // --- GrpcServiceError ---

    #[test]
    fn grpc_service_error_code() {
        let err = GrpcServiceError::UnexpectedMissingField { field: "db".to_string() };
        assert_eq!(err.code().to_string(), "GSR1");
    }

    #[test]
    fn grpc_service_error_component() {
        let err = GrpcServiceError::UnexpectedMissingField { field: "db".to_string() };
        assert_eq!(err.component(), "GRPC Service");
    }

    #[test]
    fn grpc_service_error_format_description() {
        let err = GrpcServiceError::UnexpectedMissingField { field: "db".to_string() };
        let desc = error::TypeDBError::format_description(&err);
        assert!(desc.contains("missing field"));
        assert!(desc.contains("db"));
    }
}
