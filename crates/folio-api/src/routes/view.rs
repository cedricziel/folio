//! View models shared by the page templates

use std::collections::HashMap;

use folio_core::models::{Correspondent, Document, DocumentStatus, DocumentType, Id, Tag, VaultId};

use crate::error::ApiResult;
use crate::state::AppState;
use crate::vault::DefaultVault;

/// Which sidebar entry is highlighted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Inbox,
    All,
    Manage,
    Settings,
    Workflows,
    None,
}

impl Nav {
    pub fn as_str(self) -> &'static str {
        match self {
            Nav::Inbox => "inbox",
            Nav::All => "all",
            Nav::Manage => "manage",
            Nav::Settings => "settings",
            Nav::Workflows => "workflows",
            Nav::None => "",
        }
    }
}

tokio::task_local! {
    /// The signed-in user of the request being handled, set by `enforce_auth`
    /// so page handlers need no extractor of their own.
    pub static CURRENT_USER: SidebarUser;
}

/// The signed-in user as the sidebar shows them
#[derive(Debug, Clone)]
pub struct SidebarUser {
    pub name: String,
    pub initials: String,
    /// "Owner", "Admin" or empty
    pub role: &'static str,
    pub csrf: String,
}

impl SidebarUser {
    pub fn new(user: &folio_auth::User, csrf: String) -> Self {
        let name = user
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&user.username)
            .to_string();
        let role = if user.is_owner {
            "Owner"
        } else if user.is_admin {
            "Admin"
        } else {
            ""
        };
        Self {
            initials: initials(&name),
            name,
            role,
            csrf,
        }
    }
}

/// Up to two uppercase letters: the first letters of the first two words
fn initials(name: &str) -> String {
    let mut letters: Vec<char> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() {
        letters.push('?');
    }
    letters.into_iter().collect()
}

/// Data the sidebar needs on every page
pub struct Sidebar {
    pub user: Option<SidebarUser>,
    pub active: &'static str,
    pub inbox_count: usize,
    pub total_count: usize,
    pub correspondents: Vec<Correspondent>,
    pub document_types: Vec<DocumentType>,
    pub tags: Vec<TagView>,
}

impl Sidebar {
    pub async fn load(state: &AppState, vault: VaultId, nav: Nav) -> ApiResult<Self> {
        let documents = state.document_repo.list(vault, None).await?;
        Self::build(state, vault, nav, &documents).await
    }

    /// Like `load`, for pages that already fetched every document
    pub async fn build(
        state: &AppState,
        vault: VaultId,
        nav: Nav,
        documents: &[Document],
    ) -> ApiResult<Self> {
        Ok(Self {
            user: CURRENT_USER.try_with(Clone::clone).ok(),
            active: nav.as_str(),
            inbox_count: inbox_count(documents),
            total_count: documents.len(),
            correspondents: state.correspondent_repo.list(vault).await?,
            document_types: state.document_type_repo.list(vault).await?,
            tags: state
                .tag_repo
                .list(vault)
                .await?
                .iter()
                .map(TagView::from)
                .collect(),
        })
    }
}

/// Documents that are still being read or wait for a person to look at them
pub fn inbox_count(documents: &[Document]) -> usize {
    documents
        .iter()
        .filter(|d| d.status() != DocumentStatus::Filed)
        .count()
}

/// The numbers shown in the sidebar and the review notice
pub struct Counts {
    pub inbox: usize,
    pub total: usize,
    pub review: usize,
    pub first_review: String,
    pub failed: usize,
    pub first_failed: String,
}

impl Counts {
    pub fn from_documents(documents: &[Document]) -> Self {
        let with_status = |status| documents.iter().filter(move |d| d.status() == status);
        let first_id = |status| {
            with_status(status)
                .next()
                .map(|d| d.id.to_string())
                .unwrap_or_default()
        };
        Self {
            inbox: inbox_count(documents),
            total: documents.len(),
            review: with_status(DocumentStatus::NeedsReview).count(),
            first_review: first_id(DocumentStatus::NeedsReview),
            failed: with_status(DocumentStatus::Failed).count(),
            first_failed: first_id(DocumentStatus::Failed),
        }
    }

    pub async fn load(state: &AppState, vault: VaultId) -> ApiResult<Self> {
        Ok(Self::from_documents(
            &state.document_repo.list(vault, None).await?,
        ))
    }
}

/// A tag with the palette tone it is drawn in
pub struct TagView {
    pub id: Id,
    pub name: String,
    pub tone: &'static str,
}

impl From<&Tag> for TagView {
    fn from(tag: &Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name.clone(),
            tone: tag.tone(),
        }
    }
}

/// "Reading 3 pages…", the status of a document that is still being read
pub fn reading_label(page_count: Option<u32>) -> String {
    match page_count {
        Some(1) => "Reading 1 page…".to_string(),
        Some(n) => format!("Reading {n} pages…"),
        None => "Reading…".to_string(),
    }
}

/// One row of a document list
pub struct DocumentRow {
    pub id: Id,
    pub title: String,
    pub from: Option<String>,
    pub doc_type: Option<String>,
    pub tags: Vec<TagView>,
    pub status: DocumentStatus,
    pub page_count: Option<u32>,
    pub date: String,
    pub version: i64,
}

impl DocumentRow {
    pub fn is_processing(&self) -> bool {
        self.status == DocumentStatus::Processing
    }

    pub fn needs_review(&self) -> bool {
        self.status == DocumentStatus::NeedsReview
    }

    pub fn is_failed(&self) -> bool {
        self.status == DocumentStatus::Failed
    }

    /// "From · Type", skipping whichever is not set
    pub fn subline(&self) -> String {
        [self.from.as_deref(), self.doc_type.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub fn reading_label(&self) -> String {
        reading_label(self.page_count)
    }
}

/// Builds list rows, resolving names and tags
pub async fn rows(
    DefaultVault(vault): DefaultVault,
    state: &AppState,
    documents: Vec<Document>,
) -> ApiResult<Vec<DocumentRow>> {
    let correspondents: HashMap<Id, String> = state
        .correspondent_repo
        .list(vault)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let types: HashMap<Id, String> = state
        .document_type_repo
        .list(vault)
        .await?
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();

    let mut out = Vec::with_capacity(documents.len());
    for doc in documents {
        let tags = state
            .document_repo
            .get_tags(vault, doc.id)
            .await?
            .iter()
            .map(TagView::from)
            .collect();
        out.push(row_from(&doc, &correspondents, &types, tags));
    }
    Ok(out)
}

/// Builds the row for a single document
pub async fn row(state: &AppState, document: Document) -> ApiResult<DocumentRow> {
    Ok(rows(state, vec![document]).await?.remove(0))
}

fn row_from(
    doc: &Document,
    correspondents: &HashMap<Id, String>,
    types: &HashMap<Id, String>,
    tags: Vec<TagView>,
) -> DocumentRow {
    DocumentRow {
        id: doc.id,
        title: doc.title.clone(),
        from: doc
            .correspondent_id
            .and_then(|id| correspondents.get(&id).cloned()),
        doc_type: doc.document_type_id.and_then(|id| types.get(&id).cloned()),
        tags,
        status: doc.status(),
        page_count: doc.page_count,
        date: doc.created_at.format("%Y-%m-%d").to_string(),
        version: doc.updated_at.timestamp_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio_core::models::ProcessingStatus;
    use folio_core::models::VaultId;

    #[test]
    fn counts_split_inbox_from_review() {
        let processing = Document::new(VaultId::new(), "a");
        let review = Document::new(VaultId::new(), "b").with_content("x");
        let filed = Document::new(VaultId::new(), "c")
            .with_content("x")
            .with_document_type(Id::new_v4());
        let mut unreadable = Document::new("d");
        unreadable.ocr_status = ProcessingStatus::Failed;
        let review_id = review.id.to_string();
        let failed_id = unreadable.id.to_string();
        let counts = Counts::from_documents(&[processing, review, filed, unreadable]);
        assert_eq!(counts.inbox, 3);
        assert_eq!(counts.total, 4);
        assert_eq!(counts.review, 1);
        assert_eq!(counts.first_review, review_id);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.first_failed, failed_id);
    }

    fn row(from: Option<&str>, doc_type: Option<&str>, pages: Option<u32>) -> DocumentRow {
        DocumentRow {
            id: Id::new_v4(),
            title: "t".into(),
            from: from.map(Into::into),
            doc_type: doc_type.map(Into::into),
            tags: vec![],
            status: DocumentStatus::Processing,
            page_count: pages,
            date: "2025-03-14".into(),
            version: 0,
        }
    }

    #[test]
    fn subline_skips_missing_parts() {
        assert_eq!(
            row(Some("Stadtwerke"), Some("Invoice"), None).subline(),
            "Stadtwerke · Invoice"
        );
        assert_eq!(row(None, Some("Invoice"), None).subline(), "Invoice");
        assert_eq!(row(None, None, None).subline(), "");
    }

    #[test]
    fn reading_label_counts_pages() {
        assert_eq!(row(None, None, Some(3)).reading_label(), "Reading 3 pages…");
        assert_eq!(row(None, None, Some(1)).reading_label(), "Reading 1 page…");
        assert_eq!(row(None, None, None).reading_label(), "Reading…");
    }

    #[test]
    fn initials_take_two_words_or_one_letter() {
        assert_eq!(initials("ada lovelace"), "AL");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("ada.king-lovelace"), "AK");
        assert_eq!(initials("--"), "?");
    }
}
