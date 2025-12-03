//! Repository port traits

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentType, Id, Tag, UpdateCorrespondent, UpdateDocument, UpdateDocumentType,
    UpdateTag,
};

/// Repository for document operations
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// Find a document by ID
    async fn find_by_id(&self, id: Id) -> Result<Option<Document>>;

    /// List all documents with optional filtering
    async fn list(&self, filter: Option<DocumentFilter>) -> Result<Vec<Document>>;

    /// Create a new document
    async fn create(&self, params: CreateDocument) -> Result<Document>;

    /// Update an existing document
    async fn update(&self, id: Id, params: UpdateDocument) -> Result<Document>;

    /// Delete a document by ID
    async fn delete(&self, id: Id) -> Result<()>;

    /// Update the file path for a document
    async fn set_file_path(&self, id: Id, file_path: &str) -> Result<()>;

    /// Get tags for a document
    async fn get_tags(&self, document_id: Id) -> Result<Vec<Tag>>;

    /// Set tags for a document (replaces existing tags)
    async fn set_tags(&self, document_id: Id, tag_ids: &[Id]) -> Result<()>;

    /// Add a tag to a document
    async fn add_tag(&self, document_id: Id, tag_id: Id) -> Result<()>;

    /// Remove a tag from a document
    async fn remove_tag(&self, document_id: Id, tag_id: Id) -> Result<()>;
}

/// Repository for tag operations
#[async_trait]
pub trait TagRepository: Send + Sync {
    /// Find a tag by ID
    async fn find_by_id(&self, id: Id) -> Result<Option<Tag>>;

    /// Find a tag by name
    async fn find_by_name(&self, name: &str) -> Result<Option<Tag>>;

    /// List all tags
    async fn list(&self) -> Result<Vec<Tag>>;

    /// Create a new tag
    async fn create(&self, params: CreateTag) -> Result<Tag>;

    /// Update an existing tag
    async fn update(&self, id: Id, params: UpdateTag) -> Result<Tag>;

    /// Delete a tag by ID
    async fn delete(&self, id: Id) -> Result<()>;
}

/// Repository for correspondent operations
#[async_trait]
pub trait CorrespondentRepository: Send + Sync {
    /// Find a correspondent by ID
    async fn find_by_id(&self, id: Id) -> Result<Option<Correspondent>>;

    /// Find a correspondent by name
    async fn find_by_name(&self, name: &str) -> Result<Option<Correspondent>>;

    /// List all correspondents
    async fn list(&self) -> Result<Vec<Correspondent>>;

    /// Create a new correspondent
    async fn create(&self, params: CreateCorrespondent) -> Result<Correspondent>;

    /// Update an existing correspondent
    async fn update(&self, id: Id, params: UpdateCorrespondent) -> Result<Correspondent>;

    /// Delete a correspondent by ID
    async fn delete(&self, id: Id) -> Result<()>;
}

/// Repository for document type operations
#[async_trait]
pub trait DocumentTypeRepository: Send + Sync {
    /// Find a document type by ID
    async fn find_by_id(&self, id: Id) -> Result<Option<DocumentType>>;

    /// Find a document type by name
    async fn find_by_name(&self, name: &str) -> Result<Option<DocumentType>>;

    /// List all document types
    async fn list(&self) -> Result<Vec<DocumentType>>;

    /// Create a new document type
    async fn create(&self, params: CreateDocumentType) -> Result<DocumentType>;

    /// Update an existing document type
    async fn update(&self, id: Id, params: UpdateDocumentType) -> Result<DocumentType>;

    /// Delete a document type by ID
    async fn delete(&self, id: Id) -> Result<()>;
}
