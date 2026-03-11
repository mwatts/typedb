/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use concurrency::TokioIntervalRunner;
use error::typedb_error;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::{self, Rng};
use resource::constants::server::{MAX_AUTHENTICATION_TOKEN_EXPIRATION, MIN_AUTHENTICATION_TOKEN_EXPIRATION};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct TokenManager {
    token_owners: Arc<RwLock<HashMap<String, String>>>,
    tokens_expiration_time: Duration,
    secret_key: String,
    _tokens_cleanup_job: Arc<TokioIntervalRunner>,
}

impl TokenManager {
    const TOKENS_CLEANUP_INTERVAL_MULTIPLIER: u32 = 2;

    pub fn new(tokens_expiration_time: Duration) -> Result<Self, TokenManagerError> {
        Self::validate_tokens_expiration_time(tokens_expiration_time)?;

        let token_owners = Arc::new(RwLock::new(HashMap::new()));
        let token_owners_clone = token_owners.clone();

        // We do not specifically aim to use JWT, as we perform additional manual validation
        // and use local caches (meaning that every server restart invalidates previously generated tokens).
        // Therefore, it is acceptable to generate random secret keys without exposing or configuring them.
        // This approach can be changed in the future if needed.
        let secret_key = Self::random_key();
        let secret_key_clone = secret_key.clone();

        let tokens_cleanup_interval = tokens_expiration_time * Self::TOKENS_CLEANUP_INTERVAL_MULTIPLIER;
        let tokens_cleanup_job = Arc::new(TokioIntervalRunner::new(
            move || {
                let token_owners = token_owners_clone.clone();
                let secret_key = secret_key_clone.clone();
                async move {
                    Self::cleanup_expired_tokens(secret_key.as_ref(), token_owners).await;
                }
            },
            tokens_cleanup_interval,
            false,
        ));
        Ok(Self { token_owners, tokens_expiration_time, secret_key, _tokens_cleanup_job: tokens_cleanup_job })
    }

    pub async fn new_token(&self, username: String) -> String {
        // Lock earlier to make sure that `issued_at` and the token are unique
        let mut write_guard = self.token_owners.write().await;

        let issued_at = SystemTime::now();
        let expires_at = issued_at + self.tokens_expiration_time;
        let claims = Claims {
            sub: username.clone(),
            exp: Self::system_time_to_seconds(expires_at),
            iat: Self::system_time_to_seconds(issued_at),
        };

        let token = Self::encode_token(self.secret_key.as_ref(), claims);
        write_guard.insert(token.clone(), username);
        token
    }

    pub async fn get_valid_token_owner(&self, token: &str) -> Option<String> {
        if let Some(claims) = Self::decode_token(self.secret_key.as_ref(), token) {
            if !Self::is_expired(claims.exp) {
                return self.token_owners.read().await.get(token).cloned();
            }
        }
        None
    }

    pub async fn invalidate_user(&self, username: &str) {
        let mut write_guard = self.token_owners.write().await;
        write_guard.retain(|_, token_username| token_username != username);
    }

    async fn cleanup_expired_tokens(secret_key: &[u8], token_owners: Arc<RwLock<HashMap<String, String>>>) {
        let mut write_guard = token_owners.write().await;
        write_guard.retain(|token, _| {
            let Some(claims) = Self::decode_token(secret_key, token) else { return false };
            !Self::is_expired(claims.exp)
        });
    }

    fn encode_token(secret_key: &[u8], claims: Claims) -> String {
        // Default algorithm is HS512
        encode(&Header::default(), &claims, &EncodingKey::from_secret(secret_key))
            .expect("Expected authentication token encoding")
    }

    fn decode_token(secret_key: &[u8], token: &str) -> Option<Claims> {
        // We pass all invalid and expired tokens here. If it's somehow incorrect and returns an
        // error, we don't care - just say that decoding leads to no valid claims.
        decode(token, &DecodingKey::from_secret(secret_key), &Validation::default()).map(|res| res.claims).ok()
    }

    fn system_time_to_seconds(time: SystemTime) -> u64 {
        time.duration_since(UNIX_EPOCH).expect("Expected duration since Unix epoch").as_secs()
    }

    fn is_expired(token_exp: u64) -> bool {
        token_exp <= Self::system_time_to_seconds(SystemTime::now())
    }

    fn random_key() -> String {
        rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(128).map(char::from).collect()
    }

    fn validate_tokens_expiration_time(tokens_expiration_time: Duration) -> Result<(), TokenManagerError> {
        if tokens_expiration_time < MIN_AUTHENTICATION_TOKEN_EXPIRATION
            || tokens_expiration_time > MAX_AUTHENTICATION_TOKEN_EXPIRATION
        {
            Err(TokenManagerError::InvlaidTokensExpirationTime {
                value: tokens_expiration_time.as_secs(),
                min: MIN_AUTHENTICATION_TOKEN_EXPIRATION.as_secs(),
                max: MAX_AUTHENTICATION_TOKEN_EXPIRATION.as_secs(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub(crate) struct Claims {
    sub: String,
    exp: u64,
    iat: u64,
}

typedb_error! {
    pub TokenManagerError(component = "Token manager", prefix = "TKM") {
        InvlaidTokensExpirationTime(1, "Invalid tokens expiration time '{value}'. It must be between '{min}' and '{max}' seconds.", value: u64, min: u64, max: u64),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use resource::constants::server::{MAX_AUTHENTICATION_TOKEN_EXPIRATION, MIN_AUTHENTICATION_TOKEN_EXPIRATION};

    use super::*;

    fn valid_expiration() -> Duration {
        Duration::from_secs(60)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_token_manager_with_valid_expiration() {
        let tm = TokenManager::new(valid_expiration());
        assert!(tm.is_ok());
    }

    #[test]
    fn reject_expiration_below_minimum() {
        let too_short = MIN_AUTHENTICATION_TOKEN_EXPIRATION - Duration::from_secs(1);
        let result = TokenManager::new(too_short);
        assert!(result.is_err());
    }

    #[test]
    fn reject_expiration_above_maximum() {
        let too_long = MAX_AUTHENTICATION_TOKEN_EXPIRATION + Duration::from_secs(1);
        let result = TokenManager::new(too_long);
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accept_minimum_expiration() {
        let result = TokenManager::new(MIN_AUTHENTICATION_TOKEN_EXPIRATION);
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accept_maximum_expiration() {
        let result = TokenManager::new(MAX_AUTHENTICATION_TOKEN_EXPIRATION);
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn new_token_is_valid() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let token = tm.new_token("alice".to_string()).await;
        assert!(!token.is_empty());
        let owner = tm.get_valid_token_owner(&token).await;
        assert_eq!(owner, Some("alice".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn multiple_tokens_for_same_user() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let token1 = tm.new_token("alice".to_string()).await;
        // Sleep to ensure different iat timestamp (JWT tokens with identical claims are deterministic)
        tokio::time::sleep(Duration::from_secs(1)).await;
        let token2 = tm.new_token("alice".to_string()).await;
        assert_ne!(token1, token2);
        assert_eq!(tm.get_valid_token_owner(&token1).await, Some("alice".to_string()));
        assert_eq!(tm.get_valid_token_owner(&token2).await, Some("alice".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tokens_for_different_users() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let token_alice = tm.new_token("alice".to_string()).await;
        let token_bob = tm.new_token("bob".to_string()).await;
        assert_eq!(tm.get_valid_token_owner(&token_alice).await, Some("alice".to_string()));
        assert_eq!(tm.get_valid_token_owner(&token_bob).await, Some("bob".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_token_returns_none() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let owner = tm.get_valid_token_owner("invalid-token").await;
        assert_eq!(owner, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_token_returns_none() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let owner = tm.get_valid_token_owner("").await;
        assert_eq!(owner, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalidate_user_revokes_all_tokens() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let token1 = tm.new_token("alice".to_string()).await;
        let token2 = tm.new_token("alice".to_string()).await;
        let token_bob = tm.new_token("bob".to_string()).await;

        tm.invalidate_user("alice").await;

        assert_eq!(tm.get_valid_token_owner(&token1).await, None);
        assert_eq!(tm.get_valid_token_owner(&token2).await, None);
        // Bob's token should still be valid
        assert_eq!(tm.get_valid_token_owner(&token_bob).await, Some("bob".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalidate_nonexistent_user_is_noop() {
        let tm = TokenManager::new(valid_expiration()).unwrap();
        let token = tm.new_token("alice".to_string()).await;
        tm.invalidate_user("nonexistent").await;
        assert_eq!(tm.get_valid_token_owner(&token).await, Some("alice".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_from_different_manager_is_invalid() {
        let tm1 = TokenManager::new(valid_expiration()).unwrap();
        let tm2 = TokenManager::new(valid_expiration()).unwrap();
        let token = tm1.new_token("alice".to_string()).await;
        // Different manager has different secret key and no record of this token
        assert_eq!(tm2.get_valid_token_owner(&token).await, None);
    }

    #[test]
    fn random_key_is_128_chars() {
        let key = TokenManager::random_key();
        assert_eq!(key.len(), 128);
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn random_keys_are_unique() {
        let key1 = TokenManager::random_key();
        let key2 = TokenManager::random_key();
        assert_ne!(key1, key2);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let secret = b"test-secret-key";
        let claims = Claims { sub: "alice".to_string(), exp: u64::MAX, iat: 0 };
        let token = TokenManager::encode_token(secret, claims.clone());
        let decoded = TokenManager::decode_token(secret, &token);
        assert!(decoded.is_some());
        let decoded = decoded.unwrap();
        assert_eq!(decoded.sub, "alice");
    }

    #[test]
    fn decode_with_wrong_key_fails() {
        let claims = Claims { sub: "alice".to_string(), exp: u64::MAX, iat: 0 };
        let token = TokenManager::encode_token(b"key1", claims);
        let decoded = TokenManager::decode_token(b"key2", &token);
        assert!(decoded.is_none());
    }

    #[test]
    fn decode_expired_token_fails() {
        let claims = Claims { sub: "alice".to_string(), exp: 0, iat: 0 };
        let token = TokenManager::encode_token(b"key", claims);
        let decoded = TokenManager::decode_token(b"key", &token);
        // jsonwebtoken library validates expiration during decode
        assert!(decoded.is_none());
    }

    #[test]
    fn is_expired_past_time() {
        assert!(TokenManager::is_expired(0));
    }

    #[test]
    fn is_expired_future_time() {
        assert!(!TokenManager::is_expired(u64::MAX));
    }

    #[test]
    fn token_manager_error_display() {
        let err = TokenManagerError::InvlaidTokensExpirationTime { value: 0, min: 1, max: 100 };
        let msg = error::TypeDBError::format_description(&err);
        assert!(msg.contains("Invalid tokens expiration time"));
        assert!(msg.contains("0"));
    }
}
