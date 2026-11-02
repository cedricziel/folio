//! Shared fixture: the MCP server wired to a temp SQLite database

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use folio_core::models::{CreateDocument, CreateTag, Document, Id, Tag, UpdateTag};
use folio_core::ports::{
    DocumentRepository, DocumentStore, EmbeddingBatch, EmbeddingService, SearchService,
    SettingsRepository, StoredObject, TagRepository,
};
use folio_core::services::LibraryService;
use folio_infra::{Database, ObjectStoreDocumentStore};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Embeddings are unavailable in tests, so search exercises the literal path
struct NoEmbeddings;

#[async_trait]
impl EmbeddingService for NoEmbeddings {
    async fn embed(&self, _texts: &[&str]) -> folio_core::Result<EmbeddingBatch> {
        Err(folio_core::Error::AiService("no provider".into()))
    }

    async fn model_id(&self) -> folio_core::Result<String> {
        Ok("none".into())
    }

    async fn health(&self) -> folio_core::Result<()> {
        Ok(())
    }
}

/// Embeddings that always succeed so the semantic path is taken
pub struct FakeEmbeddings;

#[async_trait]
impl EmbeddingService for FakeEmbeddings {
    async fn embed(&self, texts: &[&str]) -> folio_core::Result<EmbeddingBatch> {
        Ok(EmbeddingBatch {
            vectors: texts.iter().map(|_| vec![1.0]).collect(),
            model: "fake".into(),
            dimensions: 1,
        })
    }

    async fn model_id(&self) -> folio_core::Result<String> {
        Ok("fake".into())
    }

    async fn health(&self) -> folio_core::Result<()> {
        Ok(())
    }
}

/// Vector search returning a ranking set by the test
#[derive(Default)]
pub struct FakeSearch(std::sync::Mutex<Vec<(folio_core::models::Id, f32)>>);

impl FakeSearch {
    pub fn rank(&self, hits: Vec<(folio_core::models::Id, f32)>) {
        *self.0.lock().unwrap() = hits;
    }
}

#[async_trait]
impl SearchService for FakeSearch {
    async fn store_chunks(
        &self,
        _vault: folio_core::models::VaultId,
        _id: folio_core::models::Id,
        _model: &str,
        _chunks: &[folio_core::ports::EmbeddedChunk],
    ) -> folio_core::Result<()> {
        Ok(())
    }

    async fn mark_stale_except(&self, _model: &str) -> folio_core::Result<u64> {
        Ok(0)
    }

    async fn delete_embedding(
        &self,
        _vault: folio_core::models::VaultId,
        _id: folio_core::models::Id,
    ) -> folio_core::Result<()> {
        Ok(())
    }

    async fn search(
        &self,
        _vault: folio_core::models::VaultId,
        _model: &str,
        _embedding: &[f32],
        limit: usize,
    ) -> folio_core::Result<Vec<folio_core::ports::SearchResult>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .map(|(id, score)| folio_core::ports::SearchResult {
                document_id: id,
                score,
                snippet: None,
            })
            .collect())
    }
}

/// Tag repository that counts how often the whole table is listed
struct CountingTags {
    actors: Arc<std::sync::Mutex<Vec<Option<folio_core::models::UserId>>>>,
    inner: Arc<dyn TagRepository>,
    lists: Arc<AtomicUsize>,
    delete_after_list: Arc<AtomicBool>,
}

#[async_trait]
impl TagRepository for CountingTags {
    async fn find_by_id(
        &self,
        vault: folio_core::models::VaultId,
        id: Id,
    ) -> folio_core::Result<Option<Tag>> {
        self.inner.find_by_id(vault, id).await
    }
    async fn find_by_name(
        &self,
        vault: folio_core::models::VaultId,
        name: &str,
    ) -> folio_core::Result<Option<Tag>> {
        self.inner.find_by_name(vault, name).await
    }
    async fn list(&self, vault: folio_core::models::VaultId) -> folio_core::Result<Vec<Tag>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let tags = self.inner.list(vault).await?;
        if self.delete_after_list.swap(false, Ordering::SeqCst) {
            for tag in &tags {
                self.inner.delete(vault, tag.id).await?;
            }
        }
        Ok(tags)
    }
    async fn create(
        &self,
        vault: folio_core::models::VaultId,
        params: CreateTag,
    ) -> folio_core::Result<Tag> {
        self.actors
            .lock()
            .unwrap()
            .push(folio_core::mutation::current_actor());
        self.inner.create(vault, params).await
    }
    async fn update(
        &self,
        vault: folio_core::models::VaultId,
        id: Id,
        params: UpdateTag,
    ) -> folio_core::Result<Tag> {
        self.inner.update(vault, id, params).await
    }
    async fn delete(&self, vault: folio_core::models::VaultId, id: Id) -> folio_core::Result<()> {
        self.inner.delete(vault, id).await
    }
}

/// A store that rejects every write, to exercise upload failure handling
pub struct FailingStore(ObjectStoreDocumentStore);

#[async_trait]
impl DocumentStore for FailingStore {
    fn backend(&self) -> &'static str {
        "failing"
    }
    async fn put(&self, _key: &str, _data: bytes::Bytes) -> folio_core::Result<()> {
        Err(folio_core::Error::Storage("disk full".into()))
    }
    async fn get(&self, key: &str) -> folio_core::Result<StoredObject> {
        self.0.get(key).await
    }
    async fn size(&self, key: &str) -> folio_core::Result<Option<u64>> {
        self.0.size(key).await
    }
    async fn exists(&self, key: &str) -> folio_core::Result<bool> {
        self.0.exists(key).await
    }
    async fn delete(&self, key: &str) -> folio_core::Result<()> {
        self.0.delete(key).await
    }
    async fn delete_prefix(&self, prefix: &str) -> folio_core::Result<u64> {
        self.0.delete_prefix(prefix).await
    }
    async fn list(&self, prefix: &str) -> folio_core::Result<Vec<String>> {
        self.0.list(prefix).await
    }
}

pub struct Fixture {
    pub library: Arc<LibraryService>,
    pub vault: folio_core::models::VaultId,
    pub vaults: Arc<folio_core::services::VaultService>,
    pub vault_repo: Arc<dyn folio_core::ports::VaultRepository>,
    pub members: Arc<dyn folio_core::ports::MembershipRepository>,
    pub auth: Arc<dyn folio_auth::AuthStore>,
    /// Number of times the tags table was listed in full
    pub tag_lists: Arc<AtomicUsize>,
    pub tag_actors: Arc<std::sync::Mutex<Vec<Option<folio_core::models::UserId>>>>,
    pub delete_tags_after_next_list: Arc<AtomicBool>,
    pub documents: Arc<dyn DocumentRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        Self::with_semantic(Arc::new(NoEmbeddings), None).await
    }

    /// Like [`Fixture::new`] with custom embedding and vector search ports
    pub async fn with_semantic(
        embeddings: Arc<dyn EmbeddingService>,
        search: Option<Arc<dyn SearchService>>,
    ) -> Self {
        let store = ObjectStoreDocumentStore::in_memory();
        Self::build(embeddings, search, Arc::new(store)).await
    }

    /// Like [`Fixture::new`] with a store whose writes fail
    pub async fn with_failing_store() -> Self {
        let store = FailingStore(ObjectStoreDocumentStore::in_memory());
        Self::build(Arc::new(NoEmbeddings), None, Arc::new(store)).await
    }

    async fn build(
        embeddings: Arc<dyn EmbeddingService>,
        search: Option<Arc<dyn SearchService>>,
        storage: Arc<dyn DocumentStore>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("folio.db").display());
        let db = Database::connect(&url).await.unwrap();
        db.migrate().await.unwrap();
        let repos = db.repositories().await;
        let vault = repos.vaults.instance_default().await.unwrap();
        let vaults = Arc::new(folio_core::services::VaultService::new(
            repos.vaults.clone(),
            repos.members.clone(),
        ));

        let tag_lists = Arc::new(AtomicUsize::new(0));
        let tag_actors = Arc::new(std::sync::Mutex::new(Vec::new()));
        let delete_tags_after_next_list = Arc::new(AtomicBool::new(false));
        let tags = Arc::new(CountingTags {
            inner: repos.tags,
            lists: tag_lists.clone(),
            actors: tag_actors.clone(),
            delete_after_list: delete_tags_after_next_list.clone(),
        });
        let library = Arc::new(LibraryService::new(
            repos.documents.clone(),
            tags,
            repos.correspondents,
            repos.document_types,
            embeddings,
            search.unwrap_or(repos.search),
            storage,
            Arc::new(repos.jobs),
        ));

        Self {
            vault,
            vaults,
            vault_repo: repos.vaults,
            members: repos.members,
            auth: repos.auth,
            library,
            tag_lists,
            tag_actors,
            delete_tags_after_next_list,
            documents: repos.documents,
            settings: repos.settings,
            _dir: dir,
        }
    }

    pub async fn insert(&self, title: &str, content: &str) -> Document {
        self.documents
            .create(
                self.vault,
                CreateDocument {
                    title: title.to_string(),
                    content: Some(content.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
    }

    pub fn settings_port(&self) -> Arc<dyn SettingsRepository> {
        self.settings.clone()
    }

    /// An rmcp client connected to the handler over an in-memory pipe
    pub async fn in_process_client(&self) -> RunningService<RoleClient, ()> {
        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        let server = folio_mcp::FolioMcp::new(self.library.clone(), self.vaults.clone());
        tokio::spawn(async move {
            let running = server.serve(server_io).await.unwrap();
            running.waiting().await.unwrap();
        });
        ().serve(client_io).await.unwrap()
    }
}

pub async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> Result<CallToolResult, ServiceError> {
    let args = args.as_object().cloned().unwrap_or_default();
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
}

/// Structured output of a successful tool call
pub async fn call_ok(
    client: &RunningService<RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> Value {
    let result = call(client, tool, args).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{tool} failed: {result:?}");
    result.structured_content.unwrap_or(json!(null))
}
