mod common;

use std::io::{Cursor, Read};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    response::Response,
};
use bytes::Bytes;
use common::{harness, Harness};
use folio_core::{
    models::{
        ArchiveCompletion, ArchiveSource, ArchiveTool, CreateDocument, FileIdentity, Id,
        ValidatedArchive,
    },
    ports::keys,
};
use folio_infra::Database;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use zip::ZipArchive;

const ORIGINAL: &[u8] = b"%PDF original\x00\xff";
const ARCHIVAL: &[u8] = b"%PDF validated archive fixture";
const TEXT: &str = "Grüße\n領収書";
const FILENAME: &str = "../../unsafe\\résumé.pdf";

async fn fixture(h: &Harness) -> ArchiveSource {
    let document = h
        .state
        .document_repo
        .create_for_upload(
            h.vault,
            CreateDocument {
                title: "Scanned receipt".into(),
                original_filename: Some(FILENAME.into()),
                content: Some(TEXT.into()),
                mime_type: Some("application/pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let identity =
        FileIdentity::from_bytes(keys::original(document.id), "application/pdf", ORIGINAL);
    h.state
        .store
        .put(&identity.storage_key, Bytes::from_static(ORIGINAL))
        .await
        .unwrap();
    h.state
        .document_repo
        .finalize_verified_upload(h.vault, document.id, &identity)
        .await
        .unwrap();
    h.state
        .archival
        .current_source(h.vault, document.id)
        .await
        .unwrap()
        .unwrap()
}

async fn ready(h: &Harness, source: &ArchiveSource) -> (Id, FileIdentity) {
    let attempt = Id::new_v4();
    let file = FileIdentity::from_bytes(
        keys::revision_artifact(source.document_id, source.revision_id, attempt),
        "application/pdf",
        ARCHIVAL,
    );
    let completion = ArchiveCompletion::Ready(ValidatedArchive {
        file: file.clone(),
        profile: "PDF/A-2b".into(),
        converter: ArchiveTool {
            name: "converter".into(),
            version: "1.0".into(),
        },
        validator: ArchiveTool {
            name: "validator".into(),
            version: "2.0".into(),
        },
    });
    h.state
        .store
        .put(&file.storage_key, Bytes::from_static(ARCHIVAL))
        .await
        .unwrap();
    assert!(h
        .state
        .archival
        .enqueue_archive(source, attempt, None)
        .await
        .unwrap());
    assert!(h
        .state
        .archival
        .complete_archive(source, attempt, &completion)
        .await
        .unwrap());
    (attempt, file)
}

async fn request(h: &Harness, id: Id) -> Response {
    h.app
        .clone()
        .oneshot(
            Request::get(format!("/api/documents/{id}/export"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn read(zip: &mut ZipArchive<Cursor<Bytes>>, name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
    data
}

async fn assert_bundle(h: &Harness, source: &ArchiveSource, attempt: Option<Id>) {
    let response = request(h, source.document_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(response.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .starts_with(&format!(
            "attachment; filename=\"folio-{}.zip\"",
            source.document_id
        )));
    let length: usize = response.headers()[header::CONTENT_LENGTH]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let bytes = to_bytes(response.into_body(), 32 * 1024).await.unwrap();
    assert_eq!(bytes.len(), length);
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut names: Vec<_> = zip.file_names().collect::<Result<_, _>>().unwrap();
    names.sort_unstable();
    assert_eq!(
        names,
        if attempt.is_some() {
            vec!["archival.pdf", "manifest.json", "original", "text.txt"]
        } else {
            vec!["manifest.json", "original", "text.txt"]
        }
    );
    let manifest: Value = serde_json::from_slice(&read(&mut zip, "manifest.json")).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(
        manifest["source_revision_id"],
        source.revision_id.to_string()
    );
    assert_eq!(manifest["document"]["id"], source.document_id.to_string());
    assert_eq!(
        manifest["document"]["vault_id"],
        source.vault_id.to_string()
    );
    assert_eq!(manifest["document"]["original_filename"], FILENAME);
    let mut payloads = vec![
        (&manifest["original"], "original", ORIGINAL),
        (&manifest["text"], "text.txt", TEXT.as_bytes()),
    ];
    if let Some(attempt) = attempt {
        let archive = &manifest["archival"];
        assert_eq!(archive["attempt_id"], attempt.to_string());
        assert_eq!(archive["profile"], "PDF/A-2b");
        assert_eq!(archive["converter"]["version"], "1.0");
        assert_eq!(archive["validator"]["version"], "2.0");
        payloads.push((&archive["file"], "archival.pdf", ARCHIVAL));
    } else {
        assert!(manifest["archival"].is_null());
    }
    for (metadata, name, expected) in payloads {
        let bytes = read(&mut zip, name);
        assert_eq!(bytes, expected);
        assert_eq!(metadata["path"], name);
        assert_eq!(metadata["byte_size"], bytes.len());
        assert_eq!(metadata["sha256"], hex::encode(Sha256::digest(&bytes)));
        assert!(metadata.get("storage_key").is_none());
    }
}

#[tokio::test]
async fn exports_verified_bytes_and_omits_missing_queued_failed_or_stale_archives() {
    let h = harness().await;
    let source = fixture(&h).await;
    assert_bundle(&h, &source, None).await;

    let (attempt, _) = ready(&h, &source).await;
    assert_bundle(&h, &source, Some(attempt)).await;
    // A restore-style revision can select the same bytes. The old attempt still
    // cannot be applied to it, even though the source hash matches.
    let id = source.document_id.to_string();
    let new_revision = Id::new_v4().to_string();
    let Database::Sqlite(pool) = &h.database else {
        unreachable!()
    };
    sqlx::query(
        "INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at, restored_from_revision_id)
         VALUES (?1, ?2, ?3, 2, ?2, '2001-02-03T04:05:06Z', ?2)"
    ).bind(&new_revision).bind(&id).bind(h.vault.to_string()).execute(pool).await.unwrap();
    sqlx::query("UPDATE documents SET current_revision_id = ? WHERE id = ?")
        .bind(&new_revision)
        .bind(&id)
        .execute(pool)
        .await
        .unwrap();
    let source = h
        .state
        .archival
        .current_source(h.vault, source.document_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.revision_id.to_string(), new_revision);
    assert_bundle(&h, &source, None).await;

    let (attempt, _) = ready(&h, &source).await;
    let retry = Id::new_v4();
    assert!(h
        .state
        .archival
        .enqueue_archive(&source, retry, Some(attempt))
        .await
        .unwrap());
    assert_bundle(&h, &source, None).await;
    assert!(h
        .state
        .archival
        .complete_archive(
            &source,
            retry,
            &ArchiveCompletion::Failed("fixture failure".into())
        )
        .await
        .unwrap());
    assert_bundle(&h, &source, None).await;
}

#[tokio::test]
async fn export_fails_before_download_if_a_verified_object_is_missing_or_corrupt() {
    let h = harness().await;
    let source = fixture(&h).await;
    let (_, archive) = ready(&h, &source).await;

    h.state
        .store
        .put(&archive.storage_key, Bytes::from(vec![0; ARCHIVAL.len()]))
        .await
        .unwrap();
    assert_export_failure(&h, source.document_id).await;
    h.state.store.delete(&archive.storage_key).await.unwrap();
    assert_export_failure(&h, source.document_id).await;
    h.state
        .store
        .put(&archive.storage_key, Bytes::from_static(ARCHIVAL))
        .await
        .unwrap();
    h.state
        .store
        .put(
            &source.original.storage_key,
            Bytes::from(vec![0; ORIGINAL.len()]),
        )
        .await
        .unwrap();
    assert_export_failure(&h, source.document_id).await;
}

async fn assert_export_failure(h: &Harness, id: Id) {
    let response = request(h, id).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.headers().contains_key(header::CONTENT_DISPOSITION));
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["code"], "export_failed");
}

#[tokio::test]
async fn a_source_change_while_building_the_zip_returns_conflict_before_download() {
    use std::sync::Arc;
    let store = Arc::new(GatedStore::default());
    let h = common::harness_storing_in(store.clone()).await;
    let source = fixture(&h).await;
    let app = h.app.clone();
    let request = Request::get(format!("/api/documents/{}/export", source.document_id))
        .body(Body::empty())
        .unwrap();
    let export = tokio::spawn(app.oneshot(request));
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.read_started.notified(),
    )
    .await
    .unwrap();
    h.state
        .document_repo
        .delete(h.vault, source.document_id)
        .await
        .unwrap();
    store.resume_read.notify_one();
    let response = export.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::CONTENT_DISPOSITION));
}

struct GatedStore {
    inner: folio_infra::ObjectStoreDocumentStore,
    read_started: tokio::sync::Notify,
    resume_read: tokio::sync::Notify,
}

impl Default for GatedStore {
    fn default() -> Self {
        Self {
            inner: folio_infra::ObjectStoreDocumentStore::in_memory(),
            read_started: Default::default(),
            resume_read: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl folio_core::ports::DocumentStore for GatedStore {
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }

    async fn put(&self, key: &str, data: Bytes) -> folio_core::Result<()> {
        self.inner.put(key, data).await
    }

    async fn get(&self, key: &str) -> folio_core::Result<folio_core::ports::StoredObject> {
        self.read_started.notify_one();
        self.resume_read.notified().await;
        self.inner.get(key).await
    }

    async fn size(&self, key: &str) -> folio_core::Result<Option<u64>> {
        self.inner.size(key).await
    }

    async fn exists(&self, key: &str) -> folio_core::Result<bool> {
        self.inner.exists(key).await
    }

    async fn delete(&self, key: &str) -> folio_core::Result<()> {
        self.inner.delete(key).await
    }

    async fn delete_prefix(&self, prefix: &str) -> folio_core::Result<u64> {
        self.inner.delete_prefix(prefix).await
    }

    async fn list(&self, prefix: &str) -> folio_core::Result<Vec<String>> {
        self.inner.list(prefix).await
    }
}
