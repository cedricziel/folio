//! OCR through an OpenAI-compatible chat completions endpoint with image input

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use folio_core::error::{Error, Result};
use folio_core::ports::OcrService;

use super::{PageEngine, PageOcr, PageText, VISION_PROMPT};
use crate::ai::http::{self, base64_encode};
use crate::ai::registry::ProviderContext;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    let ocr = &ctx.settings.ocr;
    let custom = !ocr.base_url.is_empty();
    Ok(Arc::new(PageOcr(OpenAiVision {
        client: ctx.http.clone(),
        base_url: http::base_url(&ocr.base_url, DEFAULT_BASE_URL),
        api_key: http::api_key(&ctx.settings.ai.openai_api_key, custom, "OpenAI")?,
        model: if ocr.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            ocr.model.clone()
        },
    })))
}

struct OpenAiVision {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message>,
    max_tokens: u32,
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
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
}

#[async_trait]
impl PageEngine for OpenAiVision {
    fn engine_id(&self) -> &'static str {
        "openai-vision"
    }

    async fn recognize(&self, image: &[u8], mime_type: &str) -> Result<PageText> {
        let request = ChatRequest {
            model: &self.model,
            messages: vec![Message {
                role: "user",
                content: vec![
                    Content::Text {
                        text: VISION_PROMPT,
                    },
                    Content::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:{mime_type};base64,{}", base64_encode(image)),
                        },
                    },
                ],
            }],
            max_tokens: 4096,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| http::request_error("OpenAI", e))?;
        let result: ChatResponse = http::ensure_success(response, "OpenAI")
            .await?
            .json()
            .await
            .map_err(|e| http::parse_error("OpenAI", e))?;

        let text = result
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::AiService("No response from OpenAI".to_string()))?
            .message
            .content
            .unwrap_or_default();
        Ok(PageText {
            text,
            confidence: None,
        })
    }

    async fn health(&self) -> Result<()> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| http::request_error("OpenAI", e))?;
        http::ensure_success(response, "OpenAI").await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::test_support::MockServer;
    use folio_core::models::PipelineSettings;

    fn ctx(base_url: &str, model: &str) -> ProviderContext {
        let mut settings = PipelineSettings::default();
        settings.ocr.provider = "openai-vision".into();
        settings.ocr.base_url = base_url.into();
        settings.ocr.model = model.into();
        ProviderContext {
            settings,
            http: reqwest::Client::new(),
        }
    }

    #[tokio::test]
    async fn uses_the_configured_model() {
        let server = MockServer::start(vec![(
            200,
            r#"{"choices":[{"message":{"content":"hello"}}]}"#.into(),
        )])
        .await;
        let ocr = build(&ctx(&server.url(), "my-vision-model")).unwrap();

        let text = ocr.extract_text(b"png", "image/png").await.unwrap();

        assert_eq!(text, "hello");
        let requests = server.requests();
        assert_eq!(requests[0].path, "/chat/completions");
        assert!(requests[0].body.contains("\"model\":\"my-vision-model\""));
    }

    #[test]
    fn hosted_endpoint_requires_a_key() {
        assert!(build(&ctx("", "")).is_err());
    }
}
