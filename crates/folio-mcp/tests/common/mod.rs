//! Shared fixture: the MCP server wired to a temp SQLite database

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use folio_core::models::{CreateDocument, Document};
use folio_core::ports::{
    DocumentRepository, EmbeddingBatch, EmbeddingService, SearchService, SettingsRepository,
};
use folio_core::services::LibraryService;
use folio_infra::{Database, ObjectStoreDocumentStore};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

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
        _id: folio_core::models::Id,
        _model: &str,
        _chunks: &[folio_core::ports::EmbeddedChunk],
    ) -> folio_core::Result<()> {
        Ok(())
    }

    async fn mark_stale_except(&self, _model: &str) -> folio_core::Result<u64> {
        Ok(0)
    }

    async fn delete_embedding(&self, _id: folio_core::models::Id) -> folio_core::Result<()> {
        Ok(())
    }

    async fn search(
        &self,
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

pub struct Fixture {
    pub library: Arc<LibraryService>,
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
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("folio.db").display());
        let db = Database::connect(&url).await.unwrap();
        db.migrate().await.unwrap();
        let repos = db.repositories().await;

        let storage = ObjectStoreDocumentStore::in_memory();

        let library = Arc::new(LibraryService::new(
            repos.documents.clone(),
            repos.tags,
            repos.correspondents,
            repos.document_types,
            embeddings,
            search.unwrap_or(repos.search),
            Arc::new(storage),
            Arc::new(repos.jobs),
        ));

        Self {
            library,
            documents: repos.documents,
            settings: repos.settings,
            _dir: dir,
        }
    }

    pub async fn insert(&self, title: &str, content: &str) -> Document {
        self.documents
            .create(CreateDocument {
                title: title.to_string(),
                content: Some(content.to_string()),
                ..Default::default()
            })
            .await
            .unwrap()
    }

    pub fn settings_port(&self) -> Arc<dyn SettingsRepository> {
        self.settings.clone()
    }

    /// An rmcp client connected to the handler over an in-memory pipe
    pub async fn in_process_client(&self) -> RunningService<RoleClient, ()> {
        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        let server = folio_mcp::FolioMcp::new(self.library.clone());
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
