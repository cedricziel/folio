mod common;

use std::sync::Arc;

use axum::{
    body::{Body, Bytes},
    http::{header, Request, StatusCode},
    response::Response,
};
use folio_api::{create_router, AuthState};
use folio_auth::{crypto::hash_token, AccessToken, OAuthClient, User};
use folio_core::{
    models::{CreateDocument, Principal, UserId, VaultId, VaultRole},
    ports::{keys, DocumentStore, StoredObject},
    Result,
};
use tower::ServiceExt;
use uuid::Uuid;

use common::{harness, harness_requiring_login, text, Harness};

async fn document(h: &Harness, vault: VaultId) -> Uuid {
    h.state
        .document_repo
        .create(
            vault,
            CreateDocument {
                title: "Thumbnail fixture".into(),
                mime_type: Some("image/png".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

async fn get(h: &Harness, id: Uuid, token: Option<&str>, vault: Option<VaultId>) -> Response {
    let mut request = Request::get(format!("/api/documents/{id}/thumbnail"));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(vault) = vault {
        request = request.header("x-folio-vault", vault.to_string());
    }
    h.app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn thumbnail_returns_stored_jpeg_with_private_cache_and_hardening() {
    let h = harness().await;
    let id = document(&h, h.vault).await;
    h.state
        .store
        .put(&keys::thumbnail(id), Bytes::from_static(b"jpeg thumbnail"))
        .await
        .unwrap();
    let response = get(&h, id, None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "14");
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "inline");
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; sandbox"
    );
    assert_eq!(text(response).await, "jpeg thumbnail");
}

#[tokio::test]
async fn thumbnail_does_not_fall_back_to_original_or_serve_unknown_or_trashed_documents() {
    let h = harness().await;
    let id = document(&h, h.vault).await;
    let original = keys::original(id);
    h.state
        .store
        .put(&original, Bytes::from_static(b"large original image"))
        .await
        .unwrap();
    h.state
        .document_repo
        .set_file_path(h.vault, id, &original)
        .await
        .unwrap();
    let absent = get(&h, id, None, None).await;
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);
    assert_eq!(absent.headers()[header::CACHE_CONTROL], "private, no-store");
    assert_eq!(absent.headers()[header::CONTENT_TYPE], "application/json");
    assert!(!text(absent).await.contains("large original image"));

    let orphan = Uuid::new_v4();
    for key in [keys::thumbnail(id), keys::thumbnail(orphan)] {
        h.state
            .store
            .put(&key, Bytes::from_static(b"retained thumbnail"))
            .await
            .unwrap();
    }
    h.state.document_repo.delete(h.vault, id).await.unwrap();
    for hidden in [id, orphan] {
        let response = get(&h, hidden, None, None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!text(response).await.contains("retained thumbnail"));
    }
}

struct UnavailableStore {
    fail_during_read: bool,
}

#[async_trait::async_trait]
impl DocumentStore for UnavailableStore {
    fn backend(&self) -> &'static str {
        "unavailable-test-store"
    }
    async fn get(&self, _: &str) -> Result<StoredObject> {
        let error = folio_core::Error::Storage("private backend details".into());
        if self.fail_during_read {
            Ok(StoredObject {
                size: Some(14),
                stream: Box::pin(futures::stream::once(async { Err(error) })),
            })
        } else {
            Err(error)
        }
    }
    async fn put(&self, _: &str, _: Bytes) -> Result<()> {
        unreachable!()
    }
    async fn size(&self, _: &str) -> Result<Option<u64>> {
        unreachable!()
    }
    async fn exists(&self, _: &str) -> Result<bool> {
        unreachable!()
    }
    async fn delete(&self, _: &str) -> Result<()> {
        unreachable!()
    }
    async fn delete_prefix(&self, _: &str) -> Result<u64> {
        unreachable!()
    }
    async fn list(&self, _: &str) -> Result<Vec<String>> {
        unreachable!()
    }
}

#[tokio::test]
async fn thumbnail_reports_storage_failure_without_leaking_backend_details() {
    let mut h = harness().await;
    let id = document(&h, h.vault).await;
    for fail_during_read in [false, true] {
        h.state.store = Arc::new(UnavailableStore { fail_during_read });
        h.app = create_router(
            h.state.clone(),
            AuthState::new(h.auth_store.clone(), None).with_required(false),
        );
        let response = get(&h, id, None, None).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert!(!text(response).await.contains("private backend details"));
    }
}

async fn account(h: &Harness, name: &str) -> (UserId, VaultId) {
    h.auth_store
        .create_user(User {
            id: name.into(),
            username: name.into(),
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
    let user = UserId::from(name);
    let vault = h.state.vaults.provision_user(&user).await.unwrap();
    (user, vault)
}

async fn token(h: &Harness, user: &UserId, scope: &str, audience: &str) -> String {
    let value = Uuid::new_v4().to_string();
    h.auth_store
        .insert_client(OAuthClient {
            client_id: value.clone(),
            client_name: "Thumbnail test".into(),
            redirect_uris: vec!["test:/callback".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    h.auth_store
        .insert_access_token(AccessToken {
            token_hash: hash_token(&value),
            client_id: value.clone(),
            user_id: user.0.clone(),
            scopes: vec![scope.into()],
            audience: audience.into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            grant_id: value.clone(),
        })
        .await
        .unwrap();
    value
}

#[tokio::test]
async fn thumbnail_requires_api_read_scope_and_authorized_vault_selection() {
    let h = harness_requiring_login().await;
    let (alice, alice_vault) = account(&h, "thumbnail-reader").await;
    let (_, bob_vault) = account(&h, "thumbnail-owner").await;
    let id = document(&h, bob_vault).await;
    h.state
        .store
        .put(
            &keys::thumbnail(id),
            Bytes::from_static(b"private thumbnail"),
        )
        .await
        .unwrap();
    let read = token(&h, &alice, "documents:read", "http://localhost/api").await;
    let write = token(&h, &alice, "documents:write", "http://localhost/api").await;
    let wrong_audience = token(&h, &alice, "documents:read", "http://localhost/mcp").await;
    for bearer in [None, Some("invalid"), Some(wrong_audience.as_str())] {
        assert_eq!(
            get(&h, id, bearer, Some(bob_vault)).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        get(&h, id, Some(&write), Some(bob_vault)).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&h, id, Some(&read), Some(bob_vault)).await.status(),
        StatusCode::NOT_FOUND
    );
    h.members
        .grant(
            bob_vault,
            &Principal::User(alice.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    for vault in [None, Some(alice_vault)] {
        assert_eq!(
            get(&h, id, Some(&read), vault).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    let allowed = get(&h, id, Some(&read), Some(bob_vault)).await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(text(allowed).await, "private thumbnail");
}
