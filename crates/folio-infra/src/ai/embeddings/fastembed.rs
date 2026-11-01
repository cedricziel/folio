//! Fully local embeddings with ONNX models through the `fastembed` crate
//!
//! Compiled only with the `fastembed` cargo feature. Models are downloaded
//! from Hugging Face on first use and cached on disk.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use folio_core::error::{Error, Result};
use folio_core::ports::{EmbeddingBatch, EmbeddingService};

use super::validate_batch;
use crate::ai::registry::ProviderContext;

const DEFAULT_MODEL: &str = "BAAI/bge-small-en-v1.5";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn EmbeddingService>> {
    let wanted = match ctx.settings.embeddings.model.as_str() {
        "" => DEFAULT_MODEL,
        other => other,
    };
    let info = TextEmbedding::list_supported_models()
        .into_iter()
        .find(|m| m.model_code.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| {
            Error::AiService(format!("fastembed does not support the model '{wanted}'"))
        })?;

    Ok(Arc::new(FastEmbed {
        model: info.model,
        id: format!("fastembed:{}", info.model_code),
        loaded: Arc::new(Mutex::new(None)),
    }))
}

struct FastEmbed {
    model: EmbeddingModel,
    id: String,
    loaded: Arc<Mutex<Option<TextEmbedding>>>,
}

#[async_trait]
impl EmbeddingService for FastEmbed {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
        let inputs: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
        let loaded = self.loaded.clone();
        let model = self.model.clone();

        let vectors = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>> {
            let mut guard = loaded
                .lock()
                .map_err(|_| Error::AiService("fastembed lock poisoned".into()))?;
            if guard.is_none() {
                let session = TextEmbedding::try_new(
                    TextInitOptions::new(model).with_show_download_progress(false),
                )
                .map_err(|e| Error::AiService(format!("Failed to load fastembed model: {e}")))?;
                *guard = Some(session);
            }
            guard
                .as_mut()
                .expect("initialised above")
                .embed(inputs, None)
                .map_err(|e| Error::AiService(format!("fastembed failed: {e}")))
        })
        .await
        .map_err(|e| Error::AiService(format!("fastembed task failed: {e}")))??;

        let dimensions = validate_batch("fastembed", texts.len(), &vectors)?;
        Ok(EmbeddingBatch {
            vectors,
            model: self.id.clone(),
            dimensions,
        })
    }

    async fn model_id(&self) -> Result<String> {
        Ok(self.id.clone())
    }

    async fn health(&self) -> Result<()> {
        self.embed(&["health check"]).await.map(|_| ())
    }
}
