//! OCR through the Anthropic messages API with image input

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use folio_core::error::{Error, Result};
use folio_core::ports::OcrService;

use super::{PageEngine, PageOcr, PageText, VISION_PROMPT};
use crate::ai::http::{self, base64_encode};
use crate::ai::registry::ProviderContext;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    let ocr = &ctx.settings.ocr;
    let custom = !ocr.base_url.is_empty();
    Ok(Arc::new(PageOcr(AnthropicVision {
        client: ctx.http.clone(),
        base_url: http::base_url(&ocr.base_url, DEFAULT_BASE_URL),
        api_key: http::api_key(&ctx.settings.ai.anthropic_api_key, custom, "Anthropic")?,
        model: if ocr.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            ocr.model.clone()
        },
    })))
}

struct AnthropicVision {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: Vec<Content>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: &'static str },
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: &'static str,
    media_type: String,
    data: String,
}

#[derive(Deserialize)]
struct Response {
    content: Vec<ResponseContent>,
}

#[derive(Deserialize)]
struct ResponseContent {
    text: Option<String>,
}

impl AnthropicVision {
    fn authed(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
    }
}

#[async_trait]
impl PageEngine for AnthropicVision {
    fn engine_id(&self) -> &'static str {
        "anthropic-vision"
    }

    async fn recognize(&self, image: &[u8], mime_type: &str) -> Result<PageText> {
        let request = Request {
            model: &self.model,
            max_tokens: 4096,
            messages: vec![Message {
                role: "user",
                content: vec![
                    Content::Image {
                        source: ImageSource {
                            source_type: "base64",
                            media_type: mime_type.to_string(),
                            data: base64_encode(image),
                        },
                    },
                    Content::Text {
                        text: VISION_PROMPT,
                    },
                ],
            }],
        };

        let response = self
            .authed(self.client.post(format!("{}/messages", self.base_url)))
            .json(&request)
            .send()
            .await
            .map_err(|e| http::request_error("Anthropic", e))?;
        let result: Response = http::ensure_success(response, "Anthropic")
            .await?
            .json()
            .await
            .map_err(|e| http::parse_error("Anthropic", e))?;

        let text = result
            .content
            .into_iter()
            .next()
            .ok_or_else(|| Error::AiService("No response from Anthropic".to_string()))?
            .text
            .unwrap_or_default();
        Ok(PageText {
            text,
            confidence: None,
        })
    }

    async fn health(&self) -> Result<()> {
        let response = self
            .authed(self.client.get(format!("{}/models", self.base_url)))
            .send()
            .await
            .map_err(|e| http::request_error("Anthropic", e))?;
        http::ensure_success(response, "Anthropic")
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::test_support::MockServer;
    use folio_core::models::PipelineSettings;

    #[tokio::test]
    async fn uses_the_configured_model() {
        let server = MockServer::start(vec![(
            200,
            r#"{"content":[{"type":"text","text":"hallo"}]}"#.into(),
        )])
        .await;
        let mut settings = PipelineSettings::default();
        settings.ocr.base_url = server.url();
        settings.ocr.model = "claude-custom".into();
        let ocr = build(&ProviderContext {
            settings,
            http: reqwest::Client::new(),
        })
        .unwrap();

        assert_eq!(
            ocr.extract_text(b"png", "image/png").await.unwrap(),
            "hallo"
        );
        let requests = server.requests();
        assert_eq!(requests[0].path, "/messages");
        assert!(requests[0].body.contains("\"model\":\"claude-custom\""));
    }
}
