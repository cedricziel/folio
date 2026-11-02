use std::sync::Arc;

use folio_auth::{crypto::hash_token, AuthStore, PersonalToken, TokenValidator, User};
use folio_infra::{create_pool, run_migrations, SqliteAuthStore};

async fn store() -> (tempfile::TempDir, sqlx::SqlitePool, Arc<dyn AuthStore>) {
    let dir = tempfile::tempdir().unwrap();
    let pool = create_pool(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("auth.db").display()
    ))
    .await
    .unwrap();
    run_migrations(&pool).await.unwrap();
    let store: Arc<dyn AuthStore> = Arc::new(SqliteAuthStore::new(pool.clone()));
    for id in ["alice", "bob"] {
        store
            .create_user(User {
                id: id.into(),
                username: id.into(),
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
    }
    (dir, pool, store)
}

fn token() -> PersonalToken {
    PersonalToken {
        id: "pat-id".into(),
        user_id: "alice".into(),
        name: "Script".into(),
        token_hash: hash_token("folio_pat_secret"),
        prefix: "folio_pat_secr".into(),
        audience: "http://folio.test/api".into(),
        scopes: vec!["documents:write".into()],
        created_at: 1,
        expires_at: i64::MAX,
        revoked_at: None,
    }
}

#[tokio::test]
async fn personal_token_lifecycle_is_owner_isolated() {
    let (_dir, _pool, store) = store().await;
    store.insert_personal_token(token()).await.unwrap();
    assert!(store.list_personal_tokens("bob").await.unwrap().is_empty());
    assert!(!store
        .revoke_personal_token("bob", "pat-id", 10)
        .await
        .unwrap());
    assert_eq!(
        store.list_personal_tokens("alice").await.unwrap()[0].revoked_at,
        None
    );
    assert!(store
        .revoke_personal_token("alice", "pat-id", 10)
        .await
        .unwrap());
    assert_eq!(
        store
            .find_personal_token(&hash_token("folio_pat_secret"))
            .await
            .unwrap()
            .unwrap()
            .revoked_at,
        Some(10)
    );
}

#[tokio::test]
async fn validator_preserves_explicit_scopes_and_checks_audience_and_revocation() {
    let (_dir, _pool, store) = store().await;
    store.insert_personal_token(token()).await.unwrap();
    let headers = axum::http::HeaderMap::from_iter([(
        axum::http::header::AUTHORIZATION,
        "Bearer folio_pat_secret".parse().unwrap(),
    )]);
    let api = TokenValidator::new(store.clone(), Some("http://folio.test".into()), "/api");
    let caller = api.validate(&headers).await.unwrap();
    assert_eq!(caller.user_id, "alice");
    assert!(caller.has_scope("documents:write"));
    assert!(!caller.has_scope("documents:read"));
    assert!(
        TokenValidator::new(store.clone(), Some("http://folio.test".into()), "/mcp")
            .validate(&headers)
            .await
            .is_none()
    );
    store
        .revoke_personal_token("alice", "pat-id", 10)
        .await
        .unwrap();
    assert!(api.validate(&headers).await.is_none());
}

#[tokio::test]
async fn expired_and_deleted_owner_tokens_are_rejected() {
    let (_dir, pool, store) = store().await;
    let mut expired = token();
    expired.expires_at = 1;
    store.insert_personal_token(expired).await.unwrap();
    let headers = axum::http::HeaderMap::from_iter([(
        axum::http::header::AUTHORIZATION,
        "Bearer folio_pat_secret".parse().unwrap(),
    )]);
    let api = TokenValidator::new(store.clone(), Some("http://folio.test".into()), "/api");
    assert!(api.validate(&headers).await.is_none());
    sqlx::query("DELETE FROM users WHERE id = 'alice'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(store
        .find_personal_token(&hash_token("folio_pat_secret"))
        .await
        .unwrap()
        .is_none());
    assert!(api.validate(&headers).await.is_none());
}

#[tokio::test]
async fn postgres_personal_token_lifecycle_is_owner_isolated() {
    use folio_infra::Database;
    use testcontainers_modules::{
        postgres::Postgres,
        testcontainers::{runners::AsyncRunner, ImageExt},
    };
    let started = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            assert!(
                !std::env::var("FOLIO_REQUIRE_DOCKER").is_ok_and(|v| !v.is_empty() && v != "0"),
                "PostgreSQL is required: {error}"
            );
            eprintln!("SKIPPED PostgreSQL personal token contract: {error}");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let store = db.repositories().await.auth;
    store
        .create_user(User {
            id: "alice".into(),
            username: "alice".into(),
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
    store.insert_personal_token(token()).await.unwrap();
    assert!(store.list_personal_tokens("bob").await.unwrap().is_empty());
    assert!(!store
        .revoke_personal_token("bob", "pat-id", 10)
        .await
        .unwrap());
    assert_eq!(
        store.list_personal_tokens("alice").await.unwrap()[0].revoked_at,
        None
    );
    assert!(store
        .revoke_personal_token("alice", "pat-id", 10)
        .await
        .unwrap());
    assert_eq!(
        store
            .find_personal_token(&hash_token("folio_pat_secret"))
            .await
            .unwrap()
            .unwrap()
            .revoked_at,
        Some(10)
    );
}
