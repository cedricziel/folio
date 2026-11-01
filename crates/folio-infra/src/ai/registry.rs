//! Provider registry: maps provider ids from settings to adapter constructors

use std::collections::BTreeMap;
use std::sync::Arc;

use folio_core::error::{Error, Result};
use folio_core::models::PipelineSettings;
use folio_core::ports::{EmbeddingService, OcrService};

/// Everything a provider constructor may need
#[derive(Clone)]
pub struct ProviderContext {
    pub settings: PipelineSettings,
    pub http: reqwest::Client,
}

/// Constructor for one provider
pub type Factory<T> = fn(&ProviderContext) -> Result<Arc<T>>;

/// Providers of one stage, keyed by the id used in settings
pub struct Registry<T: ?Sized> {
    stage: &'static str,
    factories: BTreeMap<&'static str, Factory<T>>,
}

pub type OcrRegistry = Registry<dyn OcrService>;
pub type EmbeddingRegistry = Registry<dyn EmbeddingService>;

impl<T: ?Sized> Registry<T> {
    pub fn new(stage: &'static str) -> Self {
        Self {
            stage,
            factories: BTreeMap::new(),
        }
    }

    /// Make a provider selectable under `id`
    pub fn register(&mut self, id: &'static str, factory: Factory<T>) -> &mut Self {
        self.factories.insert(id, factory);
        self
    }

    /// Ids of all registered providers, sorted
    pub fn ids(&self) -> Vec<&'static str> {
        self.factories.keys().copied().collect()
    }

    /// Build the provider registered under `id`
    pub fn create(&self, id: &str, ctx: &ProviderContext) -> Result<Arc<T>> {
        let factory = self.factories.get(id).ok_or_else(|| {
            Error::AiService(format!(
                "Unknown {} provider '{id}' (available: {})",
                self.stage,
                self.ids().join(", ")
            ))
        })?;
        factory(ctx)
    }
}
