//! AI adapters: OCR and embedding providers behind a registry
//!
//! See `docs/pipeline.md` for how providers are selected and added.

pub mod chunker;
pub mod embeddings;
pub(crate) mod http;
pub mod ocr;
pub mod pdf;
pub mod pipeline;
pub mod registry;
mod search;

#[cfg(test)]
pub(crate) mod test_support;

pub use pipeline::{DynamicEmbeddingService, DynamicOcrService, PipelineContext};
pub use registry::{EmbeddingRegistry, OcrRegistry, ProviderContext, Registry};
pub use search::SqliteSearchService;
