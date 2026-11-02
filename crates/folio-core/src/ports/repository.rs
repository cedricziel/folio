//! Repository port traits

use async_trait::async_trait;

use crate::error::Result;
use crate::models::AuditEvent;
use crate::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentType, GroupId, Id, NewAuditEvent, Principal, ProcessingStage,
    ProcessingStatus, Setting, Tag, UpdateCorrespondent, UpdateDocument, UpdateDocumentType,
    UpdateTag, UserId, Vault, VaultId, VaultMember, VaultRole,
};

/// Repository for document operations
///
/// Every method that addresses a document or vocabulary entry takes the vault
/// it must live in; a row of another vault behaves as if it did not exist. The
/// exceptions are explicit: [`DocumentRepository::locate`] and the
/// background-work queries ([`DocumentRepository::list_all`], [`DocumentRepository::list_pending`],
/// [`DocumentRepository::reset_embedding_status`]), which span all vaults.
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// The vault a document lives in
    ///
    /// Callers must check access to the returned vault before using a scoped
    /// method.
    async fn locate(&self, id: Id) -> Result<Option<VaultId>>;

    /// Move a document between vaults in one transaction
    ///
    /// Tags, correspondent and document type are re-created by name in the
    /// target when missing. Writes an audit entry to both vaults.
    async fn move_to(
        &self,
        id: Id,
        from: VaultId,
        to: VaultId,
        actor: Option<&UserId>,
    ) -> Result<Document>;

    /// Find a document by ID
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>>;

    /// List all documents with optional filtering
    async fn list(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Document>>;

    /// Create a new document
    async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document>;

    /// Update an existing document
    async fn update(&self, vault: VaultId, id: Id, params: UpdateDocument) -> Result<Document>;

    /// Delete a document by ID
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;

    /// Update the file path for a document
    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()>;

    /// Update the page count for a document
    async fn set_page_count(&self, vault: VaultId, id: Id, page_count: u32) -> Result<()>;

    /// Set the status of one pipeline stage
    async fn set_stage_status(
        &self,
        vault: VaultId,
        id: Id,
        stage: ProcessingStage,
        status: ProcessingStatus,
    ) -> Result<()>;

    /// Store OCR output (possibly empty) and mark the OCR stage done
    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()>;

    /// Every document of every vault, for storage maintenance and counts
    async fn list_all(&self) -> Result<Vec<Document>>;

    /// List documents whose given stage is still pending, in every vault
    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>>;

    /// Mark the embedding stage pending for every document that has text, in every vault
    ///
    /// Returns the ids of the affected documents.
    async fn reset_embedding_status(&self) -> Result<Vec<Id>>;

    /// Get tags for a document
    async fn get_tags(&self, vault: VaultId, document_id: Id) -> Result<Vec<Tag>>;

    /// Set tags for a document (replaces existing tags)
    async fn set_tags(&self, vault: VaultId, document_id: Id, tag_ids: &[Id]) -> Result<()>;

    /// Add a tag to a document
    async fn add_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()>;

    /// Remove a tag from a document
    async fn remove_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()>;
}

/// Repository for tag operations
#[async_trait]
pub trait TagRepository: Send + Sync {
    /// Find a tag by ID
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Tag>>;

    /// Find a tag by name
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Tag>>;

    /// List all tags
    async fn list(&self, vault: VaultId) -> Result<Vec<Tag>>;

    /// Create a new tag
    async fn create(&self, vault: VaultId, params: CreateTag) -> Result<Tag>;

    /// Update an existing tag
    async fn update(&self, vault: VaultId, id: Id, params: UpdateTag) -> Result<Tag>;

    /// Delete a tag by ID
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;
}

/// Repository for correspondent operations
#[async_trait]
pub trait CorrespondentRepository: Send + Sync {
    /// Find a correspondent by ID
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Correspondent>>;

    /// Find a correspondent by name
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<Correspondent>>;

    /// List all correspondents
    async fn list(&self, vault: VaultId) -> Result<Vec<Correspondent>>;

    /// Create a new correspondent
    async fn create(&self, vault: VaultId, params: CreateCorrespondent) -> Result<Correspondent>;

    /// Update an existing correspondent
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateCorrespondent,
    ) -> Result<Correspondent>;

    /// Delete a correspondent by ID
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;
}

/// Repository for document type operations
#[async_trait]
pub trait DocumentTypeRepository: Send + Sync {
    /// Find a document type by ID
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<DocumentType>>;

    /// Find a document type by name
    async fn find_by_name(&self, vault: VaultId, name: &str) -> Result<Option<DocumentType>>;

    /// List all document types
    async fn list(&self, vault: VaultId) -> Result<Vec<DocumentType>>;

    /// Create a new document type
    async fn create(&self, vault: VaultId, params: CreateDocumentType) -> Result<DocumentType>;

    /// Update an existing document type
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        params: UpdateDocumentType,
    ) -> Result<DocumentType>;

    /// Delete a document type by ID
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;
}

/// Repository for application settings
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// Get a setting by key
    async fn get(&self, key: &str) -> Result<Option<Setting>>;

    /// Get all settings
    async fn list(&self) -> Result<Vec<Setting>>;

    /// Get settings by prefix (e.g., "ai." for all AI settings)
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Setting>>;

    /// Set a setting value
    async fn set(&self, key: &str, value: &str) -> Result<Setting>;

    /// Set multiple settings at once
    async fn set_many(&self, settings: &[(String, String)]) -> Result<()>;

    /// Delete a setting
    async fn delete(&self, key: &str) -> Result<()>;
}

/// Repository for vaults
#[async_trait]
pub trait VaultRepository: Send + Sync {
    /// Create a vault with no members
    async fn create(&self, name: &str, created_by: Option<&UserId>) -> Result<Vault>;

    async fn find(&self, id: VaultId) -> Result<Option<Vault>>;

    /// Vaults where the user holds a role (directly or through a group), with
    /// that role, ordered by name
    async fn list_for_user(&self, user: &UserId) -> Result<Vec<(Vault, VaultRole)>>;

    async fn rename(&self, id: VaultId, name: &str) -> Result<Vault>;

    /// Delete an empty vault with its vocabularies, workflows and grants
    ///
    /// Fails with [`crate::error::Error::Conflict`] while it holds documents.
    async fn delete(&self, id: VaultId) -> Result<()>;

    /// The user's default vault: the stored choice while they still hold a
    /// role in it, otherwise the first vault by name where they do, otherwise
    /// `None`
    async fn default_for_user(&self, user: &UserId) -> Result<Option<VaultId>>;

    async fn set_default(&self, user: &UserId, vault: VaultId) -> Result<()>;

    async fn provisioned(&self, user: &UserId) -> Result<bool>;
    async fn mark_provisioned(&self, user: &UserId) -> Result<()>;

    /// The vault used for requests and sources that have no user: the owner's
    /// default vault, else the oldest vault. Creates a member-less `Archive`
    /// when no vault exists at all.
    async fn instance_default(&self) -> Result<VaultId>;

    /// Vaults named `name` that have no direct user owner
    async fn find_ownerless(&self, name: &str) -> Result<Vec<VaultId>>;
}

/// Repository for grants
#[async_trait]
pub trait MembershipRepository: Send + Sync {
    async fn list(&self, vault: VaultId) -> Result<Vec<VaultMember>>;

    /// Grant a role, replacing any earlier grant of the same principal
    ///
    /// Fails with [`crate::error::Error::Conflict`] when it would downgrade
    /// the last owner held directly by a user.
    async fn grant(
        &self,
        vault: VaultId,
        principal: &Principal,
        role: VaultRole,
        granted_by: Option<&UserId>,
    ) -> Result<()>;

    /// Remove a grant; same last-owner rule as [`Self::grant`]
    async fn revoke(&self, vault: VaultId, principal: &Principal) -> Result<()>;

    /// Owners of the vault that are users, not groups
    async fn direct_owner_count(&self, vault: VaultId) -> Result<u64>;

    /// The group with this namespace and name, created when nobody has used it yet
    async fn ensure_group(&self, source: &str, name: &str) -> Result<GroupId>;
}

/// Append-only record of access changes and moves
#[async_trait]
pub trait AuditLog: Send + Sync {
    async fn record(&self, event: NewAuditEvent) -> Result<()>;

    /// Newest first
    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<AuditEvent>>;
}
