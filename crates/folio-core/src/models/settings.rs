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

/// Credentials for hosted AI providers
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSettings {
    /// OpenAI API key
    pub openai_api_key: String,
    /// Anthropic API key
    pub anthropic_api_key: String,
}

impl AiSettings {
    pub fn from_settings(settings: &[Setting]) -> Self {
        let mut ai = AiSettings::default();

        for setting in settings {
            match setting.key.as_str() {
                "ai.openai_api_key" => ai.openai_api_key = setting.value.clone(),
                "ai.anthropic_api_key" => ai.anthropic_api_key = setting.value.clone(),
                _ => {}
            }
        }

        ai
    }

    pub fn to_settings(&self) -> Vec<(String, String)> {
        vec![
            ("ai.openai_api_key".to_string(), self.openai_api_key.clone()),
            (
                "ai.anthropic_api_key".to_string(),
                self.anthropic_api_key.clone(),
            ),
        ]
    }
}

/// Which OCR provider runs and how
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrSettings {
    /// Registry id of the provider (`tesseract`, `openai-vision`, `none`, ...)
    pub provider: String,
    /// Model name for vision providers; empty selects the provider default
    pub model: String,
    /// Tesseract-style language list, e.g. `deu+eng`
    pub languages: String,
    /// Endpoint override for Ollama or OpenAI-compatible servers
    pub base_url: String,
}

impl Default for OcrSettings {
    fn default() -> Self {
        Self {
            provider: "tesseract".to_string(),
            model: String::new(),
            languages: "eng".to_string(),
            base_url: String::new(),
        }
    }
}

/// Which embedding provider runs and how
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingSettings {
    /// Registry id of the provider (`openai`, `ollama`, `fastembed`, `none`)
    pub provider: String,
    /// Model name; empty selects the provider default
    pub model: String,
    /// Endpoint override for Ollama or OpenAI-compatible servers
    pub base_url: String,
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            provider: "none".to_string(),
            model: String::new(),
            base_url: String::new(),
        }
    }
}

/// Processing configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessingSettings {
    /// Automatically run OCR on uploaded documents
    pub auto_ocr: bool,
    /// Automatically generate thumbnails
    pub auto_thumbnail: bool,
    /// Automatically generate embeddings for search
    pub auto_embed: bool,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            auto_ocr: true,
            auto_thumbnail: true,
            auto_embed: true,
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
                "processing.auto_embed" => proc.auto_embed = setting.value == "true",
                _ => {}
            }
        }

        proc
    }
}

/// Everything the processing pipeline reads from settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineSettings {
    pub ocr: OcrSettings,
    pub embeddings: EmbeddingSettings,
    pub processing: ProcessingSettings,
    pub ai: AiSettings,
}

impl PipelineSettings {
    /// Build from raw settings rows; unknown or missing keys keep their defaults
    pub fn from_settings(settings: &[Setting]) -> Self {
        Self::default().with_settings(settings)
    }

    /// Overlay saved rows, retaining configured values for missing keys.
    pub fn with_settings(mut self, settings: &[Setting]) -> Self {
        for setting in settings {
            let value = setting.value.trim().to_string();
            match setting.key.as_str() {
                "ocr.provider" if !value.is_empty() => self.ocr.provider = value,
                "ocr.model" => self.ocr.model = value,
                "ocr.languages" if !value.is_empty() => self.ocr.languages = value,
                "ocr.base_url" => self.ocr.base_url = value,
                "embeddings.provider" if !value.is_empty() => self.embeddings.provider = value,
                "embeddings.model" => self.embeddings.model = value,
                "embeddings.base_url" => self.embeddings.base_url = value,
                "processing.auto_ocr" => self.processing.auto_ocr = value == "true",
                "processing.auto_thumbnail" => self.processing.auto_thumbnail = value == "true",
                "processing.auto_embed" => self.processing.auto_embed = value == "true",
                "ai.openai_api_key" => self.ai.openai_api_key = setting.value.clone(),
                "ai.anthropic_api_key" => self.ai.anthropic_api_key = setting.value.clone(),
                _ => {}
            }
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(key: &str, value: &str) -> Setting {
        Setting {
            key: key.to_string(),
            value: value.to_string(),
            description: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn defaults_are_local_ocr_and_no_embeddings() {
        let s = PipelineSettings::from_settings(&[]);
        assert_eq!(s.ocr.provider, "tesseract");
        assert_eq!(s.ocr.languages, "eng");
        assert_eq!(s.embeddings.provider, "none");
        assert!(s.processing.auto_ocr && s.processing.auto_embed);
    }

    #[test]
    fn reads_pipeline_keys() {
        let s = PipelineSettings::from_settings(&[
            setting("ocr.provider", "ollama-vision"),
            setting("ocr.model", "llama3.2-vision"),
            setting("ocr.languages", "deu+eng"),
            setting("embeddings.provider", "ollama"),
            setting("embeddings.model", "nomic-embed-text"),
            setting("embeddings.base_url", " http://gpu:11434 "),
            setting("processing.auto_ocr", "false"),
            setting("processing.auto_embed", "false"),
        ]);
        assert_eq!(s.ocr.provider, "ollama-vision");
        assert_eq!(s.ocr.languages, "deu+eng");
        assert_eq!(s.embeddings.model, "nomic-embed-text");
        assert_eq!(s.embeddings.base_url, "http://gpu:11434");
        assert!(!s.processing.auto_ocr);
        assert!(!s.processing.auto_embed);
    }

    #[test]
    fn saved_rows_override_config_without_resetting_missing_keys() {
        let mut configured = PipelineSettings::default();
        configured.ocr.provider = "ollama-vision".into();
        configured.ocr.model = "vision-model".into();
        configured.processing.auto_embed = false;
        let effective = configured.with_settings(&[
            setting("ocr.provider", "none"),
            setting("processing.auto_embed", "true"),
        ]);
        assert_eq!(effective.ocr.provider, "none");
        assert_eq!(effective.ocr.model, "vision-model");
        assert!(effective.processing.auto_embed);
    }

    #[test]
    fn blank_provider_falls_back_to_default() {
        let s = PipelineSettings::from_settings(&[setting("ocr.provider", "  ")]);
        assert_eq!(s.ocr.provider, "tesseract");
    }
}
