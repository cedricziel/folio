//! AI service implementations

use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::ports::{EmbeddingService, OcrService, SettingsRepository};

use crate::config::AiConfig;

/// Factory for creating AI services based on configuration
pub struct AiServiceFactory;

impl AiServiceFactory {
    /// Create OCR service based on provider configuration
    pub fn create_ocr_service(config: &AiConfig) -> Arc<dyn OcrService> {
        match config.provider.as_str() {
            "openai" => Arc::new(OpenAiOcrService::new(
                config.openai_api_key.clone().unwrap_or_default(),
            )),
            "anthropic" => Arc::new(AnthropicOcrService::new(
                config.anthropic_api_key.clone().unwrap_or_default(),
            )),
            _ => Arc::new(NoOpOcrService),
        }
    }

    /// Create embedding service based on provider configuration
    pub fn create_embedding_service(config: &AiConfig) -> Arc<dyn EmbeddingService> {
        match config.provider.as_str() {
            "openai" => Arc::new(OpenAiEmbeddingService::new(
                config.openai_api_key.clone().unwrap_or_default(),
            )),
            _ => Arc::new(NoOpEmbeddingService),
        }
    }
}

// ============================================================================
// OpenAI implementations
// ============================================================================

/// OpenAI OCR service using GPT-4 Vision
pub struct OpenAiOcrService {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiOcrService {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    max_tokens: u32,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: Vec<OpenAiContent>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum OpenAiContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

#[async_trait]
impl OcrService for OpenAiOcrService {
    #[instrument(skip(self, data))]
    async fn extract_text(&self, data: &[u8], mime_type: &str) -> Result<String> {
        if self.api_key.is_empty() {
            return Err(Error::AiService(
                "OpenAI API key not configured".to_string(),
            ));
        }

        // For now, we only support images via base64 encoding
        let base64_data = base64_encode(data);
        let data_url = format!("data:{};base64,{}", mime_type, base64_data);

        let request = OpenAiChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: vec![
                    OpenAiContent::Text {
                        text: "Extract all text from this document image. Return only the extracted text, nothing else.".to_string(),
                    },
                    OpenAiContent::ImageUrl {
                        image_url: ImageUrl { url: data_url },
                    },
                ],
            }],
            max_tokens: 4096,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "OpenAI API error ({}): {}",
                status, body
            )));
        }

        let result: OpenAiChatResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse OpenAI response: {}", e)))?;

        result
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| Error::AiService("No response from OpenAI".to_string()))
    }
}

/// OpenAI embedding service
pub struct OpenAiEmbeddingService {
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiEmbeddingService {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OpenAiEmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingService for OpenAiEmbeddingService {
    #[instrument(skip(self, text))]
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        if self.api_key.is_empty() {
            return Err(Error::AiService(
                "OpenAI API key not configured".to_string(),
            ));
        }

        let embeddings = self.generate_embeddings(&[text]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Error::AiService("No embedding returned".to_string()))
    }

    #[instrument(skip(self, texts))]
    async fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if self.api_key.is_empty() {
            return Err(Error::AiService(
                "OpenAI API key not configured".to_string(),
            ));
        }

        let request = OpenAiEmbeddingRequest {
            model: "text-embedding-3-small".to_string(),
            input: texts.iter().map(|s| s.to_string()).collect(),
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "OpenAI API error ({}): {}",
                status, body
            )));
        }

        let result: OpenAiEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse OpenAI response: {}", e)))?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn embedding_dimension(&self) -> usize {
        1536 // text-embedding-3-small dimension
    }
}

// ============================================================================
// Anthropic implementations
// ============================================================================

/// Anthropic OCR service using Claude Vision
pub struct AnthropicOcrService {
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicOcrService {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum AnthropicContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseContent>,
}

#[derive(Deserialize)]
struct AnthropicResponseContent {
    text: String,
}

#[async_trait]
impl OcrService for AnthropicOcrService {
    #[instrument(skip(self, data))]
    async fn extract_text(&self, data: &[u8], mime_type: &str) -> Result<String> {
        if self.api_key.is_empty() {
            return Err(Error::AiService(
                "Anthropic API key not configured".to_string(),
            ));
        }

        let base64_data = base64_encode(data);

        let request = AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: vec![
                    AnthropicContent::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: mime_type.to_string(),
                            data: base64_data,
                        },
                    },
                    AnthropicContent::Text {
                        text: "Extract all text from this document image. Return only the extracted text, nothing else.".to_string(),
                    },
                ],
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("Anthropic request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "Anthropic API error ({}): {}",
                status, body
            )));
        }

        let result: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse Anthropic response: {}", e)))?;

        result
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| Error::AiService("No response from Anthropic".to_string()))
    }
}

// ============================================================================
// No-op implementations (for local/testing)
// ============================================================================

/// No-op OCR service that returns empty text
pub struct NoOpOcrService;

#[async_trait]
impl OcrService for NoOpOcrService {
    async fn extract_text(&self, _data: &[u8], _mime_type: &str) -> Result<String> {
        debug!("NoOpOcrService: returning empty text");
        Ok(String::new())
    }
}

/// No-op embedding service that returns zero vectors
pub struct NoOpEmbeddingService;

#[async_trait]
impl EmbeddingService for NoOpEmbeddingService {
    async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>> {
        debug!("NoOpEmbeddingService: returning zero vector");
        Ok(vec![0.0; 384])
    }

    async fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
    }

    fn embedding_dimension(&self) -> usize {
        384
    }
}

// ============================================================================
// Dynamic AI services (read settings from database)
// ============================================================================

/// Dynamic OCR service that reads settings from the database on each request
pub struct DynamicOcrService {
    settings_repo: Arc<dyn SettingsRepository>,
    fallback_config: AiConfig,
    client: reqwest::Client,
    /// Cache for settings to avoid reading DB on every request
    cached_settings: RwLock<Option<CachedAiSettings>>,
    /// Storage path for temp files during PDF conversion
    storage_path: String,
}

/// Dynamic embedding service that reads settings from the database on each request
pub struct DynamicEmbeddingService {
    settings_repo: Arc<dyn SettingsRepository>,
    fallback_config: AiConfig,
    client: reqwest::Client,
    /// Cache for settings to avoid reading DB on every request
    cached_settings: RwLock<Option<CachedAiSettings>>,
}

#[derive(Clone)]
struct CachedAiSettings {
    provider: String,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    cached_at: std::time::Instant,
}

impl CachedAiSettings {
    fn is_expired(&self) -> bool {
        // Cache for 5 seconds to balance freshness vs DB load
        self.cached_at.elapsed() > std::time::Duration::from_secs(5)
    }
}

impl DynamicOcrService {
    pub fn new(
        settings_repo: Arc<dyn SettingsRepository>,
        fallback_config: AiConfig,
        storage_path: impl Into<String>,
    ) -> Self {
        Self {
            settings_repo,
            fallback_config,
            client: reqwest::Client::new(),
            cached_settings: RwLock::new(None),
            storage_path: storage_path.into(),
        }
    }

    async fn get_settings(&self) -> CachedAiSettings {
        // Check cache first
        {
            let cache = self.cached_settings.read().await;
            if let Some(ref cached) = *cache {
                if !cached.is_expired() {
                    return cached.clone();
                }
            }
        }

        // Fetch from database
        let settings = self.fetch_settings_from_db().await;

        // Update cache
        {
            let mut cache = self.cached_settings.write().await;
            *cache = Some(settings.clone());
        }

        settings
    }

    async fn fetch_settings_from_db(&self) -> CachedAiSettings {
        let db_settings = self
            .settings_repo
            .list_by_prefix("ai.")
            .await
            .unwrap_or_default();

        let provider = db_settings
            .iter()
            .find(|s| s.key == "ai.provider")
            .map(|s| s.value.clone())
            .unwrap_or_else(|| self.fallback_config.provider.clone());

        let openai_api_key = db_settings
            .iter()
            .find(|s| s.key == "ai.openai_api_key")
            .map(|s| s.value.clone())
            .filter(|k| !k.is_empty())
            .or_else(|| self.fallback_config.openai_api_key.clone());

        let anthropic_api_key = db_settings
            .iter()
            .find(|s| s.key == "ai.anthropic_api_key")
            .map(|s| s.value.clone())
            .filter(|k| !k.is_empty())
            .or_else(|| self.fallback_config.anthropic_api_key.clone());

        info!(
            "Loaded AI settings: provider={}, openai_key={}, anthropic_key={}",
            provider,
            if openai_api_key
                .as_ref()
                .map(|k| !k.is_empty())
                .unwrap_or(false)
            {
                "configured"
            } else {
                "not set"
            },
            if anthropic_api_key
                .as_ref()
                .map(|k| !k.is_empty())
                .unwrap_or(false)
            {
                "configured"
            } else {
                "not set"
            }
        );

        CachedAiSettings {
            provider,
            openai_api_key,
            anthropic_api_key,
            cached_at: std::time::Instant::now(),
        }
    }
}

#[async_trait]
impl OcrService for DynamicOcrService {
    #[instrument(skip(self, data))]
    async fn extract_text(&self, data: &[u8], mime_type: &str) -> Result<String> {
        let settings = self.get_settings().await;

        match settings.provider.as_str() {
            "openai" => {
                let api_key = settings.openai_api_key.unwrap_or_default();
                if api_key.is_empty() {
                    return Err(Error::AiService(
                        "OpenAI API key not configured".to_string(),
                    ));
                }
                self.openai_ocr(data, mime_type, &api_key).await
            }
            "anthropic" => {
                let api_key = settings.anthropic_api_key.unwrap_or_default();
                if api_key.is_empty() {
                    return Err(Error::AiService(
                        "Anthropic API key not configured".to_string(),
                    ));
                }
                self.anthropic_ocr(data, mime_type, &api_key).await
            }
            _ => {
                debug!("No AI provider configured, returning empty text");
                Ok(String::new())
            }
        }
    }
}

impl DynamicOcrService {
    async fn openai_ocr(&self, data: &[u8], mime_type: &str, api_key: &str) -> Result<String> {
        // Handle PDFs by converting to images first
        if mime_type == "application/pdf" {
            return self.openai_ocr_pdf(data, api_key).await;
        }

        self.openai_ocr_image(data, mime_type, api_key).await
    }

    /// OCR a PDF by converting pages to images and processing each
    async fn openai_ocr_pdf(&self, data: &[u8], api_key: &str) -> Result<String> {
        let page_images = self.convert_pdf_to_images(data).await?;

        if page_images.is_empty() {
            return Err(Error::AiService(
                "Failed to convert PDF to images".to_string(),
            ));
        }

        info!("Processing {} PDF pages for OCR", page_images.len());

        let mut all_text = Vec::new();
        for (i, image_data) in page_images.iter().enumerate() {
            debug!("Processing page {} of {}", i + 1, page_images.len());
            match self
                .openai_ocr_image(image_data, "image/jpeg", api_key)
                .await
            {
                Ok(text) => {
                    if !text.is_empty() {
                        all_text.push(text);
                    }
                }
                Err(e) => {
                    warn!("Failed to OCR page {}: {}", i + 1, e);
                    // Continue with other pages
                }
            }
        }

        Ok(all_text.join("\n\n--- Page Break ---\n\n"))
    }

    /// Convert PDF to a vector of JPEG images (one per page)
    async fn convert_pdf_to_images(&self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        let temp_id = Uuid::new_v4();
        let temp_pdf = format!("{}/temp_ocr_{}.pdf", self.storage_path, temp_id);
        let temp_output = format!("{}/temp_ocr_{}", self.storage_path, temp_id);

        // Write PDF to temp file
        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        // Use pdftoppm to convert all pages to JPEG images
        let result = Command::new("pdftoppm")
            .args(["-jpeg", "-r", "300", &temp_pdf, &temp_output])
            .output();

        // Clean up temp PDF
        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                // Read all generated page images
                // pdftoppm creates files with zero-padded numbers: {output}-01.jpg, {output}-02.jpg, etc.
                let mut page_files: Vec<String> = Vec::new();

                // Get just the filename part for matching
                let output_basename = temp_output.rsplit('/').next().unwrap_or(&temp_output);

                if let Ok(entries) = std::fs::read_dir(&self.storage_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            // Match files like: temp_ocr_UUID-01.jpg, temp_ocr_UUID-02.jpg
                            if name.starts_with(output_basename)
                                && name.ends_with(".jpg")
                                && name.len() > output_basename.len()
                            {
                                page_files.push(path.to_string_lossy().to_string());
                            }
                        }
                    }
                }

                // Sort files to ensure correct page order
                page_files.sort();

                debug!(
                    "Found {} page files for prefix {}",
                    page_files.len(),
                    output_basename
                );

                let mut images = Vec::new();
                for page_file in &page_files {
                    match fs::read(page_file).await {
                        Ok(image_data) => {
                            images.push(image_data);
                            let _ = fs::remove_file(page_file).await;
                        }
                        Err(e) => {
                            warn!("Failed to read page image {}: {}", page_file, e);
                        }
                    }
                }

                if images.is_empty() {
                    warn!(
                        "pdftoppm produced no output files (looked for prefix: {} in {})",
                        output_basename, self.storage_path
                    );
                    return Err(Error::AiService(
                        "PDF conversion produced no images".to_string(),
                    ));
                }

                debug!("Converted PDF to {} page images", images.len());
                Ok(images)
            }
            Ok(output) => {
                warn!(
                    "pdftoppm failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                Err(Error::AiService(format!(
                    "PDF conversion failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
            Err(e) => {
                warn!("pdftoppm not available: {}", e);
                Err(Error::AiService(format!(
                    "PDF conversion tool (pdftoppm) not available: {}. Install poppler-utils.",
                    e
                )))
            }
        }
    }

    /// OCR a single image
    async fn openai_ocr_image(
        &self,
        data: &[u8],
        mime_type: &str,
        api_key: &str,
    ) -> Result<String> {
        let base64_data = base64_encode(data);
        let data_url = format!("data:{};base64,{}", mime_type, base64_data);

        let request = OpenAiChatRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: vec![
                    OpenAiContent::Text {
                        text: "Extract all text from this document image. Return only the extracted text, nothing else.".to_string(),
                    },
                    OpenAiContent::ImageUrl {
                        image_url: ImageUrl { url: data_url },
                    },
                ],
            }],
            max_tokens: 4096,
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "OpenAI API error ({}): {}",
                status, body
            )));
        }

        let result: OpenAiChatResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse OpenAI response: {}", e)))?;

        result
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| Error::AiService("No response from OpenAI".to_string()))
    }

    async fn anthropic_ocr(&self, data: &[u8], mime_type: &str, api_key: &str) -> Result<String> {
        // Handle PDFs by converting to images first
        if mime_type == "application/pdf" {
            return self.anthropic_ocr_pdf(data, api_key).await;
        }

        self.anthropic_ocr_image(data, mime_type, api_key).await
    }

    /// OCR a PDF using Anthropic by converting pages to images and processing each
    async fn anthropic_ocr_pdf(&self, data: &[u8], api_key: &str) -> Result<String> {
        let page_images = self.convert_pdf_to_images(data).await?;

        if page_images.is_empty() {
            return Err(Error::AiService(
                "Failed to convert PDF to images".to_string(),
            ));
        }

        info!(
            "Processing {} PDF pages for OCR with Anthropic",
            page_images.len()
        );

        let mut all_text = Vec::new();
        for (i, image_data) in page_images.iter().enumerate() {
            debug!("Processing page {} of {}", i + 1, page_images.len());
            match self
                .anthropic_ocr_image(image_data, "image/jpeg", api_key)
                .await
            {
                Ok(text) => {
                    if !text.is_empty() {
                        all_text.push(text);
                    }
                }
                Err(e) => {
                    warn!("Failed to OCR page {}: {}", i + 1, e);
                    // Continue with other pages
                }
            }
        }

        Ok(all_text.join("\n\n--- Page Break ---\n\n"))
    }

    /// OCR a single image using Anthropic
    async fn anthropic_ocr_image(
        &self,
        data: &[u8],
        mime_type: &str,
        api_key: &str,
    ) -> Result<String> {
        let base64_data = base64_encode(data);

        let request = AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: vec![
                    AnthropicContent::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: mime_type.to_string(),
                            data: base64_data,
                        },
                    },
                    AnthropicContent::Text {
                        text: "Extract all text from this document image. Return only the extracted text, nothing else.".to_string(),
                    },
                ],
            }],
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("Anthropic request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "Anthropic API error ({}): {}",
                status, body
            )));
        }

        let result: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse Anthropic response: {}", e)))?;

        result
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| Error::AiService("No response from Anthropic".to_string()))
    }
}

impl DynamicEmbeddingService {
    pub fn new(settings_repo: Arc<dyn SettingsRepository>, fallback_config: AiConfig) -> Self {
        Self {
            settings_repo,
            fallback_config,
            client: reqwest::Client::new(),
            cached_settings: RwLock::new(None),
        }
    }

    async fn get_settings(&self) -> CachedAiSettings {
        // Check cache first
        {
            let cache = self.cached_settings.read().await;
            if let Some(ref cached) = *cache {
                if !cached.is_expired() {
                    return cached.clone();
                }
            }
        }

        // Fetch from database
        let settings = self.fetch_settings_from_db().await;

        // Update cache
        {
            let mut cache = self.cached_settings.write().await;
            *cache = Some(settings.clone());
        }

        settings
    }

    async fn fetch_settings_from_db(&self) -> CachedAiSettings {
        let db_settings = self
            .settings_repo
            .list_by_prefix("ai.")
            .await
            .unwrap_or_default();

        let provider = db_settings
            .iter()
            .find(|s| s.key == "ai.provider")
            .map(|s| s.value.clone())
            .unwrap_or_else(|| self.fallback_config.provider.clone());

        let openai_api_key = db_settings
            .iter()
            .find(|s| s.key == "ai.openai_api_key")
            .map(|s| s.value.clone())
            .filter(|k| !k.is_empty())
            .or_else(|| self.fallback_config.openai_api_key.clone());

        let anthropic_api_key = db_settings
            .iter()
            .find(|s| s.key == "ai.anthropic_api_key")
            .map(|s| s.value.clone())
            .filter(|k| !k.is_empty())
            .or_else(|| self.fallback_config.anthropic_api_key.clone());

        CachedAiSettings {
            provider,
            openai_api_key,
            anthropic_api_key,
            cached_at: std::time::Instant::now(),
        }
    }
}

#[async_trait]
impl EmbeddingService for DynamicEmbeddingService {
    #[instrument(skip(self, text))]
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.generate_embeddings(&[text]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| Error::AiService("No embedding returned".to_string()))
    }

    #[instrument(skip(self, texts))]
    async fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let settings = self.get_settings().await;

        match settings.provider.as_str() {
            "openai" => {
                let api_key = settings.openai_api_key.unwrap_or_default();
                if api_key.is_empty() {
                    return Err(Error::AiService(
                        "OpenAI API key not configured".to_string(),
                    ));
                }
                self.openai_embeddings(texts, &api_key).await
            }
            _ => {
                debug!("No embedding provider configured, returning zero vectors");
                Ok(texts.iter().map(|_| vec![0.0; 1536]).collect())
            }
        }
    }

    fn embedding_dimension(&self) -> usize {
        1536 // text-embedding-3-small dimension
    }
}

impl DynamicEmbeddingService {
    async fn openai_embeddings(&self, texts: &[&str], api_key: &str) -> Result<Vec<Vec<f32>>> {
        let request = OpenAiEmbeddingRequest {
            model: "text-embedding-3-small".to_string(),
            input: texts.iter().map(|s| s.to_string()).collect(),
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::AiService(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::AiService(format!(
                "OpenAI API error ({}): {}",
                status, body
            )));
        }

        let result: OpenAiEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::AiService(format!("Failed to parse OpenAI response: {}", e)))?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }
}

// ============================================================================
// Utilities
// ============================================================================

/// Simple base64 encoding (no extra dependencies)
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = if i + 1 < data.len() {
            data[i + 1] as usize
        } else {
            0
        };
        let b2 = if i + 2 < data.len() {
            data[i + 2] as usize
        } else {
            0
        };

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if i + 1 < data.len() {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }

        if i + 2 < data.len() {
            result.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}

/// Compute cosine similarity between two vectors
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}
