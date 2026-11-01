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

#[cfg(test)]
pub(crate) mod test_support;

pub use pipeline::{DynamicEmbeddingService, DynamicOcrService, PipelineContext};
pub use registry::{EmbeddingRegistry, OcrRegistry, ProviderContext, Registry};

/// Cosine similarity of two vectors; 0 when they differ in length or are zero
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}
