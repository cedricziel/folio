//! Shared setup for route tests: an app on a throwaway SQLite database.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use http_body_util::BodyExt;

use folio_api::{create_router, AppState, AuthState, EventBroadcaster, IngestionState};
use folio_core::models::IngestionSourceDef;
use folio_core::ports::DocumentStore;
use folio_infra::{
    Database, DynamicEmbeddingService, DynamicOcrService, ObjectStoreDocumentStore,
    PipelineContext, ThumbnailService, WebhookClient,
};

pub struct Harness {
    _directory: tempfile::TempDir,
    pub app: Router,
    pub database: Database,
    pub state: AppState,
    pub vault: folio_core::models::VaultId,
    pub members: Arc<dyn folio_core::ports::MembershipRepository>,
    pub vault_repository: Arc<dyn folio_core::ports::VaultRepository>,
    pub auth_store: Arc<dyn folio_auth::AuthStore>,
}

pub async fn harness() -> Harness {
    harness_with_sources(Vec::new()).await
}

/// A harness that requires sign-in; the first account is created through
/// `/setup?token=`[`SETUP_TOKEN`]
pub async fn harness_requiring_login() -> Harness {
    harness_with(Vec::new(), |store| {
        AuthState::new(store, None)
            .with_setup_token(SETUP_TOKEN)
            .with_required(true)
    })
    .await
}

pub const SETUP_TOKEN: &str = "test-setup-token";

/// A harness whose configuration defines `configured` ingestion sources
pub async fn harness_with_sources(configured: Vec<IngestionSourceDef>) -> Harness {
    harness_with(configured, |store| {
        AuthState::new(store, None).with_required(false)
    })
    .await
}

/// A harness whose auth state is built by `make_auth` from the auth store
pub async fn harness_with(
    configured: Vec<IngestionSourceDef>,
    make_auth: impl FnOnce(Arc<dyn folio_auth::AuthStore>) -> AuthState,
) -> Harness {
    harness_with_push(configured, make_auth, Default::default(), None).await
}

/// Like [`harness_with`], with push notifications configured by `push`
pub async fn harness_with_push(
    configured: Vec<IngestionSourceDef>,
    make_auth: impl FnOnce(Arc<dyn folio_auth::AuthStore>) -> AuthState,
    push: folio_infra::config::PushConfig,
    // An `auth.secret_key` from configuration, which allows storing keys
    secret_key: Option<&str>,
) -> Harness {
    harness_full(
        configured,
        make_auth,
        Arc::new(ObjectStoreDocumentStore::in_memory()),
        push,
        secret_key,
    )
    .await
}

/// A harness that keeps originals in `store`, without sign-in
pub async fn harness_storing_in(store: Arc<dyn DocumentStore>) -> Harness {
    harness_full(
        Vec::new(),
        |store| AuthState::new(store, None).with_required(false),
        store,
        Default::default(),
        None,
    )
    .await
}

async fn harness_full(
    configured: Vec<IngestionSourceDef>,
    make_auth: impl FnOnce(Arc<dyn folio_auth::AuthStore>) -> AuthState,
    store: Arc<dyn DocumentStore>,
    push: folio_infra::config::PushConfig,
    secret_key: Option<&str>,
) -> Harness {
    let directory = tempfile::Builder::new()
        .prefix("folio-test-")
        .tempdir()
        .unwrap();
    let storage = directory.path().to_string_lossy().to_string();

    let database = Database::connect(&format!("sqlite:{}/folio.db", storage))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let auth_store = repos.auth.clone();
    let members = repos.members.clone();
    let vault_repository = repos.vaults.clone();
    let vaults = Arc::new(
        folio_core::services::VaultService::new(repos.vaults.clone(), repos.members)
            .with_audit(repos.audit),
    );
    let auth = make_auth(repos.auth.clone());

    let ai = folio_infra::config::AiConfig {
        provider: "local".into(),
        openai_api_key: None,
        anthropic_api_key: None,
    };
    let thumbnails = Arc::new(ThumbnailService::new(store.clone()));

    let pipeline = PipelineContext::new(repos.settings.clone(), ai);
    let push_settings = Arc::new(folio_infra::push::ConfiguredPush::new(
        repos.settings.clone(),
        push,
        secret_key.map(|key| Arc::new(folio_auth::secret::SecretBox::new(key).unwrap())),
        secret_key.is_some(),
    ));
    let push_state = folio_api::routes::devices::PushState {
        devices: repos.push.clone(),
        sender: Arc::new(
            folio_core::services::notifications::NotificationSender::new(
                repos.push.clone(),
                Arc::new(folio_core::ports::RepositoryDocuments(
                    repos.documents.clone(),
                )),
                repos.access.clone(),
                repos.grants.clone(),
                push_settings.clone(),
                push_settings.clone(),
                Default::default(),
            ),
        ),
        settings: push_settings,
    };
    let ingestion = IngestionState::new(
        repos.sources,
        repos.ledger,
        repos.ingest_outcomes,
        configured,
    );
    let state = AppState::new(
        repos.saved_views,
        repos.documents,
        repos.entries,
        repos.upload_receipts,
        repos.tags,
        repos.correspondents,
        repos.document_types,
        repos.settings.clone(),
        Arc::new(DynamicOcrService::new(pipeline.clone())),
        Arc::new(DynamicEmbeddingService::new(pipeline.clone())),
        repos.search,
        pipeline,
        thumbnails,
        repos.jobs,
        store,
        EventBroadcaster::new(16),
        repos.outbox,
        repos.workflows,
        WebhookClient::new(),
        "http://folio.test".to_string(),
        ingestion,
        vaults,
        repos.access,
    )
    .with_push(push_state);
    Harness {
        _directory: directory,
        database,
        app: create_router(state.clone(), auth),
        state,
        vault,
        members,
        vault_repository,
        auth_store,
    }
}

pub const BOUNDARY: &str = "folioboundary";

pub fn multipart(parts: &[(&str, Option<&str>, &str)]) -> Request<Body> {
    let mut body = String::new();
    for (name, filename, content) in parts {
        body.push_str(&format!("--{BOUNDARY}\r\n"));
        match filename {
            Some(f) => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\nContent-Type: image/png\r\n\r\n"
            )),
            None => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"\r\n\r\n"
            )),
        }
        body.push_str(content);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{BOUNDARY}--\r\n"));
    Request::builder()
        .method("POST")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap()
}

pub async fn text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

/// A signed-in account with `role` in the harness vault; returns its session cookie
pub async fn vault_session(
    h: &Harness,
    name: &str,
    role: folio_core::models::VaultRole,
    is_admin: bool,
) -> String {
    h.auth_store
        .create_user(folio_auth::User {
            id: name.into(),
            username: name.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    let user = folio_core::models::UserId::from(name);
    h.members
        .grant(
            h.vault,
            &folio_core::models::Principal::User(user.clone()),
            role,
            None,
        )
        .await
        .unwrap();
    h.vault_repository
        .set_default(&user, h.vault)
        .await
        .unwrap();
    let token = format!("vault-session-{name}");
    h.auth_store
        .create_session(folio_auth::Session {
            id_hash: folio_auth::crypto::hash_token(&token),
            user_id: name.into(),
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    format!("folio_session={token}")
}
