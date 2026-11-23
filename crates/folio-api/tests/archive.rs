mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    response::Response,
};
use common::{text, vault_session, Harness};
use folio_api::{create_router, AuthState};
use folio_core::{
    models::{
        ArchiveCompletion, ArchiveSource, ArchiveTool, CreateDocument, FileIdentity,
        ValidatedArchive, VaultRole,
    },
    ports::{keys, ArchivalRepository},
};
use folio_infra::{archival::ArchiveConverter, queue::ArchiveService, Database};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGINAL: &[u8] = b"Original file bytes";
const PDF_A: &[u8] = b"Validated archival fixture";

struct Fixture {
    app: Harness,
    repository: Arc<dyn ArchivalRepository>,
    source: ArchiveSource,
    editor: String,
    viewer: String,
}

impl Fixture {
    async fn new() -> Self {
        let mut app = common::harness_requiring_login().await;
        let repository = app.database.repositories().await.archival;
        app.state.archive = Some(Arc::new(ArchiveService::new(
            app.database.clone(),
            repository.clone(),
            app.state.store.clone(),
            ArchiveConverter::new("eng"),
            false,
        )));
        app.app = create_router(
            app.state.clone(),
            AuthState::new(app.auth_store.clone(), Some("http://folio.test".into()))
                .with_required(true),
        );
        let editor = vault_session(&app, "editor", VaultRole::Editor, false).await;
        let viewer = vault_session(&app, "viewer", VaultRole::Viewer, false).await;

        let document = app
            .state
            .document_repo
            .create_for_upload(
                app.vault,
                CreateDocument {
                    title: "Invoice".into(),
                    original_filename: Some("invoice.pdf".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let identity =
            FileIdentity::from_bytes(keys::original(document.id), "application/pdf", ORIGINAL);
        app.state
            .store
            .put(&identity.storage_key, ORIGINAL.into())
            .await
            .unwrap();
        app.state
            .document_repo
            .finalize_verified_upload(app.vault, document.id, &identity)
            .await
            .unwrap();
        let source = repository
            .current_source(app.vault, document.id)
            .await
            .unwrap()
            .unwrap();
        Self {
            app,
            repository,
            source,
            editor,
            viewer,
        }
    }

    fn archive_path(&self) -> String {
        format!("/api/documents/{}/archive", self.source.document_id)
    }

    fn page_path(&self) -> String {
        format!(
            "/vaults/{}/documents/{}",
            self.app.vault, self.source.document_id
        )
    }

    async fn send(&self, request: Request<Body>) -> Response {
        self.app.app.clone().oneshot(request).await.unwrap()
    }

    async fn get(&self, path: &str, session: &str) -> Response {
        self.send(
            Request::builder()
                .uri(path)
                .header(header::COOKIE, session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn post(&self, session: &str, body: Value) -> Response {
        self.send(
            Request::builder()
                .method(Method::POST)
                .uri(self.archive_path())
                .header(header::COOKIE, session)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    async fn ready(&self) -> String {
        let attempt = Uuid::new_v4();
        let key =
            keys::revision_artifact(self.source.document_id, self.source.revision_id, attempt);
        let output = ValidatedArchive {
            file: FileIdentity::from_bytes(&key, "application/pdf", PDF_A),
            profile: "PDF/A-2b".into(),
            converter: ArchiveTool {
                name: "OCRmyPDF".into(),
                version: "1.0".into(),
            },
            validator: ArchiveTool {
                name: "veraPDF".into(),
                version: "2.0".into(),
            },
        };
        assert!(self
            .repository
            .enqueue_archive(&self.source, attempt, None)
            .await
            .unwrap());
        self.app.state.store.put(&key, PDF_A.into()).await.unwrap();
        assert!(self
            .repository
            .complete_archive(&self.source, attempt, &ArchiveCompletion::Ready(output))
            .await
            .unwrap());
        key
    }

    async fn token(&self, session: &str, scope: &str) -> String {
        let response = self.send(Request::builder().method(Method::POST)
            .uri("/api/auth/tokens")
            .header(header::COOKIE, session)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({
                "name": scope, "audience": "api", "scopes": [scope], "expires_in_days": 30
            }).to_string())).unwrap()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        json_body(response).await["token"].as_str().unwrap().into()
    }

    async fn bearer(&self, method: Method, path: &str, token: &str) -> Response {
        self.send(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
    }
}

async fn json_body(response: Response) -> Value {
    serde_json::from_str(&text(response).await).unwrap()
}

#[tokio::test]
async fn viewer_can_download_the_current_archive_and_cannot_queue_it() {
    let f = Fixture::new().await;
    let path = f.archive_path();
    assert_eq!(
        json_body(f.get(&path, &f.viewer).await).await["status"],
        "not_created"
    );
    assert_eq!(
        f.post(&f.viewer, json!({})).await.status(),
        StatusCode::FORBIDDEN
    );
    let page = text(f.get(&f.page_path(), &f.viewer).await).await;
    assert!(page.contains("Not created"));
    assert!(!page.contains("Generate archival copy"));

    f.ready().await;
    let response = f.get(&format!("{path}/download"), &f.viewer).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/pdf");
    assert_eq!(text(response).await.as_bytes(), PDF_A);
    let page = text(f.get(&f.page_path(), &f.viewer).await).await;
    assert!(page.contains("Download original"));
    assert!(page.contains("Download PDF/A"));
    assert!(page.contains("PDF/A-2b"));
}

#[tokio::test]
async fn archive_personal_tokens_require_the_matching_scope_and_vault_role() {
    let f = Fixture::new().await;
    let path = f.archive_path();
    let read = f.token(&f.editor, "documents:read").await;
    let write = f.token(&f.editor, "documents:write").await;
    let viewer_write = f.token(&f.viewer, "documents:write").await;

    assert_eq!(
        f.bearer(Method::GET, &path, &read).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        f.bearer(Method::GET, &path, &write).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.bearer(Method::POST, &path, &read).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.bearer(Method::POST, &path, &viewer_write).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.bearer(Method::POST, &path, &write).await.status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        f.bearer(Method::GET, &format!("{path}/download"), &read)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.bearer(Method::GET, &format!("{path}/download"), &write)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn web_archive_mutation_requires_same_origin_and_preserves_the_return_path() {
    let f = Fixture::new().await;
    let page = f.page_path();
    let html = text(f.get(&page, &f.editor).await).await;
    assert!(html.contains("Generate archival copy"));
    let post = |site| {
        Request::builder()
            .method(Method::POST)
            .uri(format!("{page}/archive"))
            .header(header::COOKIE, &f.editor)
            .header("sec-fetch-site", site)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("return_to=%2F%3Fview%3Dall"))
            .unwrap()
    };
    assert_eq!(
        f.send(post("cross-site")).await.status(),
        StatusCode::FORBIDDEN
    );
    let response = f.send(post("same-origin")).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(location.starts_with(&format!("{page}?return_to=")));
    let html = text(f.get(location, &f.editor).await).await;
    assert!(html.contains("Queued"));
    assert!(html.contains("Refresh archival status"));
}

#[tokio::test]
async fn archive_failure_preserves_original_and_retry_enqueues_at_most_once() {
    let f = Fixture::new().await;
    let first = f.post(&f.editor, json!({})).await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let first_id = json_body(first).await["attempt_id"].clone();
    let repeated = f.post(&f.editor, json!({})).await;
    assert_eq!(json_body(repeated).await["attempt_id"], first_id);
    assert!(f
        .repository
        .complete_archive(
            &f.source,
            first_id.as_str().unwrap().parse().unwrap(),
            &ArchiveCompletion::Failed("Could not write /private/staging/example.pdf".into())
        )
        .await
        .unwrap());

    let status = json_body(f.get(&f.archive_path(), &f.viewer).await).await;
    assert_eq!(status["status"], "failed");
    assert!(!status.to_string().contains("/private/staging"));
    let response = f
        .get(
            &format!("/api/documents/{}/download", f.source.document_id),
            &f.viewer,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(text(response).await.as_bytes(), ORIGINAL);
    let html = text(f.get(&f.page_path(), &f.editor).await).await;
    assert!(html.contains("Retry archival copy"));
    assert!(!html.contains("/private/staging"));

    let retry = json!({"expected_attempt_id": first_id});
    let second = f.post(&f.editor, retry.clone()).await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    assert_ne!(json_body(second).await["attempt_id"], first_id);
    assert_eq!(
        f.post(&f.editor, retry).await.status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn archival_download_rejects_missing_and_same_size_corrupt_bytes() {
    let f = Fixture::new().await;
    let key = f.ready().await;
    let path = format!("{}/download", f.archive_path());
    f.app
        .state
        .store
        .put(&key, vec![b'x'; PDF_A.len()].into())
        .await
        .unwrap();
    assert_eq!(
        f.get(&path, &f.viewer).await.status(),
        StatusCode::NOT_FOUND
    );
    f.app.state.store.delete(&key).await.unwrap();
    assert_eq!(
        f.get(&path, &f.viewer).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn archival_download_rejects_the_previous_revision_even_with_the_same_original_bytes() {
    let f = Fixture::new().await;
    f.ready().await;
    let Database::Sqlite(pool) = &f.app.database else {
        unreachable!()
    };
    let replacement = Uuid::new_v4().to_string();
    let document = f.source.document_id.to_string();
    sqlx::query(
        "INSERT INTO document_revisions (
        id, document_id, vault_id, number, original_file_id, created_at, restored_from_revision_id
        ) VALUES (?, ?, ?, 2, ?, '2001-02-03T04:05:06Z', ?)",
    )
    .bind(&replacement)
    .bind(&document)
    .bind(f.app.vault.to_string())
    .bind(&document)
    .bind(&document)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE documents SET current_revision_id = ? WHERE id = ?")
        .bind(&replacement)
        .bind(&document)
        .execute(pool)
        .await
        .unwrap();

    let path = f.archive_path();
    assert_eq!(
        f.get(&format!("{path}/download"), &f.viewer).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        json_body(f.get(&path, &f.viewer).await).await["status"],
        "not_created"
    );
}

#[tokio::test]
async fn archive_routes_do_not_resolve_a_document_from_another_vault() {
    let f = Fixture::new().await;
    f.ready().await;
    let user = folio_core::models::UserId::from("editor");
    let other = f.app.state.vaults.create(&user, "Other").await.unwrap();
    let path = format!("/vaults/{}{}", other.id, f.archive_path());
    assert_eq!(
        f.get(&path, &f.editor).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.get(&format!("{path}/download"), &f.editor).await.status(),
        StatusCode::NOT_FOUND
    );
    let response = f
        .send(
            Request::builder()
                .method(Method::POST)
                .uri(path)
                .header(header::COOKIE, &f.editor)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn signed_pdf_skip_explains_the_reason_and_keeps_the_original_action() {
    let f = Fixture::new().await;
    let attempt = Uuid::new_v4();
    assert!(f
        .repository
        .enqueue_archive(&f.source, attempt, None)
        .await
        .unwrap());
    assert!(f
        .repository
        .complete_archive(
            &f.source,
            attempt,
            &ArchiveCompletion::Skipped("source PDF contains a signature".into()),
        )
        .await
        .unwrap());

    let status = json_body(f.get(&f.archive_path(), &f.viewer).await).await;
    assert_eq!(status["status"], "skipped");
    assert_eq!(status["reason"], "This PDF contains a digital signature.");
    let html = text(f.get(&f.page_path(), &f.editor).await).await;
    assert!(html.contains("This PDF contains a digital signature."));
    assert!(html.contains("Download original"));
    assert!(!html.contains("Retry archival copy"));
    assert!(!html.contains("Download PDF/A"));
}
