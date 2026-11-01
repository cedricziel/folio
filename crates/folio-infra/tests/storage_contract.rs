//! One behavioural contract for every storage backend: local directory,
//! in-memory, and S3 (MinIO via testcontainers).

mod common;

use std::time::Duration;

use bytes::Bytes;
use folio_core::error::Error;
use folio_core::models::CreateDocument;
use folio_core::ports::{keys, purge_document, DocumentStore};
use folio_infra::config::{S3Config, StorageBackend, StorageConfig};
use folio_infra::{build_store, migrate_legacy_layout, Database, ObjectStoreDocumentStore};
use futures::StreamExt;
use testcontainers_modules::testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{GenericImage, ImageExt};
use uuid::Uuid;

async fn contract(store: &dyn DocumentStore) {
    let id = Uuid::new_v4();
    let key = keys::original(id);

    // put / get / size
    assert!(!store.exists(&key).await.unwrap());
    store.put(&key, Bytes::from_static(b"hello")).await.unwrap();
    assert!(store.exists(&key).await.unwrap());
    let object = store.get(&key).await.unwrap();
    assert_eq!(object.size, Some(5));
    assert_eq!(object.into_bytes().await.unwrap(), b"hello");

    // overwrite
    store
        .put(&key, Bytes::from_static(b"replaced!"))
        .await
        .unwrap();
    assert_eq!(store.get_bytes(&key).await.unwrap(), b"replaced!");

    // missing objects
    let missing = store.get(&keys::thumbnail(id)).await.err().unwrap();
    assert!(matches!(missing, Error::NotFound { .. }), "{missing:?}");
    store.delete(&keys::thumbnail(id)).await.unwrap();

    // large objects stream in more than one chunk
    let big = vec![7u8; 3 * 1024 * 1024];
    store
        .put(&keys::page(id, 2), Bytes::from(big.clone()))
        .await
        .unwrap();
    let mut object = store.get(&keys::page(id, 2)).await.unwrap();
    assert_eq!(object.size, Some(big.len() as u64));
    let mut received = 0;
    while let Some(chunk) = object.stream.next().await {
        received += chunk.unwrap().len();
    }
    assert_eq!(received, big.len());

    // temp copy for tools that need a path; removed on drop
    let copy = store.download_to_temp(&key).await.unwrap();
    let path = copy.path().to_path_buf();
    assert_eq!(std::fs::read(&path).unwrap(), b"replaced!");
    drop(copy);
    assert!(!path.exists());
    assert!(store
        .download_to_temp("documents/none/original")
        .await
        .is_err());

    // prefixes are directory-aligned: `a` must not capture `ab`
    let other = Uuid::new_v4();
    store
        .put(&keys::original(other), Bytes::from_static(b"x"))
        .await
        .unwrap();
    let mut listed = store.list(&keys::document_prefix(id)).await.unwrap();
    listed.sort();
    assert_eq!(listed, [keys::original(id), keys::page(id, 2)]);
    assert_eq!(
        store
            .delete_prefix(&keys::document_prefix(id))
            .await
            .unwrap(),
        2
    );
    assert!(!store.exists(&key).await.unwrap());
    assert!(store.exists(&keys::original(other)).await.unwrap());
    assert_eq!(
        store
            .delete_prefix(&keys::document_prefix(id))
            .await
            .unwrap(),
        0
    );

    // keys cannot escape the store
    assert!(store
        .put("documents/../escape", Bytes::new())
        .await
        .is_err());
    assert!(store.get("../escape").await.is_err());

    // purge removes the new layout, the legacy thumbnails and the legacy original
    let doc = Uuid::new_v4();
    let legacy_original = format!("{doc}/scan.pdf");
    for k in [
        keys::original(doc),
        keys::thumbnail(doc),
        keys::page(doc, 2),
        keys::legacy::page(doc, 2),
        legacy_original.clone(),
    ] {
        store.put(&k, Bytes::from_static(b"x")).await.unwrap();
    }
    purge_document(store, doc, Some(&legacy_original), Some(2))
        .await
        .unwrap();
    for k in [
        keys::original(doc),
        keys::thumbnail(doc),
        keys::page(doc, 2),
        keys::legacy::page(doc, 2),
        legacy_original,
    ] {
        assert!(!store.exists(&k).await.unwrap(), "{k} survived the purge");
    }
    assert!(store.exists(&keys::original(other)).await.unwrap());
}

#[tokio::test]
async fn in_memory_satisfies_the_contract() {
    contract(&ObjectStoreDocumentStore::in_memory()).await;
}

#[tokio::test]
async fn local_directory_satisfies_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let store = build_store(&StorageConfig {
        backend: StorageBackend::Local,
        path: dir
            .path()
            .join("nested/root")
            .to_string_lossy()
            .into_owned(),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(store.backend(), "local");
    contract(store.as_ref()).await;
}

#[tokio::test]
async fn local_store_has_no_presigned_urls() {
    let store = ObjectStoreDocumentStore::in_memory();
    assert!(store
        .presigned_url("documents/x/original", Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
}

#[test]
fn s3_requires_a_bucket() {
    let err = ObjectStoreDocumentStore::s3(&S3Config::default())
        .err()
        .unwrap();
    assert!(err.to_string().contains("storage.s3.bucket"), "{err}");
}

#[tokio::test]
async fn s3_satisfies_the_contract_on_minio() {
    // Docker Hub no longer serves minio/minio, so use Chainguard's build.
    let started = GenericImage::new("cgr.dev/chainguard/minio", "latest")
        .with_exposed_port(9000.tcp())
        .with_wait_for(WaitFor::message_on_stderr("API:"))
        .with_cmd(["server", "/data"])
        .with_env_var("MINIO_ROOT_USER", common::MINIO_USER)
        .with_env_var("MINIO_ROOT_PASSWORD", common::MINIO_PASSWORD)
        .start()
        .await;
    let Some(container) = common::container_or_skip("MinIO", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(9000).await.unwrap();
    let endpoint = format!("http://{host}:{port}");
    common::create_bucket(&endpoint, "folio-test").await;

    let config = |prefix: Option<&str>| S3Config {
        bucket: Some("folio-test".into()),
        region: Some("us-east-1".into()),
        endpoint: Some(endpoint.clone()),
        access_key_id: Some(common::MINIO_USER.into()),
        secret_access_key: Some(common::MINIO_PASSWORD.into()),
        path_style: true,
        allow_http: true,
        prefix: prefix.map(str::to_string),
    };

    let store = ObjectStoreDocumentStore::s3(&config(None)).unwrap();
    assert_eq!(store.backend(), "s3");
    contract(&store).await;

    // presigned URLs work without credentials
    let key = keys::original(Uuid::new_v4());
    store
        .put(&key, Bytes::from_static(b"signed"))
        .await
        .unwrap();
    let url = store
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap()
        .expect("S3 supports presigned URLs");
    let body = reqwest::get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(&body[..], b"signed");

    // a prefix isolates tenants sharing a bucket, also for presigned URLs
    let prefixed = ObjectStoreDocumentStore::s3(&config(Some("/tenant-a/"))).unwrap();
    contract(&prefixed).await;
    prefixed
        .put(&key, Bytes::from_static(b"scoped"))
        .await
        .unwrap();
    assert_eq!(store.get_bytes(&key).await.unwrap(), b"signed");
    let url = prefixed
        .presigned_url(&key, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert!(url.contains("tenant-a"), "{url}");
    let body = reqwest::get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(&body[..], b"scoped");
}

#[tokio::test]
async fn legacy_layout_is_migrated_to_object_keys() {
    let dir = tempfile::tempdir().unwrap();
    let database = Database::connect(&format!("sqlite:{}", dir.path().join("folio.db").display()))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let store = ObjectStoreDocumentStore::in_memory();

    let legacy = repos
        .documents
        .create(CreateDocument {
            title: "legacy".into(),
            content: None,
            original_filename: Some("scan.pdf".into()),
            mime_type: Some("application/pdf".into()),
            correspondent_id: None,
            document_type_id: None,
        })
        .await
        .unwrap();
    let legacy_path = format!("{}/scan.pdf", legacy.id);
    repos
        .documents
        .set_file_path(legacy.id, &legacy_path)
        .await
        .unwrap();
    repos.documents.set_page_count(legacy.id, 2).await.unwrap();
    store
        .put(&legacy_path, Bytes::from_static(b"pdf"))
        .await
        .unwrap();
    store
        .put(
            &keys::legacy::thumbnail(legacy.id),
            Bytes::from_static(b"t1"),
        )
        .await
        .unwrap();
    store
        .put(&keys::legacy::page(legacy.id, 2), Bytes::from_static(b"t2"))
        .await
        .unwrap();

    let current = repos
        .documents
        .create(CreateDocument {
            title: "current".into(),
            content: None,
            original_filename: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
        })
        .await
        .unwrap();
    repos
        .documents
        .set_file_path(current.id, &keys::original(current.id))
        .await
        .unwrap();
    store
        .put(&keys::original(current.id), Bytes::from_static(b"new"))
        .await
        .unwrap();

    assert_eq!(
        migrate_legacy_layout(&store, repos.documents.as_ref())
            .await
            .unwrap(),
        1
    );

    let moved = repos
        .documents
        .find_by_id(legacy.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.file_path, Some(keys::original(legacy.id)));
    assert_eq!(
        store.get_bytes(&keys::original(legacy.id)).await.unwrap(),
        b"pdf"
    );
    assert_eq!(
        store.get_bytes(&keys::thumbnail(legacy.id)).await.unwrap(),
        b"t1"
    );
    assert_eq!(
        store.get_bytes(&keys::page(legacy.id, 2)).await.unwrap(),
        b"t2"
    );
    assert!(!store.exists(&legacy_path).await.unwrap());
    assert!(!store
        .exists(&keys::legacy::thumbnail(legacy.id))
        .await
        .unwrap());

    // second run is a no-op
    assert_eq!(
        migrate_legacy_layout(&store, repos.documents.as_ref())
            .await
            .unwrap(),
        0
    );
}
