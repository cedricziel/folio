use rmcp::model::{CacheScope, CustomRequest, CustomResult, ReadResourceResult, ResourceContents};
use rmcp::ErrorData;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const EXTENSION: &str = "io.modelcontextprotocol/skills";
const SKILL_URI: &str = "skill://folio-archive/SKILL.md";
const TTL_MS: u64 = 300_000;
const FILES: &[(&str, &str)] = &[
    (SKILL_URI, include_str!("../skills/folio-archive/SKILL.md")),
    (
        "skill://folio-archive/references/workflows.md",
        include_str!("../skills/folio-archive/references/workflows.md"),
    ),
];

#[derive(Deserialize)]
struct ListParams {
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct GetParams {
    uri: String,
}

pub(crate) fn request(request: CustomRequest) -> Result<CustomResult, ErrorData> {
    let params = request.params.unwrap_or_else(|| json!({}));
    let mut result = json!({
        "resultType": "complete", "ttlMs": TTL_MS, "cacheScope": "public"
    });
    match request.method.as_str() {
        "skills/list" => {
            let params: ListParams = serde_json::from_value(params)
                .map_err(|_| ErrorData::invalid_params("expected skills/list parameters", None))?;
            if params.cursor.is_some() {
                return Err(ErrorData::invalid_params("unknown skills cursor", None));
            }
            result["skills"] = json!([entry()?]);
        }
        "skills/get" => {
            let params: GetParams = serde_json::from_value(params)
                .map_err(|_| ErrorData::invalid_params("expected a skill uri", None))?;
            if params.uri != SKILL_URI {
                return Err(ErrorData::invalid_params("unknown skill uri", None));
            }
            result["skill"] = entry()?;
        }
        _ => {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                request.method,
                None,
            ))
        }
    }
    Ok(CustomResult::new(result))
}

fn entry() -> Result<Value, ErrorData> {
    let frontmatter = FILES[0]
        .1
        .strip_prefix("---\n")
        .and_then(|text| text.split_once("\n---\n"))
        .map(|(frontmatter, _)| frontmatter)
        .ok_or_else(|| ErrorData::internal_error("invalid bundled skill frontmatter", None))?;
    // JSON flow mappings are valid YAML and preserve every frontmatter field.
    let frontmatter: Value = serde_json::from_str(frontmatter)
        .map_err(|_| ErrorData::internal_error("invalid bundled skill frontmatter", None))?;
    let resources: Vec<_> = FILES
        .iter()
        .map(|(uri, text)| {
            json!({
                "uri": uri,
                "digest": format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes()))),
                "size": text.len(),
            })
        })
        .collect();
    Ok(json!({"uri": SKILL_URI, "frontmatter": frontmatter, "resources": resources}))
}

pub(crate) fn read(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let (_, text) = FILES
        .iter()
        .find(|(candidate, _)| *candidate == uri)
        .ok_or_else(|| ErrorData::invalid_params("unknown skill resource uri", None))?;
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(*text, uri).with_mime_type("text/markdown")
    ])
    .with_ttl_ms(TTL_MS)
    .with_cache_scope(CacheScope::Public))
}
