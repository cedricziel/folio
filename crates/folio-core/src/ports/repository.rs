//! Repository port traits

use async_trait::async_trait;

use crate::error::Result;
use crate::models::AuditEvent;
use crate::models::{
    Correspondent, CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, Document,
    DocumentFilter, DocumentType, ExternalRef, GroupId, Id, ImportedDocument, NewAuditEvent,
    Principal, ProcessingStage, ProcessingStatus, Setting, Tag, UpdateCorrespondent,
    UpdateDocument, UpdateDocumentType, UpdateTag, UserId, Vault, VaultId, VaultMember, VaultRole,
};

/// Repository for document operations
///
/// Every method that addresses a document or vocabulary entry takes the vault
/// it must live in; a row of another vault behaves as if it did not exist. The
/// exceptions are explicit: [`DocumentRepository::locate`], [`DocumentRepository::locate_retained`],
/// [`DocumentRepository::locate_trashed`], [`DocumentRepository::list_purge_requested`] and the
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

    /// Set review from the server actor and clock; unchanged requests preserve attribution.
    async fn set_reviewed(&self, vault: VaultId, id: Id, reviewed: bool) -> Result<Document>;

    /// Find a document by ID
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>>;

    /// List all documents with optional filtering
    async fn list(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Document>>;

    /// Read a bounded, stably ordered page within a vault; reject mismatched cursors.
    async fn page(
        &self,
        vault: VaultId,
        query: crate::models::DocumentPageQuery,
    ) -> Result<crate::models::DocumentPage>;

    /// Ids of the documents [`list`](Self::list) would return, without loading their content
    async fn list_ids(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Id>>;

    /// Create a new document
    async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document>;

    /// Create an upload without announcing creation until its original is finalized.
    async fn create_for_upload(&self, vault: VaultId, params: CreateDocument) -> Result<Document>;

    /// [`create_for_upload`](Self::create_for_upload) under an id reserved in advance. A repeat returns the
    /// document already created under that id instead of a second one.
    async fn create_for_upload_as(
        &self,
        vault: VaultId,
        id: Id,
        params: CreateDocument,
    ) -> Result<Document>;

    /// Create an imported document with its tags, metadata values, notes and
    /// embedding job in one transaction, announced with origin `Import` so it
    /// starts no workflows. `Conflict` when the ID or the external reference
    /// already exists, trashed or not; `Validation` for values the vault
    /// can't hold.
    async fn create_imported(&self, vault: VaultId, document: ImportedDocument)
        -> Result<Document>;

    /// The document imported under `external`, trashed or not
    async fn find_by_external_ref(
        &self,
        vault: VaultId,
        external: &ExternalRef,
    ) -> Result<Option<Id>>;

    /// Create the reserved identity once while its ledger claim still matches, deferring its event.
    async fn create_for_ingest(
        &self,
        vault: VaultId,
        claim: &crate::models::LedgerEntry,
        params: CreateDocument,
    ) -> Result<Document>;

    /// Update an existing document
    async fn update(&self, vault: VaultId, id: Id, params: UpdateDocument) -> Result<Document>;

    /// Move a document to trash without deleting stored objects. Repeats succeed.
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;

    /// Locate a retained document regardless of trash state, in one read.
    async fn locate_retained(&self, id: Id) -> Result<Option<VaultId>>;

    /// Whether an original may be removed: the document is globally absent or irreversibly purging.
    async fn is_purging_or_missing(&self, id: Id) -> Result<bool>;

    /// Locate a trashed document, including one awaiting permanent deletion.
    async fn locate_trashed(&self, id: Id) -> Result<Option<VaultId>>;

    /// Read a trashed document within its vault.
    async fn find_trashed_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>>;

    /// List trash newest first, including pending permanent deletions.
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Document>>;

    /// Restore a trashed document; conflicts once permanent deletion was requested.
    async fn restore(&self, vault: VaultId, id: Id) -> Result<Document>;

    /// Persist irreversible deletion intent and a cleanup job atomically. Repeats succeed.
    async fn request_purge(&self, vault: VaultId, id: Id) -> Result<()>;

    /// Documents explicitly awaiting permanent deletion, across all vaults.
    async fn list_purge_requested(&self) -> Result<Vec<Document>>;

    /// Remove an explicitly requested document after its storage cleanup succeeds.
    async fn finalize_purge(&self, vault: VaultId, id: Id) -> Result<()>;

    /// Commit a stored original's path and announce creation once, atomically.
    /// The caller must successfully store the original before finalizing.
    async fn finalize_upload(
        &self,
        vault: VaultId,
        id: Id,
        file_path: &str,
        file_size: u64,
    ) -> Result<()>;

    /// Update the file path for a document; a new path forgets the recorded size
    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()>;

    /// Documents with a stored original whose size was never recorded, in ID
    /// order after `after`
    async fn list_missing_file_size(&self, after: Option<Id>, limit: u32) -> Result<Vec<Document>>;

    /// Record the size of the original at `file_path`. Does nothing once the
    /// document points at another file, and leaves `updated_at` alone, since
    /// the document itself did not change.
    async fn set_file_size(&self, vault: VaultId, id: Id, file_path: &str, size: u64)
        -> Result<()>;

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

    /// Store OCR output and atomically announce completion, including empty output.
    /// Repeating an unchanged successful result is a no-op.
    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()>;

    /// Every active document of every vault, for storage maintenance and counts
    async fn list_all(&self) -> Result<Vec<Document>>;

    /// List documents whose given stage is still pending, in every vault
    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>>;

    /// Mark the embedding stage pending for every document that has text, in every vault
    ///
    /// Returns the ids of the affected documents.
    async fn reset_embedding_status(&self) -> Result<Vec<Id>>;

    /// Get tags for a document
    async fn get_tags(&self, vault: VaultId, document_id: Id) -> Result<Vec<Tag>>;

    /// Read tags for the requested documents in a vault. Untagged and inaccessible IDs are omitted.
    async fn tags_for_documents(
        &self,
        vault: VaultId,
        document_ids: &[Id],
    ) -> Result<std::collections::HashMap<Id, Vec<Tag>>>;

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
    /// Read an entry including deleted entries retained by documents.
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<Tag>>;

    /// List deleted entries in this vault.
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Tag>>;

    /// Restore an entry, rejecting a live entry with the same name.
    async fn restore(&self, vault: VaultId, id: Id) -> Result<Tag>;

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
    /// Read an entry including deleted entries retained by documents.
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<Correspondent>>;

    /// List deleted entries in this vault.
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<Correspondent>>;

    /// Restore an entry, rejecting a live entry with the same name.
    async fn restore(&self, vault: VaultId, id: Id) -> Result<Correspondent>;

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
    /// Read an entry including deleted entries retained by documents.
    async fn find_retained(&self, vault: VaultId, id: Id) -> Result<Option<DocumentType>>;

    /// List deleted entries in this vault.
    async fn list_trash(&self, vault: VaultId) -> Result<Vec<DocumentType>>;

    /// Restore an entry, rejecting a live entry with the same name.
    async fn restore(&self, vault: VaultId, id: Id) -> Result<DocumentType>;

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
    /// Create a vault and its optional initial owner atomically, using scoped attribution.
    async fn create(&self, name: &str, owner: Option<&UserId>) -> Result<Vault>;

    async fn find(&self, id: VaultId) -> Result<Option<Vault>>;

    /// Deleted vaults recoverable by a current direct or group owner.
    async fn list_deleted_for_user(&self, user: &UserId) -> Result<Vec<Vault>>;

    /// Vaults where the user holds a role (directly or through a group), with
    /// that role, ordered by name
    async fn list_for_user(&self, user: &UserId) -> Result<Vec<(Vault, VaultRole)>>;

    async fn rename(&self, id: VaultId, name: &str) -> Result<Vault>;

    /// Retain a vault and its references while hiding it from ordinary access.
    /// Protected instance and account defaults must be changed before deletion.
    async fn delete(&self, id: VaultId) -> Result<()>;

    /// Restore a retained vault without restarting historical work.
    /// Callers must authorize a current owner before requesting restoration.
    async fn restore(&self, id: VaultId) -> Result<Vault>;

    /// The user's stored default while they still hold a role in it, otherwise `None`.
    async fn default_for_user(&self, user: &UserId) -> Result<Option<VaultId>>;

    async fn set_default(&self, user: &UserId, vault: VaultId) -> Result<()>;

    async fn provisioned(&self, user: &UserId) -> Result<bool>;
    async fn mark_provisioned(&self, user: &UserId) -> Result<()>;

    /// The vault used for requests and sources that have no user: the owner's
    /// default vault, else the oldest vault. Creates a member-less `Archive`
    /// when no vault exists at all.
    async fn instance_default(&self) -> Result<VaultId>;
}

/// Repository for grants
#[async_trait]
pub trait MembershipRepository: Send + Sync {
    async fn list(&self, vault: VaultId) -> Result<Vec<VaultMember>>;

    /// Grant a role, replacing any earlier grant of the same principal
    ///
    /// `granted_by` remains for source compatibility; scoped server attribution takes precedence.
    /// An identical role leaves the existing grant metadata and history unchanged.
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

    /// Those of `vaults` with a grant to anyone besides `user`. A group grant counts, even when
    /// `user` is the group's only member.
    async fn shared_beyond(
        &self,
        user: &UserId,
        vaults: &[VaultId],
    ) -> Result<std::collections::HashSet<VaultId>> {
        let me = Principal::User(user.clone());
        let mut shared = std::collections::HashSet::new();
        for &vault in vaults {
            if self.list(vault).await?.iter().any(|m| m.principal != me) {
                shared.insert(vault);
            }
        }
        Ok(shared)
    }

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
