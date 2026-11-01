//! OCR disabled

use std::sync::Arc;

use async_trait::async_trait;

use folio_core::error::Result;
use folio_core::ports::{OcrOutput, OcrService};

use crate::ai::registry::ProviderContext;

pub fn build(_: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    Ok(Arc::new(NoOcr))
}

/// Returns no text for every document
pub struct NoOcr;

#[async_trait]
impl OcrService for NoOcr {
    async fn extract(&self, _data: &[u8], _mime_type: &str) -> Result<OcrOutput> {
        Ok(OcrOutput {
            engine: "none".to_string(),
            ..Default::default()
        })
    }

    async fn health(&self) -> Result<()> {
        Ok(())
    }
}
