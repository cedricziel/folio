//! Settings-driven access to the configured OCR and embedding providers

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;

use folio_core::error::Result;
use folio_core::models::{EmbeddingSettings, OcrSettings, PipelineSettings};
use folio_core::ports::{
    EmbeddingBatch, EmbeddingService, OcrOutput, OcrService, SettingsRepository,
};

use super::ocr::TextLayerFirst;
use super::registry::{EmbeddingRegistry, OcrRegistry, ProviderContext};
use super::{embeddings, ocr};
use crate::config::AiConfig;

const SETTINGS_TTL: Duration = Duration::from_secs(5);

/// Loads pipeline settings (database first, `config.toml` keys as fallback)
/// and builds providers from the registries
pub struct PipelineContext {
    settings_repo: Arc<dyn SettingsRepository>,
    fallback: AiConfig,
    http: reqwest::Client,
    ocr_registry: OcrRegistry,
    embedding_registry: EmbeddingRegistry,
    cache: RwLock<Option<(Instant, Arc<PipelineSettings>)>>,
}

impl PipelineContext {
    pub fn new(settings_repo: Arc<dyn SettingsRepository>, fallback: AiConfig) -> Arc<Self> {
        Arc::new(Self {
            settings_repo,
            fallback,
            http: reqwest::Client::new(),
            ocr_registry: ocr::builtin(),
            embedding_registry: embeddings::builtin(),
            cache: RwLock::new(None),
        })
    }

    /// Ids of the registered OCR providers
    pub fn ocr_providers(&self) -> Vec<&'static str> {
        self.ocr_registry.ids()
    }

    /// Ids of the registered embedding providers
    pub fn embedding_providers(&self) -> Vec<&'static str> {
        self.embedding_registry.ids()
    }

    /// Current settings, cached for a few seconds
    pub async fn settings(&self) -> Arc<PipelineSettings> {
        if let Some((at, settings)) = self.cache.read().await.as_ref() {
            if at.elapsed() < SETTINGS_TTL {
                return settings.clone();
            }
        }

        let rows = self.settings_repo.list().await.unwrap_or_default();
        let settings = Arc::new(self.with_fallbacks(PipelineSettings::from_settings(&rows)));
        *self.cache.write().await = Some((Instant::now(), settings.clone()));
        settings
    }

    /// Drop cached settings so the next read sees fresh values
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }

    /// Fill empty API keys from `config.toml`
    pub fn with_fallbacks(&self, mut settings: PipelineSettings) -> PipelineSettings {
        if settings.ai.openai_api_key.is_empty() {
            settings.ai.openai_api_key = self.fallback.openai_api_key.clone().unwrap_or_default();
        }
        if settings.ai.anthropic_api_key.is_empty() {
            settings.ai.anthropic_api_key =
                self.fallback.anthropic_api_key.clone().unwrap_or_default();
        }
        settings
    }

    /// Build the configured OCR provider, wrapped with the PDF text-layer fast path
    pub fn build_ocr(&self, settings: &PipelineSettings) -> Result<Arc<dyn OcrService>> {
        let ctx = self.provider_context(settings);
        let engine = self.ocr_registry.create(&settings.ocr.provider, &ctx)?;
        if settings.ocr.provider == "none" {
            return Ok(engine);
        }
        Ok(Arc::new(TextLayerFirst::new(engine)))
    }

    /// Build the configured embedding provider
    pub fn build_embeddings(
        &self,
        settings: &PipelineSettings,
    ) -> Result<Arc<dyn EmbeddingService>> {
        let ctx = self.provider_context(settings);
        self.embedding_registry
            .create(&settings.embeddings.provider, &ctx)
    }

    fn provider_context(&self, settings: &PipelineSettings) -> ProviderContext {
        ProviderContext {
            settings: settings.clone(),
            http: self.http.clone(),
        }
    }
}

/// Holds the last built provider and rebuilds it only when its settings change
struct Memo<K, T> {
    slot: Mutex<Option<(K, T)>>,
}

impl<K: PartialEq + Clone, T: Clone> Memo<K, T> {
    fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    fn get_or_build(&self, key: K, build: impl FnOnce() -> Result<T>) -> Result<T> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_key, value)) = slot.as_ref() {
            if *cached_key == key {
                return Ok(value.clone());
            }
        }
        let value = build()?;
        *slot = Some((key, value.clone()));
        Ok(value)
    }
}

/// OCR service that follows the `ocr.*` settings
pub struct DynamicOcrService {
    pipeline: Arc<PipelineContext>,
    current: Memo<(OcrSettings, String, String), Arc<dyn OcrService>>,
}

impl DynamicOcrService {
    pub fn new(pipeline: Arc<PipelineContext>) -> Self {
        Self {
            pipeline,
            current: Memo::new(),
        }
    }

    async fn service(&self) -> Result<Arc<dyn OcrService>> {
        let settings = self.pipeline.settings().await;
        let key = (
            settings.ocr.clone(),
            settings.ai.openai_api_key.clone(),
            settings.ai.anthropic_api_key.clone(),
        );
        self.current
            .get_or_build(key, || self.pipeline.build_ocr(&settings))
    }
}

#[async_trait]
impl OcrService for DynamicOcrService {
    async fn extract(&self, data: &[u8], mime_type: &str) -> Result<OcrOutput> {
        self.service().await?.extract(data, mime_type).await
    }

    async fn health(&self) -> Result<()> {
        self.service().await?.health().await
    }
}

/// Embedding service that follows the `embeddings.*` settings
pub struct DynamicEmbeddingService {
    pipeline: Arc<PipelineContext>,
    current: Memo<(EmbeddingSettings, String), Arc<dyn EmbeddingService>>,
}

impl DynamicEmbeddingService {
    pub fn new(pipeline: Arc<PipelineContext>) -> Self {
        Self {
            pipeline,
            current: Memo::new(),
        }
    }

    async fn service(&self) -> Result<Arc<dyn EmbeddingService>> {
        let settings = self.pipeline.settings().await;
        let key = (
            settings.embeddings.clone(),
            settings.ai.openai_api_key.clone(),
        );
        self.current
            .get_or_build(key, || self.pipeline.build_embeddings(&settings))
    }
}

#[async_trait]
impl EmbeddingService for DynamicEmbeddingService {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
        self.service().await?.embed(texts).await
    }

    async fn model_id(&self) -> Result<String> {
        self.service().await?.model_id().await
    }

    async fn health(&self) -> Result<()> {
        self.service().await?.health().await
    }
}
