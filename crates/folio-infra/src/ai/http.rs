//! Helpers shared by HTTP-based providers

use folio_core::error::{Error, Result};

/// Use the configured endpoint if set, otherwise the default, without a trailing slash
pub fn base_url(configured: &str, default: &str) -> String {
    let url = if configured.trim().is_empty() {
        default
    } else {
        configured.trim()
    };
    url.trim_end_matches('/').to_string()
}

/// Return the API key, requiring one only when talking to the hosted default endpoint
///
/// Self-hosted OpenAI-compatible servers (LocalAI, vLLM, LM Studio) usually
/// run without authentication.
pub fn api_key(key: &str, custom_endpoint: bool, provider: &str) -> Result<String> {
    if key.is_empty() && !custom_endpoint {
        return Err(Error::AiService(format!(
            "{provider} API key not configured"
        )));
    }
    Ok(key.to_string())
}

/// Turn a non-success response into an error carrying the body
pub async fn ensure_success(
    response: reqwest::Response,
    provider: &str,
) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(Error::AiService(format!(
        "{provider} API error ({status}): {body}"
    )))
}

pub fn request_error(provider: &str, e: reqwest::Error) -> Error {
    Error::AiService(format!("{provider} request failed: {e}"))
}

pub fn parse_error(provider: &str, e: reqwest::Error) -> Error {
    Error::AiService(format!("Failed to parse {provider} response: {e}"))
}

/// Check that an Ollama server answers and has pulled `model`
pub async fn check_ollama(client: &reqwest::Client, base_url: &str, model: &str) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(serde::Deserialize)]
    struct Tag {
        name: String,
    }

    let response = client
        .get(format!("{base_url}/api/tags"))
        .send()
        .await
        .map_err(|e| request_error("Ollama", e))?;
    let tags: Tags = ensure_success(response, "Ollama")
        .await?
        .json()
        .await
        .map_err(|e| parse_error("Ollama", e))?;

    let wanted = if model.contains(':') {
        model.to_string()
    } else {
        format!("{model}:latest")
    };
    if tags
        .models
        .iter()
        .any(|t| t.name == wanted || t.name == model)
    {
        Ok(())
    } else {
        Err(Error::AiService(format!(
            "Model {model} is not available on {base_url}; run `ollama pull {model}`"
        )))
    }
}

/// Standard base64 encoding
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[b2 & 0x3f] as char
        } else {
            '='
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base_url_prefers_configured_and_strips_slash() {
        assert_eq!(base_url("", "http://a/v1"), "http://a/v1");
        assert_eq!(
            base_url(" http://b:1234/v1/ ", "http://a/v1"),
            "http://b:1234/v1"
        );
    }

    #[test]
    fn api_key_is_optional_for_custom_endpoints() {
        assert!(api_key("", false, "OpenAI").is_err());
        assert!(api_key("", true, "OpenAI").is_ok());
    }
}
