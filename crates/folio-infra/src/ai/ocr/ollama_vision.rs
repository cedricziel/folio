//! OCR through an Ollama vision model

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use folio_core::error::Result;
use folio_core::ports::OcrService;

use super::{PageEngine, PageOcr, PageText, VISION_PROMPT};
use crate::ai::http::{self, base64_encode};
use crate::ai::registry::ProviderContext;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "llama3.2-vision";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    let ocr = &ctx.settings.ocr;
    Ok(Arc::new(PageOcr(OllamaVision {
        client: ctx.http.clone(),
        base_url: http::base_url(&ocr.base_url, DEFAULT_BASE_URL),
        model: if ocr.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            ocr.model.clone()
        },
    })))
}

struct OllamaVision {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: &'static str,
    images: Vec<String>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[async_trait]
impl PageEngine for OllamaVision {
    fn engine_id(&self) -> &'static str {
        "ollama-vision"
    }

    async fn recognize(&self, image: &[u8], _mime_type: &str) -> Result<PageText> {
        let request = ChatRequest {
            model: &self.model,
            stream: false,
            messages: vec![Message {
                role: "user",
                content: VISION_PROMPT,
                images: vec![base64_encode(image)],
            }],
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| http::request_error("Ollama", e))?;
        let result: ChatResponse = http::ensure_success(response, "Ollama")
            .await?
            .json()
            .await
            .map_err(|e| http::parse_error("Ollama", e))?;

        Ok(PageText {
            text: result.message.content,
            confidence: None,
        })
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

    fn ocr(server: &MockServer) -> Arc<dyn OcrService> {
        let mut settings = PipelineSettings::default();
        settings.ocr.base_url = server.url();
        settings.ocr.model = "minicpm-v".into();
        build(&ProviderContext {
            settings,
            http: reqwest::Client::new(),
        })
        .unwrap()
    }

    #[tokio::test]
    async fn sends_the_image_to_the_configured_model() {
        let server =
            MockServer::start(vec![(200, r#"{"message":{"content":"text"}}"#.into())]).await;

        assert_eq!(
            ocr(&server)
                .extract_text(b"png", "image/png")
                .await
                .unwrap(),
            "text"
        );
        let request = &server.requests()[0];
        assert_eq!(request.path, "/api/chat");
        assert!(request.body.contains("\"model\":\"minicpm-v\""));
        assert!(request.body.contains("\"images\":[\"cG5n\"]"));
    }

    #[tokio::test]
    async fn health_reports_a_missing_model() {
        let server = MockServer::start(vec![(
            200,
            r#"{"models":[{"name":"llama3:latest"}]}"#.into(),
        )])
        .await;
        let err = ocr(&server).health().await.unwrap_err().to_string();
        assert!(err.contains("ollama pull minicpm-v"), "{err}");
    }
}
