//! Embeddings through a local Ollama server

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use folio_core::error::Result;
use folio_core::ports::{EmbeddingBatch, EmbeddingService};

use super::validate_batch;
use crate::ai::http;
use crate::ai::registry::ProviderContext;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "nomic-embed-text";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn EmbeddingService>> {
    let cfg = &ctx.settings.embeddings;
    Ok(Arc::new(OllamaEmbeddings {
        client: ctx.http.clone(),
        base_url: http::base_url(&cfg.base_url, DEFAULT_BASE_URL),
        model: if cfg.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            cfg.model.clone()
        },
    }))
}

struct OllamaEmbeddings {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct Response {
    embeddings: Vec<Vec<f32>>,
}

#[async_trait]
impl EmbeddingService for OllamaEmbeddings {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
        let response = self
            .client
            .post(format!("{}/api/embed", self.base_url))
            .json(&Request {
                model: &self.model,
                input: texts,
            })
            .send()
            .await
            .map_err(|e| http::request_error("Ollama", e))?;
        let result: Response = http::ensure_success(response, "Ollama")
            .await?
            .json()
            .await
            .map_err(|e| http::parse_error("Ollama", e))?;

        let dimensions = validate_batch("Ollama", texts.len(), &result.embeddings)?;
        Ok(EmbeddingBatch {
            vectors: result.embeddings,
            model: self.model_id().await?,
            dimensions,
        })
    }

    async fn model_id(&self) -> Result<String> {
        Ok(format!("ollama:{}", self.model))
    }

    async fn health(&self) -> Result<()> {
        http::check_ollama(&self.client, &self.base_url, &self.model).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::test_support::MockServer;
    use folio_core::models::PipelineSettings;

    fn service(server: &MockServer) -> Arc<dyn EmbeddingService> {
        let mut settings = PipelineSettings::default();
        settings.embeddings.base_url = server.url();
        settings.embeddings.model = "mxbai-embed-large".into();
        build(&ProviderContext {
            settings,
            http: reqwest::Client::new(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn embeds_a_batch() {
        let server = MockServer::start(vec![(
            200,
            r#"{"model":"x","embeddings":[[0.5,0.5],[1.0,0.0]]}"#.into(),
        )])
        .await;

        let batch = service(&server).embed(&["one", "two"]).await.unwrap();

        assert_eq!(batch.model, "ollama:mxbai-embed-large");
        assert_eq!(batch.dimensions, 2);
        assert_eq!(batch.vectors.len(), 2);
        let request = &server.requests()[0];
        assert_eq!(request.path, "/api/embed");
        assert!(request.body.contains("\"model\":\"mxbai-embed-large\""));
    }

    #[tokio::test]
    async fn health_passes_when_the_model_is_pulled() {
        let server = MockServer::start(vec![(
            200,
            r#"{"models":[{"name":"mxbai-embed-large:latest"}]}"#.into(),
        )])
        .await;
        service(&server).health().await.unwrap();
    }

    #[tokio::test]
    async fn health_fails_when_the_server_is_down() {
        let mut settings = PipelineSettings::default();
        settings.embeddings.base_url = "http://127.0.0.1:1".into();
        let svc = build(&ProviderContext {
            settings,
            http: reqwest::Client::new(),
        })
        .unwrap();
        assert!(svc.health().await.is_err());
    }
}
