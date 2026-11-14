use rmcp::model::{
    CacheScope, GetSkillResult, ListSkillsResult, ReadResourceResult, ResourceContents, Skill,
};
use rmcp::ErrorData;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SKILL_URI: &str = "skill://folio-archive/SKILL.md";
const TTL_MS: u64 = 300_000;
const FILES: &[(&str, &str)] = &[
    (SKILL_URI, include_str!("../skills/folio-archive/SKILL.md")),
    (
        "skill://folio-archive/references/workflows.md",
        include_str!("../skills/folio-archive/references/workflows.md"),
    ),
];

pub(crate) fn list(cursor: Option<&str>) -> Result<ListSkillsResult, ErrorData> {
    if cursor.is_some() {
        return Err(ErrorData::invalid_params("unknown skills cursor", None));
    }
    Ok(ListSkillsResult::with_all_items(vec![entry()?])
        .with_ttl_ms(TTL_MS)
        .with_cache_scope(CacheScope::Public))
}

pub(crate) fn get(uri: &str) -> Result<GetSkillResult, ErrorData> {
    if uri != SKILL_URI {
        return Err(ErrorData::invalid_params("unknown skill uri", None));
    }
    Ok(GetSkillResult::new(entry()?)
        .with_ttl_ms(TTL_MS)
        .with_cache_scope(CacheScope::Public))
}

fn entry() -> Result<Skill, ErrorData> {
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
    serde_json::from_value(
        json!({"uri": SKILL_URI, "frontmatter": frontmatter, "resources": resources}),
    )
    .map_err(|_| ErrorData::internal_error("invalid bundled skill manifest", None))
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
