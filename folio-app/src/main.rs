//! Folio Application - Main entry point

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use folio_api::{create_router, AppState};
use folio_infra::{
    create_pool, run_migrations, AiServiceFactory, Config, FileStorage,
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteSearchService, SqliteTagRepository,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Folio...");

    // Load configuration
    let config = Config::load().context("Failed to load configuration")?;

    info!("Configuration loaded");
    info!("  Server: {}:{}", config.server.host, config.server.port);
    info!("  Database: {}", config.database.url);
    info!("  Storage: {}", config.storage.path);
    info!("  AI Provider: {}", config.ai.provider);

    // Initialize database
    let pool = create_pool(&config.database.url)
        .await
        .context("Failed to connect to database")?;

    info!("Connected to database");

    // Run migrations
    run_migrations(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");

    // Initialize storage
    let storage = FileStorage::new(&config.storage.path);
    storage
        .init()
        .await
        .context("Failed to initialize storage")?;

    info!("Storage initialized at: {}", config.storage.path);

    // Create repositories
    let document_repo = Arc::new(SqliteDocumentRepository::new(pool.clone()));
    let tag_repo = Arc::new(SqliteTagRepository::new(pool.clone()));
    let correspondent_repo = Arc::new(SqliteCorrespondentRepository::new(pool.clone()));
    let document_type_repo = Arc::new(SqliteDocumentTypeRepository::new(pool.clone()));

    // Create AI services
    let ocr_service = AiServiceFactory::create_ocr_service(&config.ai);
    let embedding_service = AiServiceFactory::create_embedding_service(&config.ai);
    let search_service = Arc::new(SqliteSearchService::new(pool.clone()));

    info!("Services initialized");

    // Create application state
    let state = AppState::new(
        document_repo,
        tag_repo,
        correspondent_repo,
        document_type_repo,
        ocr_service,
        embedding_service,
        search_service,
        config.storage.path.clone(),
    );

    // Create router
    let app = create_router(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind to {}", addr))?;

    info!("🚀 Folio is running at http://{}", addr);

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}
