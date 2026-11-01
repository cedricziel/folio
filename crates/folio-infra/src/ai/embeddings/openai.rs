//! Embeddings through an OpenAI-compatible `/embeddings` endpoint
//!
//! Works with OpenAI itself and with LocalAI, vLLM, LM Studio and similar
//! servers when `embeddings.base_url` points at them.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use folio_core::error::Result;
use folio_core::ports::{EmbeddingBatch, EmbeddingService};

use super::validate_batch;
use crate::ai::http;
use crate::ai::registry::ProviderContext;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "text-embedding-3-small";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn EmbeddingService>> {
    let cfg = &ctx.settings.embeddings;
    let custom = !cfg.base_url.is_empty();
    Ok(Arc::new(OpenAiEmbeddings {
        client: ctx.http.clone(),
        base_url: http::base_url(&cfg.base_url, DEFAULT_BASE_URL),
        api_key: http::api_key(&ctx.settings.ai.openai_api_key, custom, "OpenAI")?,
        model: if cfg.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            cfg.model.clone()
        },
    }))
}

struct OpenAiEmbeddings {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct Response {
    data: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    index: usize,
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingService for OpenAiEmbeddings {
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&Request {
                model: &self.model,
                input: texts,
            })
            .send()
            .await
            .map_err(|e| http::request_error("OpenAI", e))?;
        let mut result: Response = http::ensure_success(response, "OpenAI")
            .await?
            .json()
            .await
            .map_err(|e| http::parse_error("OpenAI", e))?;

        result.data.sort_by_key(|item| item.index);
        let vectors: Vec<Vec<f32>> = result.data.into_iter().map(|i| i.embedding).collect();
        let dimensions = validate_batch("OpenAI", texts.len(), &vectors)?;

        Ok(EmbeddingBatch {
            vectors,
            model: self.model_id().await?,
            dimensions,
        })
    }

    async fn model_id(&self) -> Result<String> {
        Ok(format!("openai:{}", self.model))
    }

    async fn health(&self) -> Result<()> {
        self.embed(&["health check"]).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::test_support::MockServer;
    use folio_core::models::PipelineSettings;

    fn service(base_url: &str, model: &str) -> Arc<dyn EmbeddingService> {
        let mut settings = PipelineSettings::default();
        settings.embeddings.base_url = base_url.into();
        settings.embeddings.model = model.into();
        build(&ProviderContext {
            settings,
            http: reqwest::Client::new(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn embeds_with_the_configured_model_and_reports_dimensions() {
        let server = MockServer::start(vec![(
            200,
            r#"{"data":[{"index":1,"embedding":[0.0,1.0,0.0]},{"index":0,"embedding":[1.0,0.0,0.0]}]}"#
                .into(),
        )])
        .await;

        let batch = service(&server.url(), "bge-m3")
            .embed(&["a", "b"])
            .await
            .unwrap();

        assert_eq!(batch.model, "openai:bge-m3");
        assert_eq!(batch.dimensions, 3);
        assert_eq!(batch.vectors[0], vec![1.0, 0.0, 0.0]);
        let request = &server.requests()[0];
        assert_eq!(request.path, "/embeddings");
        assert!(request.body.contains("\"model\":\"bge-m3\""));
        assert!(request.body.contains("\"input\":[\"a\",\"b\"]"));
    }

    #[tokio::test]
    async fn api_errors_surface_the_body() {
        let server = MockServer::start(vec![(401, "bad key".into())]).await;
        let err = service(&server.url(), "")
            .embed(&["a"])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("401") && err.contains("bad key"), "{err}");
    }

    #[tokio::test]
    async fn mismatched_vector_count_is_an_error() {
        let server =
            MockServer::start(vec![(200, r#"{"data":[{"embedding":[1.0]}]}"#.into())]).await;
        assert!(service(&server.url(), "").embed(&["a", "b"]).await.is_err());
    }

    #[test]
    fn hosted_endpoint_requires_a_key() {
        let ctx = ProviderContext {
            settings: PipelineSettings::default(),
            http: reqwest::Client::new(),
        };
        assert!(build(&ctx).is_err());
    }
}
