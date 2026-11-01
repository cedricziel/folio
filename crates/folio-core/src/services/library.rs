//! Library service: the archive operations offered to AI assistants
//!
//! Names (tags, correspondents, document types) are the vocabulary
//! assistants work with, so this service resolves them to IDs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{Importer, SearchServiceFacade};
use crate::error::{Error, Result};
use crate::models::{
    Correspondent, CreateCorrespondent, CreateDocumentType, CreateTag, Document, DocumentFilter,
    DocumentSource, DocumentType, Id, Tag, UpdateDocument,
};
use crate::ports::{
    CorrespondentRepository, DocumentImporter, DocumentRepository, DocumentStore,
    DocumentTypeRepository, EmbeddingService, ProcessingQueue, ProcessingStatus, SearchService,
    TagRepository,
};

const SNIPPET_RADIUS: usize = 80;
const SEMANTIC_OVERFETCH: usize = 3;

/// Criteria for [`LibraryService::search`]
#[derive(Debug, Clone, Default)]
pub struct DocumentQuery {
    /// Free text, matched literally per term and semantically when embeddings are available
    pub text: Option<String>,
    /// Tag names; a document must carry all of them
    pub tags: Vec<String>,
    pub correspondent: Option<String>,
    pub document_type: Option<String>,
    /// Inclusive lower bound on the creation time
    pub created_after: Option<DateTime<Utc>>,
    /// Exclusive upper bound on the creation time
    pub created_before: Option<DateTime<Utc>>,
    pub limit: usize,
}

/// A search result
#[derive(Debug, Clone, Serialize)]
pub struct DocumentHit {
    pub document: Document,
    pub tags: Vec<Tag>,
    /// Semantic relevance, absent for purely literal matches
    pub score: Option<f32>,
    pub snippet: Option<String>,
}

/// A document with its tags
#[derive(Debug, Clone, Serialize)]
pub struct DocumentDetail {
    pub document: Document,
    pub tags: Vec<Tag>,
}

/// One page of a document's extracted text
#[derive(Debug, Clone, Serialize)]
pub struct TextPage {
    pub text: String,
    /// Pass back as the cursor to read the following page; `None` at the end
    pub next_cursor: Option<usize>,
    pub total_chars: usize,
}

/// Metadata changes; `None` leaves a field untouched
#[derive(Debug, Clone, Default)]
pub struct MetadataUpdate {
    pub title: Option<String>,
    /// `Some(None)` clears the correspondent
    pub correspondent: Option<Option<String>>,
    /// `Some(None)` clears the document type
    pub document_type: Option<Option<String>>,
    /// Replaces the full tag set
    pub tags: Option<Vec<String>>,
}

/// A file to add to the archive
#[derive(Debug, Clone, Default)]
pub struct NewUpload {
    pub filename: String,
    pub data: Vec<u8>,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    /// How the file arrived; defaults to a web upload
    pub source: DocumentSource,
    /// Tags attached together with the record
    pub tag_ids: Vec<Id>,
}

/// Archive operations shared by non-REST adapters
pub struct LibraryService {
    documents: Arc<dyn DocumentRepository>,
    tags: Arc<dyn TagRepository>,
    correspondents: Arc<dyn CorrespondentRepository>,
    document_types: Arc<dyn DocumentTypeRepository>,
    semantic: SearchServiceFacade,
    files: Arc<dyn DocumentStore>,
    importer: Importer,
    queue: Arc<dyn ProcessingQueue>,
}

impl LibraryService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        tags: Arc<dyn TagRepository>,
        correspondents: Arc<dyn CorrespondentRepository>,
        document_types: Arc<dyn DocumentTypeRepository>,
        embeddings: Arc<dyn EmbeddingService>,
        search: Arc<dyn SearchService>,
        files: Arc<dyn DocumentStore>,
        queue: Arc<dyn ProcessingQueue>,
    ) -> Self {
        let semantic = SearchServiceFacade::new(documents.clone(), embeddings, search);
        Self {
            importer: Importer::new(documents.clone(), files.clone(), queue.clone()),
            files,
            documents,
            tags,
            correspondents,
            document_types,
            semantic,
            queue,
        }
    }

    pub async fn list_tags(&self) -> Result<Vec<Tag>> {
        self.tags.list().await
    }

    pub async fn list_correspondents(&self) -> Result<Vec<Correspondent>> {
        self.correspondents.list().await
    }

    pub async fn list_document_types(&self) -> Result<Vec<DocumentType>> {
        self.document_types.list().await
    }

    /// Search documents by text and metadata
    pub async fn search(&self, query: DocumentQuery) -> Result<Vec<DocumentHit>> {
        let limit = query.limit.max(1);
        let text = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());

        let mut tag_ids = Vec::new();
        for name in &query.tags {
            tag_ids.push(self.find_tag(name).await?.id);
        }
        let correspondent_id = match &query.correspondent {
            Some(name) => Some(self.find_correspondent(name).await?.id),
            None => None,
        };
        let document_type_id = match &query.document_type {
            Some(name) => Some(self.find_document_type(name).await?.id),
            None => None,
        };

        let metadata_filter = DocumentFilter {
            correspondent_id,
            document_type_id,
            tag_ids: (!tag_ids.is_empty()).then_some(tag_ids),
            search_query: None,
            created_after: query.created_after,
            created_before: query.created_before,
        };

        let mut hits: Vec<(Document, Option<f32>)> = Vec::new();

        if let Some(text) = text {
            let allowed: HashSet<Id> = self
                .documents
                .list(Some(metadata_filter.clone()))
                .await?
                .into_iter()
                .map(|d| d.id)
                .collect();

            // Semantic search is best effort: it needs a configured AI provider
            let semantic: Vec<_> = self
                .semantic
                .search_documents(text, limit * SEMANTIC_OVERFETCH)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|s| allowed.contains(&s.document.id))
                .collect();
            let scores: HashMap<Id, f32> =
                semantic.iter().map(|s| (s.document.id, s.score)).collect();

            // Exact matches come first so that semantic neighbours only fill
            // the slots that are left
            let literal = self
                .documents
                .list(Some(DocumentFilter {
                    search_query: Some(text.to_string()),
                    ..metadata_filter
                }))
                .await?;
            let mut seen = HashSet::new();
            for doc in literal {
                seen.insert(doc.id);
                let score = scores.get(&doc.id).copied();
                hits.push((doc, score));
            }
            for s in semantic {
                if seen.insert(s.document.id) {
                    hits.push((s.document, Some(s.score)));
                }
            }
        } else {
            for doc in self.documents.list(Some(metadata_filter)).await? {
                hits.push((doc, None));
            }
        }

        hits.truncate(limit);

        let mut out = Vec::with_capacity(hits.len());
        for (document, score) in hits {
            let tags = self.documents.get_tags(document.id).await?;
            let snippet = document
                .content
                .as_deref()
                .and_then(|c| make_snippet(c, text));
            out.push(DocumentHit {
                document,
                tags,
                score,
                snippet,
            });
        }
        Ok(out)
    }

    pub async fn get_document(&self, id: Id) -> Result<DocumentDetail> {
        let document = self.require_document(id).await?;
        let tags = self.documents.get_tags(id).await?;
        Ok(DocumentDetail { document, tags })
    }

    /// A document with a slice of its extracted text, `cursor` being a character offset
    pub async fn read_document(
        &self,
        id: Id,
        cursor: Option<usize>,
        max_chars: usize,
    ) -> Result<(DocumentDetail, TextPage)> {
        let document = self.require_document(id).await?;
        let tags = self.documents.get_tags(id).await?;

        let content = document.content.as_deref().unwrap_or_default();
        let total_chars = content.chars().count();
        let start = cursor.unwrap_or(0).min(total_chars);
        let end = start.saturating_add(max_chars.max(1)).min(total_chars);
        let page = TextPage {
            text: content.chars().skip(start).take(end - start).collect(),
            next_cursor: (end < total_chars).then_some(end),
            total_chars,
        };
        Ok((DocumentDetail { document, tags }, page))
    }

    /// Original file bytes of a document, if one was stored
    pub async fn read_file(&self, id: Id) -> Result<Option<(Document, Vec<u8>)>> {
        let document = self.require_document(id).await?;
        match document.file_path.clone() {
            Some(path) => {
                let data = self.files.get_bytes(&path).await?;
                Ok(Some((document, data)))
            }
            None => Ok(None),
        }
    }

    pub async fn update_metadata(&self, id: Id, update: MetadataUpdate) -> Result<DocumentDetail> {
        self.require_document(id).await?;

        // Reject bad input before the first write so a failure changes nothing
        if update.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(Error::Validation("title must not be empty".into()));
        }
        if let Some(Some(name)) = &update.correspondent {
            non_empty(name, "correspondent")?;
        }
        if let Some(Some(name)) = &update.document_type {
            non_empty(name, "document type")?;
        }
        for name in update.tags.iter().flatten() {
            non_empty(name, "tag")?;
        }

        let correspondent_id = match update.correspondent {
            Some(Some(name)) => Some(Some(self.ensure_correspondent(&name).await?.id)),
            Some(None) => Some(None),
            None => None,
        };
        let document_type_id = match update.document_type {
            Some(Some(name)) => Some(Some(self.ensure_document_type(&name).await?.id)),
            Some(None) => Some(None),
            None => None,
        };
        let tag_ids = match &update.tags {
            Some(names) => {
                let mut ids = Vec::with_capacity(names.len());
                for name in names {
                    ids.push(self.ensure_tag(name).await?.id);
                }
                Some(ids)
            }
            None => None,
        };

        if update.title.is_some() || correspondent_id.is_some() || document_type_id.is_some() {
            self.documents
                .update(
                    id,
                    UpdateDocument {
                        title: update.title.map(|t| t.trim().to_string()),
                        content: None,
                        correspondent_id,
                        document_type_id,
                    },
                )
                .await?;
        }
        if let Some(ids) = tag_ids {
            self.documents.set_tags(id, &ids).await?;
        }

        self.get_document(id).await
    }

    /// Attach tags by name, creating missing ones
    pub async fn add_tags(&self, id: Id, names: &[String]) -> Result<DocumentDetail> {
        self.require_document(id).await?;
        for name in names {
            non_empty(name, "tag")?;
        }
        for name in names {
            let tag = self.ensure_tag(name).await?;
            self.documents.add_tag(id, tag.id).await?;
        }
        self.get_document(id).await
    }

    /// Detach tags by name; names that match no tag are ignored
    pub async fn remove_tags(&self, id: Id, names: &[String]) -> Result<DocumentDetail> {
        self.require_document(id).await?;
        for name in names {
            if let Some(tag) = self.lookup_tag(name).await? {
                self.documents.remove_tag(id, tag.id).await?;
            }
        }
        self.get_document(id).await
    }

    /// Store a file, create its document record, and queue processing
    pub async fn upload(&self, upload: NewUpload) -> Result<(Document, ProcessingStatus)> {
        if upload.data.is_empty() {
            return Err(Error::Validation("file content is empty".into()));
        }
        if upload.filename.trim().is_empty() {
            return Err(Error::Validation("filename must not be empty".into()));
        }

        let mime_type = upload
            .mime_type
            .unwrap_or_else(|| guess_mime_type(&upload.filename).to_string());
        let title = upload
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| upload.filename.clone());

        let document = self
            .importer
            .import(NewUpload {
                title: Some(title),
                mime_type: Some(mime_type),
                ..upload
            })
            .await?;
        let status = self.queue.processing_status(document.id).await?;
        Ok((document, status))
    }

    pub async fn processing_status(&self, id: Id) -> Result<ProcessingStatus> {
        self.require_document(id).await?;
        self.queue.processing_status(id).await
    }

    async fn require_document(&self, id: Id) -> Result<Document> {
        self.documents
            .find_by_id(id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            })
    }

    async fn lookup_tag(&self, name: &str) -> Result<Option<Tag>> {
        let name = name.trim();
        let tags = self.tags.list().await?;
        Ok(tags.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }

    async fn lookup_correspondent(&self, name: &str) -> Result<Option<Correspondent>> {
        let name = name.trim();
        let all = self.correspondents.list().await?;
        Ok(all.into_iter().find(|c| c.name.eq_ignore_ascii_case(name)))
    }

    async fn lookup_document_type(&self, name: &str) -> Result<Option<DocumentType>> {
        let name = name.trim();
        let all = self.document_types.list().await?;
        Ok(all.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }

    async fn find_tag(&self, name: &str) -> Result<Tag> {
        self.lookup_tag(name)
            .await?
            .ok_or_else(|| unknown("tag", name))
    }

    async fn find_correspondent(&self, name: &str) -> Result<Correspondent> {
        self.lookup_correspondent(name)
            .await?
            .ok_or_else(|| unknown("correspondent", name))
    }

    async fn find_document_type(&self, name: &str) -> Result<DocumentType> {
        self.lookup_document_type(name)
            .await?
            .ok_or_else(|| unknown("document type", name))
    }

    async fn ensure_tag(&self, name: &str) -> Result<Tag> {
        let name = non_empty(name, "tag")?;
        match self.lookup_tag(name).await? {
            Some(tag) => Ok(tag),
            None => {
                self.tags
                    .create(CreateTag {
                        name: name.to_string(),
                        color: None,
                    })
                    .await
            }
        }
    }

    async fn ensure_correspondent(&self, name: &str) -> Result<Correspondent> {
        let name = non_empty(name, "correspondent")?;
        match self.lookup_correspondent(name).await? {
            Some(found) => Ok(found),
            None => {
                self.correspondents
                    .create(CreateCorrespondent {
                        name: name.to_string(),
                    })
                    .await
            }
        }
    }

    async fn ensure_document_type(&self, name: &str) -> Result<DocumentType> {
        let name = non_empty(name, "document type")?;
        match self.lookup_document_type(name).await? {
            Some(found) => Ok(found),
            None => {
                self.document_types
                    .create(CreateDocumentType {
                        name: name.to_string(),
                    })
                    .await
            }
        }
    }
}

fn unknown(kind: &str, name: &str) -> Error {
    Error::Validation(format!("unknown {kind} '{name}'"))
}

fn non_empty<'a>(name: &'a str, kind: &str) -> Result<&'a str> {
    let name = name.trim();
    if name.is_empty() {
        Err(Error::Validation(format!("{kind} name must not be empty")))
    } else {
        Ok(name)
    }
}

/// A window of text around the first occurrence of any query term, or the
/// beginning of the content when there is no term to anchor on
fn make_snippet(content: &str, query: Option<&str>) -> Option<String> {
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let lowered: Vec<char> = content.to_lowercase().chars().collect();

    let anchor = query
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter_map(|term| {
            let term: Vec<char> = term.to_lowercase().chars().collect();
            find_subslice(&lowered, &term)
        })
        .min()
        .unwrap_or(0)
        .min(chars.len() - 1);

    let start = anchor.saturating_sub(SNIPPET_RADIUS);
    let end = (anchor + SNIPPET_RADIUS).min(chars.len());
    let body: String = chars[start..end].iter().collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");

    Some(format!(
        "{}{}{}",
        if start > 0 { "..." } else { "" },
        body,
        if end < chars.len() { "..." } else { "" }
    ))
}

fn find_subslice(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn guess_mime_type(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "txt" => "text/plain",
        "md" => "text/markdown",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_centres_on_first_matching_term() {
        let content = format!(
            "{} the invoice total {}",
            "x ".repeat(100),
            "y ".repeat(100)
        );
        let snippet = make_snippet(&content, Some("Total")).unwrap();
        assert!(snippet.contains("invoice total"));
        assert!(snippet.starts_with("..."));
        assert!(snippet.ends_with("..."));
    }

    #[test]
    fn snippet_falls_back_to_start_without_match() {
        let snippet = make_snippet("hello world", Some("nothing")).unwrap();
        assert_eq!(snippet, "hello world");
    }

    #[test]
    fn mime_guess_uses_extension() {
        assert_eq!(guess_mime_type("scan.PDF"), "application/pdf");
        assert_eq!(guess_mime_type("noext"), "application/octet-stream");
    }
}
