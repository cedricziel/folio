//! Shared fixture: the MCP server wired to a temp SQLite database

#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use folio_core::models::{CreateDocument, Document};
use folio_core::ports::{DocumentRepository, EmbeddingService, SettingsRepository};
use folio_core::services::LibraryService;
use folio_infra::{
    create_pool, run_migrations, FileStorage, JobQueue, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteSearchService,
    SqliteSettingsRepository, SqliteTagRepository,
};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RunningService, ServiceError};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

/// Embeddings are unavailable in tests, so search exercises the literal path
struct NoEmbeddings;

#[async_trait]
impl EmbeddingService for NoEmbeddings {
    async fn generate_embedding(&self, _text: &str) -> folio_core::Result<Vec<f32>> {
        Err(folio_core::Error::AiService("no provider".into()))
    }

    async fn generate_embeddings(&self, _texts: &[&str]) -> folio_core::Result<Vec<Vec<f32>>> {
        Err(folio_core::Error::AiService("no provider".into()))
    }

    fn embedding_dimension(&self) -> usize {
        0
    }
}

pub struct Fixture {
    pub library: Arc<LibraryService>,
    pub documents: Arc<SqliteDocumentRepository>,
    pub settings: Arc<SqliteSettingsRepository>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("folio.db").display());
        let pool = create_pool(&url).await.unwrap();
        run_migrations(&pool).await.unwrap();

        let storage = FileStorage::new(dir.path().join("data"));
        storage.init().await.unwrap();

        let documents = Arc::new(SqliteDocumentRepository::new(pool.clone()));
        let settings = Arc::new(SqliteSettingsRepository::new(pool.clone()));
        let library = Arc::new(LibraryService::new(
            documents.clone(),
            Arc::new(SqliteTagRepository::new(pool.clone())),
            Arc::new(SqliteCorrespondentRepository::new(pool.clone())),
            Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
            Arc::new(NoEmbeddings),
            Arc::new(SqliteSearchService::new(pool.clone())),
            Arc::new(storage),
            Arc::new(JobQueue::new(pool)),
        ));

        Self {
            library,
            documents,
            settings,
            _dir: dir,
        }
    }

    pub async fn insert(&self, title: &str, content: &str) -> Document {
        self.documents
            .create(CreateDocument {
                title: title.to_string(),
                content: Some(content.to_string()),
                original_filename: None,
                mime_type: None,
                correspondent_id: None,
                document_type_id: None,
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
