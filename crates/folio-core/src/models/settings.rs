//! Settings model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single setting key-value pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub description: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// AI configuration settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSettings {
    /// AI provider: "openai", "anthropic", or "local"
    pub provider: String,
    /// OpenAI API key
    pub openai_api_key: String,
    /// Anthropic API key
    pub anthropic_api_key: String,
    /// OCR model to use
    pub ocr_model: String,
    /// Embedding model to use
    pub embedding_model: String,
}

impl AiSettings {
    pub fn from_settings(settings: &[Setting]) -> Self {
        let mut ai = AiSettings::default();

        for setting in settings {
            match setting.key.as_str() {
                "ai.provider" => ai.provider = setting.value.clone(),
                "ai.openai_api_key" => ai.openai_api_key = setting.value.clone(),
                "ai.anthropic_api_key" => ai.anthropic_api_key = setting.value.clone(),
                "ai.ocr_model" => ai.ocr_model = setting.value.clone(),
                "ai.embedding_model" => ai.embedding_model = setting.value.clone(),
                _ => {}
            }
        }

        ai
    }

    pub fn to_settings(&self) -> Vec<(String, String)> {
        vec![
            ("ai.provider".to_string(), self.provider.clone()),
            ("ai.openai_api_key".to_string(), self.openai_api_key.clone()),
            (
                "ai.anthropic_api_key".to_string(),
                self.anthropic_api_key.clone(),
            ),
            ("ai.ocr_model".to_string(), self.ocr_model.clone()),
            (
                "ai.embedding_model".to_string(),
                self.embedding_model.clone(),
            ),
        ]
    }
}

/// Processing configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingSettings {
    /// Automatically run OCR on uploaded documents
    pub auto_ocr: bool,
    /// Automatically generate thumbnails
    pub auto_thumbnail: bool,
    /// Automatically generate embeddings for search
    pub auto_embedding: bool,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            auto_ocr: true,
            auto_thumbnail: true,
            auto_embedding: true,
        }
    }
}

impl ProcessingSettings {
    pub fn from_settings(settings: &[Setting]) -> Self {
        let mut proc = ProcessingSettings::default();

        for setting in settings {
            match setting.key.as_str() {
                "processing.auto_ocr" => proc.auto_ocr = setting.value == "true",
                "processing.auto_thumbnail" => proc.auto_thumbnail = setting.value == "true",
                "processing.auto_embedding" => proc.auto_embedding = setting.value == "true",
                _ => {}
            }
        }

        proc
    }
}
