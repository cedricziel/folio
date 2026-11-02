//! MCP handler: tools and resources over the archive

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use folio_core::models::{Document, DocumentSource, Id, SourceKind, Tag, UserId, VaultId};
use folio_core::ports::ProcessingStatus;
use folio_core::services::{
    DocumentDetail, DocumentQuery, LibraryService, MetadataUpdate, NewUpload, VaultService,
};
use folio_core::Error;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData, Json, RoleServer, ServerHandler};
use serde::{Deserialize, Serialize};

use crate::auth::{require_scope, SCOPE_READ, SCOPE_WRITE};

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;
const DEFAULT_TEXT_CHARS: usize = 8_000;
const MAX_TEXT_CHARS: usize = 50_000;
const MAX_RESOURCE_TEXT_CHARS: usize = 200_000;
const MAX_BLOB_BYTES: usize = 5 * 1024 * 1024;
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
const RESOURCE_LIST_LIMIT: usize = 50;
const DOCUMENT_URI_PREFIX: &str = "folio://documents/";

const INSTRUCTIONS: &str = "Folio is a personal document archive. Use search_documents to find \
documents, get_document to read one (long text is paginated with a cursor), and the list_* \
tools to see which tags, correspondents and document types exist before filtering or \
updating. Tags, correspondents and document types are referred to by name, case-insensitively.";

type ToolResult<T> = Result<Json<T>, String>;

/// MCP server handler for the Folio archive
#[derive(Clone)]
pub struct FolioMcp {
    library: Arc<LibraryService>,
    vaults: Arc<VaultService>,
    tool_router: ToolRouter<Self>,
}

impl FolioMcp {
    pub fn new(library: Arc<LibraryService>, vaults: Arc<VaultService>) -> Self {
        Self {
            library,
            vaults,
            tool_router: Self::tool_router(),
        }
    }

    /// The caller's default vault; tools get an explicit `vault` argument later
    async fn vault(&self, ctx: &RequestContext<RoleServer>) -> Result<VaultId, String> {
        let user = crate::auth::subject_of(&ctx.extensions).map(UserId::from);
        self.vaults
            .default_vault(user.as_ref().map(|u| (u, false)))
            .await
            .map_err(tool_error)
    }

    async fn names(&self, vault: VaultId) -> Result<Names, String> {
        let correspondents = self
            .library
            .list_correspondents(vault)
            .await
            .map_err(tool_error)?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let document_types = self
            .library
            .list_document_types(vault)
            .await
            .map_err(tool_error)?
            .into_iter()
            .map(|t| (t.id, t.name))
            .collect();
        Ok(Names {
            correspondents,
            document_types,
        })
    }
}

struct Names {
    correspondents: HashMap<Id, String>,
    document_types: HashMap<Id, String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to look for in titles and text. Every word must match. Leave out to filter by metadata only.
    query: Option<String>,
    /// Only documents carrying all of these tags (names)
    tags: Option<Vec<String>>,
    /// Only documents from this correspondent (name)
    correspondent: Option<String>,
    /// Only documents of this type (name)
    document_type: Option<String>,
    /// Only documents added on or after this date (YYYY-MM-DD or RFC 3339)
    date_from: Option<String>,
    /// Only documents added up to and including this date (YYYY-MM-DD), or before this instant (RFC 3339)
    date_to: Option<String>,
    /// Maximum number of results (default 10, max 50)
    limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DocumentSummary {
    id: String,
    title: String,
    correspondent: Option<String>,
    document_type: Option<String>,
    /// Date the document was added (YYYY-MM-DD)
    date: String,
    tags: Vec<String>,
    /// Semantic relevance, when available
    score: Option<f32>,
    snippet: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct SearchOutput {
    results: Vec<DocumentSummary>,
    count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetDocumentArgs {
    /// Document ID
    id: String,
    /// Position to continue reading from, as returned in `next_cursor`
    cursor: Option<String>,
    /// Maximum characters of text to return (default 8000, max 50000)
    max_chars: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DocumentOutput {
    id: String,
    title: String,
    correspondent: Option<String>,
    document_type: Option<String>,
    /// Date the document was added (YYYY-MM-DD)
    date: String,
    tags: Vec<String>,
    original_filename: Option<String>,
    mime_type: Option<String>,
    page_count: Option<u32>,
    /// OCR text for the requested range
    text: String,
    total_chars: usize,
    /// Pass as `cursor` to get the next part of the text; absent at the end
    next_cursor: Option<String>,
    resource_uri: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct NamedItem {
    id: String,
    name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct TagItem {
    id: String,
    name: String,
    color: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct TagsOutput {
    tags: Vec<TagItem>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct CorrespondentsOutput {
    correspondents: Vec<NamedItem>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DocumentTypesOutput {
    document_types: Vec<NamedItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateDocumentArgs {
    /// Document ID
    id: String,
    /// New title
    title: Option<String>,
    /// Correspondent name, created if new. Empty string clears it.
    correspondent: Option<String>,
    /// Document type name, created if new. Empty string clears it.
    document_type: Option<String>,
    /// Replaces ALL tags with these names (created if new). Use add_tags to keep existing tags.
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TagsArgs {
    /// Document ID
    id: String,
    /// Tag names
    tags: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct MetadataOutput {
    id: String,
    title: String,
    correspondent: Option<String>,
    document_type: Option<String>,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UploadArgs {
    /// File name including extension, e.g. "invoice-march.pdf"
    filename: String,
    /// File content, base64 encoded (max 25 MiB decoded)
    content_base64: String,
    /// Title; defaults to the file name
    title: Option<String>,
    /// MIME type; guessed from the file name when omitted
    mime_type: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct UploadOutput {
    id: String,
    /// OCR and indexing run in the background; poll get_processing_status
    processing_status: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct IdArgs {
    /// Document ID
    id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct StatusOutput {
    id: String,
    status: String,
}

#[tool_router]
impl FolioMcp {
    #[tool(
        description = "Search documents by words and/or metadata. Returns id, title, correspondent, type, date, tags and a text snippet per hit. Use get_document to read one.",
        annotations(
            title = "Search documents",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_documents(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<SearchOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;

        let query = DocumentQuery {
            text: args.query,
            tags: args.tags.unwrap_or_default(),
            correspondent: args.correspondent.filter(|s| !s.trim().is_empty()),
            document_type: args.document_type.filter(|s| !s.trim().is_empty()),
            created_after: args
                .date_from
                .as_deref()
                .map(|d| parse_date(d, false))
                .transpose()?,
            created_before: args
                .date_to
                .as_deref()
                .map(|d| parse_date(d, true))
                .transpose()?,
            limit: args
                .limit
                .unwrap_or(DEFAULT_SEARCH_LIMIT)
                .clamp(1, MAX_SEARCH_LIMIT),
        };
        let hits = self
            .library
            .search(vault, query)
            .await
            .map_err(tool_error)?;
        let names = self.names(vault).await?;

        let results: Vec<_> = hits
            .into_iter()
            .map(|hit| DocumentSummary {
                id: hit.document.id.to_string(),
                title: hit.document.title.clone(),
                correspondent: names.correspondent(&hit.document),
                document_type: names.document_type(&hit.document),
                date: day(hit.document.created_at),
                tags: tag_names(&hit.tags),
                score: hit.score,
                snippet: hit.snippet,
            })
            .collect();
        let count = results.len();
        Ok(Json(SearchOutput { results, count }))
    }

    #[tool(
        description = "Get a document's metadata and OCR text. Long text is returned in parts: pass the returned next_cursor as cursor to continue.",
        annotations(
            title = "Get document",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_document(
        &self,
        Parameters(args): Parameters<GetDocumentArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<DocumentOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;

        let id = parse_id(&args.id)?;
        let cursor = args
            .cursor
            .as_deref()
            .map(|c| {
                c.parse::<usize>()
                    .map_err(|_| format!("invalid cursor '{c}'"))
            })
            .transpose()?;
        let max_chars = args
            .max_chars
            .unwrap_or(DEFAULT_TEXT_CHARS)
            .clamp(1, MAX_TEXT_CHARS);

        let (detail, page) = self
            .library
            .read_document(vault, id, cursor, max_chars)
            .await
            .map_err(tool_error)?;
        let names = self.names(vault).await?;
        let doc = &detail.document;

        Ok(Json(DocumentOutput {
            id: doc.id.to_string(),
            title: doc.title.clone(),
            correspondent: names.correspondent(doc),
            document_type: names.document_type(doc),
            date: day(doc.created_at),
            tags: tag_names(&detail.tags),
            original_filename: doc.original_filename.clone(),
            mime_type: doc.mime_type.clone(),
            page_count: doc.page_count,
            text: page.text,
            total_chars: page.total_chars,
            next_cursor: page.next_cursor.map(|c| c.to_string()),
            resource_uri: format!("{DOCUMENT_URI_PREFIX}{}", doc.id),
        }))
    }

    #[tool(
        description = "List all tags (names to use in search_documents, update_document and add_tags).",
        annotations(
            title = "List tags",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_tags(&self, ctx: RequestContext<RoleServer>) -> ToolResult<TagsOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;
        let tags = self.library.list_tags(vault).await.map_err(tool_error)?;
        Ok(Json(TagsOutput {
            tags: tags
                .into_iter()
                .map(|t| TagItem {
                    id: t.id.to_string(),
                    name: t.name,
                    color: t.color,
                })
                .collect(),
        }))
    }

    #[tool(
        description = "List all correspondents (senders), by name.",
        annotations(
            title = "List correspondents",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_correspondents(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<CorrespondentsOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;
        let items = self
            .library
            .list_correspondents(vault)
            .await
            .map_err(tool_error)?;
        Ok(Json(CorrespondentsOutput {
            correspondents: items
                .into_iter()
                .map(|c| NamedItem {
                    id: c.id.to_string(),
                    name: c.name,
                })
                .collect(),
        }))
    }

    #[tool(
        description = "List all document types (e.g. invoice, contract), by name.",
        annotations(
            title = "List document types",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_document_types(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<DocumentTypesOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;
        let items = self
            .library
            .list_document_types(vault)
            .await
            .map_err(tool_error)?;
        Ok(Json(DocumentTypesOutput {
            document_types: items
                .into_iter()
                .map(|t| NamedItem {
                    id: t.id.to_string(),
                    name: t.name,
                })
                .collect(),
        }))
    }

    #[tool(
        description = "Update a document's title, correspondent, document type and/or tags. Only the fields you pass change. Passing tags replaces all existing tags; use add_tags/remove_tags to adjust them. Unknown correspondents, types and tags are created.",
        annotations(
            title = "Update document",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_document(
        &self,
        Parameters(args): Parameters<UpdateDocumentArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<MetadataOutput> {
        require_scope(&ctx, SCOPE_WRITE)?;
        let vault = self.vault(&ctx).await?;

        let id = parse_id(&args.id)?;
        let clearable = |v: Option<String>| {
            v.map(|s| {
                let s = s.trim().to_string();
                (!s.is_empty()).then_some(s)
            })
        };
        let detail = self
            .library
            .update_metadata(
                vault,
                id,
                MetadataUpdate {
                    title: args.title,
                    correspondent: clearable(args.correspondent),
                    document_type: clearable(args.document_type),
                    tags: args.tags,
                },
            )
            .await
            .map_err(tool_error)?;
        self.metadata_output(vault, detail).await
    }

    #[tool(
        description = "Add tags (by name, created if new) to a document, keeping its existing tags.",
        annotations(
            title = "Add tags",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_tags(
        &self,
        Parameters(args): Parameters<TagsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<MetadataOutput> {
        require_scope(&ctx, SCOPE_WRITE)?;
        let vault = self.vault(&ctx).await?;
        let id = parse_id(&args.id)?;
        let detail = self
            .library
            .add_tags(vault, id, &args.tags)
            .await
            .map_err(tool_error)?;
        self.metadata_output(vault, detail).await
    }

    #[tool(
        description = "Remove tags (by name) from a document; names it does not have are ignored. The tags themselves are kept.",
        annotations(
            title = "Remove tags",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_tags(
        &self,
        Parameters(args): Parameters<TagsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<MetadataOutput> {
        require_scope(&ctx, SCOPE_WRITE)?;
        let vault = self.vault(&ctx).await?;
        let id = parse_id(&args.id)?;
        let detail = self
            .library
            .remove_tags(vault, id, &args.tags)
            .await
            .map_err(tool_error)?;
        self.metadata_output(vault, detail).await
    }

    #[tool(
        description = "Add a file to the archive. Returns the new document id; OCR and indexing happen in the background, so poll get_processing_status before expecting text.",
        annotations(
            title = "Upload document",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn upload_document(
        &self,
        Parameters(args): Parameters<UploadArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<UploadOutput> {
        require_scope(&ctx, SCOPE_WRITE)?;
        let vault = self.vault(&ctx).await?;

        let data = base64::engine::general_purpose::STANDARD
            .decode(args.content_base64.trim())
            .map_err(|e| format!("content_base64 is not valid base64: {e}"))?;
        if data.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "file is too large ({} bytes, max {MAX_UPLOAD_BYTES})",
                data.len()
            ));
        }

        let (document, status) = self
            .library
            .upload(
                vault,
                NewUpload {
                    filename: args.filename,
                    data,
                    title: args.title,
                    mime_type: args.mime_type,
                    source: DocumentSource::of(SourceKind::Mcp),
                    ..Default::default()
                },
            )
            .await
            .map_err(tool_error)?;
        Ok(Json(UploadOutput {
            id: document.id.to_string(),
            processing_status: status_name(status),
        }))
    }

    #[tool(
        description = "Check whether a document's background processing (thumbnail, OCR, search indexing) is pending, processing, completed or failed.",
        annotations(
            title = "Get processing status",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_processing_status(
        &self,
        Parameters(args): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult<StatusOutput> {
        require_scope(&ctx, SCOPE_READ)?;
        let vault = self.vault(&ctx).await?;
        let id = parse_id(&args.id)?;
        let status = self
            .library
            .processing_status(vault, id)
            .await
            .map_err(tool_error)?;
        Ok(Json(StatusOutput {
            id: id.to_string(),
            status: status_name(status),
        }))
    }
}

impl FolioMcp {
    async fn metadata_output(
        &self,
        vault: VaultId,
        detail: DocumentDetail,
    ) -> ToolResult<MetadataOutput> {
        let names = self.names(vault).await?;
        let doc = &detail.document;
        Ok(Json(MetadataOutput {
            id: doc.id.to_string(),
            title: doc.title.clone(),
            correspondent: names.correspondent(doc),
            document_type: names.document_type(doc),
            tags: tag_names(&detail.tags),
        }))
    }

    async fn read_document_text(
        &self,
        vault: VaultId,
        id: Id,
    ) -> Result<ReadResourceResult, ErrorData> {
        let (detail, page) = self
            .library
            .read_document(vault, id, None, MAX_RESOURCE_TEXT_CHARS)
            .await
            .map_err(resource_error)?;
        let names = self
            .names(vault)
            .await
            .map_err(|e| ErrorData::internal_error(e, None))?;
        let doc = &detail.document;

        let mut text = format!("Title: {}\n", doc.title);
        if let Some(c) = names.correspondent(doc) {
            text.push_str(&format!("Correspondent: {c}\n"));
        }
        if let Some(t) = names.document_type(doc) {
            text.push_str(&format!("Type: {t}\n"));
        }
        text.push_str(&format!("Date: {}\n", day(doc.created_at)));
        if !detail.tags.is_empty() {
            text.push_str(&format!("Tags: {}\n", tag_names(&detail.tags).join(", ")));
        }
        text.push('\n');
        text.push_str(&page.text);
        if page.next_cursor.is_some() {
            text.push_str("\n\n[text truncated; use the get_document tool to page through it]");
        }

        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            format!("{DOCUMENT_URI_PREFIX}{id}"),
        )
        .with_mime_type("text/plain")]))
    }

    async fn read_document_file(
        &self,
        vault: VaultId,
        id: Id,
    ) -> Result<ReadResourceResult, ErrorData> {
        let (document, data) = self
            .library
            .read_file(vault, id)
            .await
            .map_err(resource_error)?
            .ok_or_else(|| ErrorData::resource_not_found("document has no stored file", None))?;
        if data.len() > MAX_BLOB_BYTES {
            return Err(ErrorData::invalid_params(
                format!(
                    "file is too large to inline ({} bytes, max {MAX_BLOB_BYTES}); use the text resource instead",
                    data.len()
                ),
                None,
            ));
        }
        let blob = base64::engine::general_purpose::STANDARD.encode(data);
        Ok(ReadResourceResult::new(vec![ResourceContents::blob(
            blob,
            format!("{DOCUMENT_URI_PREFIX}{id}/file"),
        )
        .with_mime_type(
            document
                .mime_type
                .unwrap_or_else(|| "application/octet-stream".into()),
        )]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FolioMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("folio", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        require_scope(&ctx, SCOPE_READ).map_err(forbidden)?;
        let vault = self.vault(&ctx).await.map_err(forbidden)?;
        let hits = self
            .library
            .search(
                vault,
                DocumentQuery {
                    limit: RESOURCE_LIST_LIMIT,
                    ..Default::default()
                },
            )
            .await
            .map_err(resource_error)?;
        let resources = hits
            .into_iter()
            .map(|h| {
                Resource::new(
                    format!("{DOCUMENT_URI_PREFIX}{}", h.document.id),
                    h.document.title,
                )
                .with_mime_type("text/plain")
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("folio://documents/{id}", "document-text")
                .with_description("Metadata and OCR text of a document")
                .with_mime_type("text/plain"),
            ResourceTemplate::new("folio://documents/{id}/file", "document-file")
                .with_description("The original file of a document, base64 encoded (up to 5 MiB)"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        require_scope(&ctx, SCOPE_READ).map_err(forbidden)?;
        let vault = self.vault(&ctx).await.map_err(forbidden)?;

        let rest = request
            .uri
            .strip_prefix(DOCUMENT_URI_PREFIX)
            .ok_or_else(|| {
                ErrorData::resource_not_found(format!("unknown resource {}", request.uri), None)
            })?;
        let (id, want_file) = match rest.strip_suffix("/file") {
            Some(id) => (id, true),
            None => (rest, false),
        };
        let id = parse_id(id).map_err(|e| ErrorData::invalid_params(e, None))?;

        let result = if want_file {
            self.read_document_file(vault, id).await?
        } else {
            self.read_document_text(vault, id).await?
        };
        Ok(result.into())
    }
}

impl Names {
    fn correspondent(&self, doc: &Document) -> Option<String> {
        doc.correspondent_id
            .and_then(|id| self.correspondents.get(&id).cloned())
    }

    fn document_type(&self, doc: &Document) -> Option<String> {
        doc.document_type_id
            .and_then(|id| self.document_types.get(&id).cloned())
    }
}

fn status_name(status: ProcessingStatus) -> String {
    match status {
        ProcessingStatus::Pending => "pending",
        ProcessingStatus::Processing => "processing",
        ProcessingStatus::Completed => "completed",
        ProcessingStatus::Failed => "failed",
    }
    .to_string()
}

fn tag_names(tags: &[Tag]) -> Vec<String> {
    tags.iter().map(|t| t.name.clone()).collect()
}

fn day(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d").to_string()
}

fn parse_id(s: &str) -> Result<Id, String> {
    Id::parse_str(s.trim()).map_err(|_| format!("'{s}' is not a valid document id"))
}

/// Accepts RFC 3339 or a plain date. `last_day` makes a plain date the final
/// day of an exclusive upper bound, i.e. midnight at the start of the next day.
fn parse_date(s: &str, last_day: bool) -> Result<DateTime<Utc>, String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s.trim()) {
        return Ok(dt.with_timezone(&Utc));
    }
    let date = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("'{s}' is not a date; use YYYY-MM-DD or RFC 3339"))?;
    let date = if last_day {
        date.succ_opt()
            .ok_or_else(|| format!("'{s}' is out of range"))?
    } else {
        date
    };
    Ok(date.and_time(NaiveTime::MIN).and_utc())
}

/// Domain errors the caller can act on are shown as-is; the rest stay in the log
fn tool_error(err: Error) -> String {
    match err {
        Error::NotFound { .. }
        | Error::Validation(_)
        | Error::Duplicate { .. }
        | Error::Conflict(_) => err.to_string(),
        other => {
            tracing::error!("MCP tool failed: {other}");
            "internal error while accessing the archive".to_string()
        }
    }
}

fn resource_error(err: Error) -> ErrorData {
    match err {
        Error::NotFound { .. } => ErrorData::resource_not_found(err.to_string(), None),
        Error::Validation(_) => ErrorData::invalid_params(err.to_string(), None),
        other => {
            tracing::error!("MCP resource failed: {other}");
            ErrorData::internal_error("internal error while accessing the archive", None)
        }
    }
}

fn forbidden(message: String) -> ErrorData {
    ErrorData::invalid_request(message, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_day_covers_the_whole_day() {
        let end = parse_date("2026-03-31", true).unwrap();
        assert_eq!(end.to_rfc3339(), "2026-04-01T00:00:00+00:00");
        let start = parse_date("2026-03-31", false).unwrap();
        assert_eq!(start.to_rfc3339(), "2026-03-31T00:00:00+00:00");
    }

    #[test]
    fn rfc3339_instants_are_used_as_given() {
        let at = parse_date("2026-03-31T10:00:00+02:00", true).unwrap();
        assert_eq!(at.to_rfc3339(), "2026-03-31T08:00:00+00:00");
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_date("yesterday", false).is_err());
    }
}
