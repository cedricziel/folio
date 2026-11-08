//! Disconnecting a client must win against a token exchange running at the
//! same moment: no token issued concurrently may outlive the revoke.

mod common;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use folio_auth::{
    crypto::pkce_challenge,
    server::{now, OAuthServer, TokenRequest, ValidatedAuthorize},
    AuthStore, OAuthClient, User,
};

const BASE: &str = "https://folio.test";
const REDIRECT: &str = "https://client.test/cb";
const VERIFIER: &str = "a-pkce-verifier-that-is-long-enough-0123456789";
const ROUNDS: usize = 40;

async fn seed(store: &Arc<dyn AuthStore>) -> OAuthClient {
    store
        .create_user(User {
            id: "ada".into(),
            username: "ada".into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    let client = OAuthClient {
        client_id: "client".into(),
        client_name: "Racer".into(),
        redirect_uris: vec![REDIRECT.into()],
        created_at: now(),
    };
    store.insert_client(client.clone()).await.unwrap();
    client
}

async fn issue_code(server: &OAuthServer, client: &OAuthClient) -> String {
    server
        .issue_code(
            "ada",
            &ValidatedAuthorize {
                client: client.clone(),
                redirect_uri: REDIRECT.into(),
                code_challenge: pkce_challenge(VERIFIER),
                state: None,
                scopes: vec!["documents:read".into()],
                audience: format!("{BASE}/mcp"),
            },
        )
        .await
        .unwrap()
}

async fn exchange(server: &OAuthServer, client_id: &str, code: String) -> Option<String> {
    let tokens = server
        .token(
            BASE,
            TokenRequest {
                grant_type: Some("authorization_code".into()),
                code: Some(code),
                redirect_uri: Some(REDIRECT.into()),
                code_verifier: Some(VERIFIER.into()),
                client_id: Some(client_id.into()),
                ..Default::default()
            },
        )
        .await
        .ok()?;
    Some(tokens["refresh_token"].as_str().unwrap().to_string())
}

async fn rotate(server: &OAuthServer, client_id: &str, refresh: &str) -> Option<String> {
    let tokens = server
        .token(
            BASE,
            TokenRequest {
                grant_type: Some("refresh_token".into()),
                refresh_token: Some(refresh.into()),
                client_id: Some(client_id.into()),
                ..Default::default()
            },
        )
        .await
        .ok()?;
    Some(tokens["refresh_token"].as_str().unwrap().to_string())
}

async fn refresh_rotation_never_outlives_a_revoke(store: Arc<dyn AuthStore>) {
    let client = seed(&store).await;
    let server = OAuthServer::new(store.clone());
    for round in 0..ROUNDS {
        let code = issue_code(&server, &client).await;
        let mut refresh = exchange(&server, &client.client_id, code).await.unwrap();
        let revoked = Arc::new(AtomicBool::new(false));
        let rotator = {
            let server = server.clone();
            let client_id = client.client_id.clone();
            let revoked = revoked.clone();
            tokio::spawn(async move {
                loop {
                    let after_revoke = revoked.load(Ordering::SeqCst);
                    match rotate(&server, &client_id, &refresh).await {
                        Some(next) if after_revoke => return Err(next),
                        Some(next) => refresh = next,
                        None => return Ok(refresh),
                    }
                }
            })
        };
        tokio::time::sleep(Duration::from_micros(200 * (round as u64 % 10))).await;
        store
            .revoke_client_tokens("ada", &client.client_id)
            .await
            .unwrap();
        revoked.store(true, Ordering::SeqCst);
        let last = rotator
            .await
            .unwrap()
            .unwrap_or_else(|_| panic!("round {round}: the client kept rotating after the revoke"));

        assert!(
            store
                .list_connected_clients("ada", now())
                .await
                .unwrap()
                .is_empty(),
            "round {round}: tokens issued during the revoke survived it"
        );
        assert!(rotate(&server, &client.client_id, &last).await.is_none());
    }
}

/// Codes issued before a disconnect and exchanged while it runs either lose
/// their tokens to the revoke or fail with `invalid_grant`.
async fn code_exchange_never_outlives_a_revoke(store: Arc<dyn AuthStore>) {
    let client = seed(&store).await;
    let server = OAuthServer::new(store.clone());
    for round in 0..ROUNDS {
        let mut exchanges = Vec::new();
        for _ in 0..4 {
            let code = issue_code(&server, &client).await;
            let server = server.clone();
            let client_id = client.client_id.clone();
            exchanges.push(tokio::spawn(async move {
                exchange(&server, &client_id, code).await
            }));
        }
        tokio::time::sleep(Duration::from_micros(200 * (round as u64 % 10))).await;
        store
            .revoke_client_tokens("ada", &client.client_id)
            .await
            .unwrap();
        let mut issued = Vec::new();
        for exchange in exchanges {
            issued.extend(exchange.await.unwrap());
        }

        assert!(
            store
                .list_connected_clients("ada", now())
                .await
                .unwrap()
                .is_empty(),
            "round {round}: tokens issued during the revoke survived it"
        );
        for refresh in issued {
            assert!(rotate(&server, &client.client_id, &refresh).await.is_none());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_refresh_rotation_never_outlives_a_revoke() {
    let (_dir, db) = common::sqlite_database().await;
    refresh_rotation_never_outlives_a_revoke(db.repositories().await.auth).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_refresh_rotation_never_outlives_a_revoke() {
    let Some((_container, db)) = common::postgres_database("PostgreSQL (OAuth revoke race)").await
    else {
        return;
    };
    refresh_rotation_never_outlives_a_revoke(db.repositories().await.auth).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_code_exchange_never_outlives_a_revoke() {
    let (_dir, db) = common::sqlite_database().await;
    code_exchange_never_outlives_a_revoke(db.repositories().await.auth).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_code_exchange_never_outlives_a_revoke() {
    let Some((_container, db)) = common::postgres_database("PostgreSQL (OAuth revoke race)").await
    else {
        return;
    };
    code_exchange_never_outlives_a_revoke(db.repositories().await.auth).await;
}
