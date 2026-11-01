//! Embedding providers
//!
//! To add a provider, write a module here that implements
//! [`EmbeddingService`](folio_core::ports::EmbeddingService) and add one
//! `register` line to [`builtin`].

#[cfg(feature = "fastembed")]
mod fastembed;
mod none;
mod ollama;
mod openai;

use super::registry::EmbeddingRegistry;

/// Registry with every built-in embedding provider
pub fn builtin() -> EmbeddingRegistry {
    let mut r = EmbeddingRegistry::new("embedding");
    r.register("none", none::build)
        .register("openai", openai::build)
        .register("ollama", ollama::build);
    #[cfg(feature = "fastembed")]
    r.register("fastembed", fastembed::build);
    r
}

/// Reject a batch whose vectors do not match the input count or each other
pub(crate) fn validate_batch(
    provider: &str,
    inputs: usize,
    vectors: &[Vec<f32>],
) -> folio_core::error::Result<usize> {
    use folio_core::error::Error;

    if vectors.len() != inputs {
        return Err(Error::AiService(format!(
            "{provider} returned {} embeddings for {inputs} inputs",
            vectors.len()
        )));
    }
    let dims = vectors.first().map_or(0, Vec::len);
    if dims == 0 || vectors.iter().any(|v| v.len() != dims) {
        return Err(Error::AiService(format!(
            "{provider} returned empty or inconsistent embeddings"
        )));
    }
    Ok(dims)
}
