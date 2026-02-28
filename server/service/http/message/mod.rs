/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

pub mod analyze;
pub mod authentication;
pub(crate) mod body;
pub mod database;
pub mod error;
pub mod query;
pub mod transaction;
pub mod user;
pub(crate) mod version;

macro_rules! stringify_kebab_case {
    ($t:tt) => {
        stringify!($t).replace("_", "-")
    };
}
pub(crate) use stringify_kebab_case;

macro_rules! from_request_parts_impl {
    ($struct_name:ident { $($field_name:ident : $field_ty:ty),* $(,)? }) => {
        #[axum::async_trait]
        impl<S> axum::extract::FromRequestParts<S> for $struct_name
        where
            S: Send + Sync,
        {
            type Rejection = axum::response::Response;

            async fn from_request_parts(
                parts: &mut axum::http::request::Parts,
                state: &S,
            ) -> Result<Self, Self::Rejection> {
                use axum::extract::Path;
                use axum::response::IntoResponse;
                use std::collections::HashMap;
                use $crate::service::http::message::stringify_kebab_case;
                use crate::service::http::error::HttpServiceError;

                let params: Path<HashMap<String, String>> = Path::<HashMap<String, String>>::from_request_parts(parts, state)
                    .await
                    .map_err(IntoResponse::into_response)?;

                $(
                    let field_name = stringify_kebab_case!($field_name);
                    let $field_name = params.get(&field_name)
                        .ok_or_else(|| HttpServiceError::MissingPathParameter { parameter: field_name.clone() }.into_response())?
                        .parse::<$field_ty>()
                        .map_err(|_| HttpServiceError::InvalidPathParameter { parameter: field_name }.into_response())?;
                )*

                Ok(Self { $($field_name),* })
            }
        }
    };
}
pub(crate) use from_request_parts_impl;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{authentication::*, database::*, transaction::*, user::*, version::*};
    use crate::service::{AnswerType, QueryType, TransactionType};

    // --- Authentication messages ---

    #[test]
    fn signin_payload_roundtrip() {
        let json = json!({"username": "admin", "password": "secret"});
        let payload: SigninPayload = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(payload.username, "admin");
        assert_eq!(payload.password, "secret");
        let serialized = serde_json::to_value(&payload).unwrap();
        assert_eq!(serialized, json);
    }

    #[test]
    fn token_response_roundtrip() {
        let response = encode_token("my-jwt-token".to_string());
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["token"], "my-jwt-token");
        let deserialized: TokenResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.token, "my-jwt-token");
    }

    // --- Database messages ---

    #[test]
    fn database_response_roundtrip() {
        let response = encode_database("test_db".to_string());
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["name"], "test_db");
        let deserialized: DatabaseResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.name, "test_db");
    }

    #[test]
    fn databases_response_roundtrip() {
        let response = encode_databases(vec!["db1".to_string(), "db2".to_string()]);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["databases"].as_array().unwrap().len(), 2);
        let deserialized: DatabasesResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.databases.len(), 2);
        assert_eq!(deserialized.databases[0].name, "db1");
        assert_eq!(deserialized.databases[1].name, "db2");
    }

    #[test]
    fn databases_response_empty() {
        let response = encode_databases(vec![]);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["databases"].as_array().unwrap().len(), 0);
    }

    // --- Transaction messages ---

    #[test]
    fn transaction_open_payload_read() {
        let json = json!({
            "databaseName": "test_db",
            "transactionType": "read"
        });
        let payload: TransactionOpenPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.database_name, "test_db");
        assert_eq!(payload.transaction_type, TransactionType::Read);
        assert!(payload.transaction_options.is_none());
    }

    #[test]
    fn transaction_open_payload_write() {
        let json = json!({
            "databaseName": "test_db",
            "transactionType": "write"
        });
        let payload: TransactionOpenPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.transaction_type, TransactionType::Write);
    }

    #[test]
    fn transaction_open_payload_schema() {
        let json = json!({
            "databaseName": "test_db",
            "transactionType": "schema"
        });
        let payload: TransactionOpenPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.transaction_type, TransactionType::Schema);
    }

    #[test]
    fn transaction_open_payload_with_options() {
        let json = json!({
            "databaseName": "test_db",
            "transactionType": "read",
            "transactionOptions": {
                "schemaLockAcquireTimeoutMillis": 5000,
                "transactionTimeoutMillis": 10000
            }
        });
        let payload: TransactionOpenPayload = serde_json::from_value(json).unwrap();
        let opts = payload.transaction_options.unwrap();
        assert_eq!(opts.schema_lock_acquire_timeout_millis, Some(5000));
        assert_eq!(opts.transaction_timeout_millis, Some(10000));
    }

    #[test]
    fn transaction_response_roundtrip() {
        let id = uuid::Uuid::new_v4();
        let response = encode_transaction(id);
        let json = serde_json::to_value(&response).unwrap();
        let deserialized: TransactionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.transaction_id, id);
    }

    #[test]
    fn transaction_options_default() {
        let opts = TransactionOptionsPayload::default();
        assert!(opts.schema_lock_acquire_timeout_millis.is_none());
        assert!(opts.transaction_timeout_millis.is_none());
    }

    // --- User messages ---

    #[test]
    fn create_user_payload_deserialize() {
        let json = json!({"password": "secret123"});
        let payload: CreateUserPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.password, "secret123");
    }

    #[test]
    fn update_user_payload_deserialize() {
        let json = json!({"password": "new_password"});
        let payload: UpdateUserPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.password, "new_password");
    }

    #[test]
    fn user_response_roundtrip() {
        let json = json!({"username": "alice"});
        let response: UserResponse = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(response.username, "alice");
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(serialized, json);
    }

    #[test]
    fn users_response_roundtrip() {
        let json = json!({"users": [{"username": "alice"}, {"username": "bob"}]});
        let response: UsersResponse = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(response.users.len(), 2);
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(serialized, json);
    }

    // --- Version messages ---

    #[test]
    fn server_version_response_roundtrip() {
        let response = encode_server_version("TypeDB".to_string(), "3.8.0".to_string());
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["distribution"], "TypeDB");
        assert_eq!(json["version"], "3.8.0");
        let deserialized: ServerVersionResponse = serde_json::from_value(json.clone()).unwrap();
        let reserialized = serde_json::to_value(&deserialized).unwrap();
        assert_eq!(json, reserialized);
    }

    #[test]
    fn protocol_version_from_str_v1() {
        use std::str::FromStr;
        let version = ProtocolVersion::from_str("v1").unwrap();
        assert_eq!(version, ProtocolVersion::V1);
    }

    #[test]
    fn protocol_version_from_str_invalid() {
        use std::str::FromStr;
        let result = ProtocolVersion::from_str("v2");
        assert!(result.is_err());
    }

    #[test]
    fn protocol_version_display() {
        assert_eq!(format!("{}", ProtocolVersion::V1), "v1");
    }

    // --- TransactionType / QueryType / AnswerType serde ---

    #[test]
    fn transaction_type_serde_roundtrip() {
        let json = serde_json::to_value(TransactionType::Read).unwrap();
        assert_eq!(json, "read");
        let json = serde_json::to_value(TransactionType::Write).unwrap();
        assert_eq!(json, "write");
        let json = serde_json::to_value(TransactionType::Schema).unwrap();
        assert_eq!(json, "schema");
    }

    #[test]
    fn query_type_serde_roundtrip() {
        let json = serde_json::to_value(QueryType::Read).unwrap();
        assert_eq!(json, "read");
        let json = serde_json::to_value(QueryType::Write).unwrap();
        assert_eq!(json, "write");
        let json = serde_json::to_value(QueryType::Schema).unwrap();
        assert_eq!(json, "schema");
    }

    #[test]
    fn answer_type_serde_roundtrip() {
        let json = serde_json::to_value(AnswerType::Ok).unwrap();
        assert_eq!(json, "ok");
        let json = serde_json::to_value(AnswerType::ConceptRows).unwrap();
        assert_eq!(json, "conceptRows");
        let json = serde_json::to_value(AnswerType::ConceptDocuments).unwrap();
        assert_eq!(json, "conceptDocuments");
    }

    // --- Error messages ---

    #[test]
    fn error_response_roundtrip() {
        use super::error::ErrorResponse;
        let json = json!({"code": "TST1", "message": "Something went wrong."});
        let response: ErrorResponse = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(response.code, "TST1");
        assert_eq!(response.message, "Something went wrong.");
        let serialized = serde_json::to_value(&response).unwrap();
        assert_eq!(serialized, json);
    }

    // --- Query options ---

    #[test]
    fn query_options_payload_default() {
        let opts = super::query::QueryOptionsPayload::default();
        assert!(opts.include_instance_types.is_none());
        assert!(opts.answer_count_limit.is_none());
        assert!(opts.include_query_structure.is_none());
    }

    #[test]
    fn query_options_payload_roundtrip() {
        let json = json!({
            "includeInstanceTypes": true,
            "answerCountLimit": 100,
            "includeQueryStructure": false
        });
        let opts: super::query::QueryOptionsPayload = serde_json::from_value(json).unwrap();
        assert_eq!(opts.include_instance_types, Some(true));
        assert_eq!(opts.answer_count_limit, Some(100));
        assert_eq!(opts.include_query_structure, Some(false));
    }

    // --- Query payloads ---

    #[test]
    fn transaction_query_payload_deserialize() {
        let json = json!({
            "query": "match $x isa person;"
        });
        let payload: super::query::TransactionQueryPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.query, "match $x isa person;");
        assert!(payload.query_options.is_none());
    }

    #[test]
    fn query_payload_with_transaction_details() {
        let json = json!({
            "query": "match $x isa person;",
            "databaseName": "test_db",
            "transactionType": "read",
            "commit": true
        });
        let payload: super::query::QueryPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.query, "match $x isa person;");
        assert_eq!(payload.commit, Some(true));
        assert_eq!(payload.transaction_open_payload.database_name, "test_db");
        assert_eq!(payload.transaction_open_payload.transaction_type, TransactionType::Read);
    }

    // --- QueryAnswerResponse ---

    #[test]
    fn query_answer_response_ok() {
        let response = super::query::encode_query_ok_answer(QueryType::Schema);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["queryType"], "schema");
        assert_eq!(json["answerType"], "ok");
        assert!(json["answers"].is_null());
    }

    #[test]
    fn query_answer_response_rows() {
        let rows = vec![json!({"x": "entity"}), json!({"x": "relation"})];
        let response = super::query::encode_query_rows_answer(QueryType::Read, rows, None, None);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["queryType"], "read");
        assert_eq!(json["answerType"], "conceptRows");
        assert_eq!(json["answers"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn query_answer_response_documents() {
        let docs = vec![json!({"name": "Alice"})];
        let response =
            super::query::encode_query_documents_answer(QueryType::Read, docs, Some("warning!".to_string()));
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["answerType"], "conceptDocuments");
        assert_eq!(json["warning"], "warning!");
    }
}
