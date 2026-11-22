use async_trait::async_trait;

use folio_core::models::{
    ArchiveAttempt, ArchiveCompletion, ArchiveSource, ArchiveState, ArchiveTool, FileIdentity, Id,
    ValidatedArchive, VaultId,
};
use folio_core::ports::{keys, ArchivalRepository};
use folio_core::{Error, Result};

use super::Database;

macro_rules! live_original {
    () => {
        " FROM documents d
          JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL
          JOIN document_revisions r ON r.id = d.current_revision_id AND r.document_id = d.id
          JOIN document_files f ON f.id = r.original_file_id AND f.document_id = d.id
              AND f.role = 'original' AND f.verification_state = 'verified'
              AND f.storage_key = d.file_path"
    };
}

macro_rules! live_scope {
    () => {
        " WHERE d.id = $1 AND d.vault_id = $2 AND d.deleted_at IS NULL
          AND d.purge_requested_at IS NULL AND NOT d.upload_pending"
    };
}

macro_rules! source_columns {
    () => {
        "CAST(r.id AS TEXT) AS revision_id, f.sha256, f.byte_size, f.mime_type, f.storage_key"
    };
}

const SOURCE: &str = concat!(
    "SELECT ",
    source_columns!(),
    live_original!(),
    live_scope!()
);
const ATTEMPT: &str = concat!(
    "SELECT ",
    source_columns!(),
    ", CAST(a.id AS TEXT) AS attempt_id, a.state, a.reason, a.output_sha256, a.output_size,
          a.profile, a.converter_name, a.converter_version, a.validator_name, a.validator_version",
    live_original!(),
    " JOIN archival_attempts a ON a.revision_id = r.id AND a.document_id = d.id
      AND a.source_sha256 = f.sha256 AND a.is_current",
    live_scope!(),
);
const LOCK_DOCUMENT: &str = concat!(
    "SELECT 1 FROM documents d
     JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL",
    live_scope!(),
    " FOR UPDATE OF d FOR SHARE OF v"
);

pub(super) const CANCEL_QUEUED: &str = "UPDATE archival_attempts
    SET state = 'skipped', reason = $1
    WHERE document_id = $2 AND is_current AND state = 'queued'";

fn repo_error(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

#[derive(sqlx::FromRow)]
struct SourceRow {
    revision_id: String,
    sha256: String,
    byte_size: i64,
    mime_type: String,
    storage_key: String,
}

impl SourceRow {
    fn into_source(self, vault: VaultId, document: Id) -> Result<ArchiveSource> {
        Ok(ArchiveSource {
            vault_id: vault,
            document_id: document,
            revision_id: self.revision_id.parse().map_err(repo_error)?,
            original: FileIdentity {
                sha256: self.sha256,
                byte_size: self.byte_size.try_into().map_err(repo_error)?,
                mime_type: self.mime_type,
                storage_key: self.storage_key,
            },
        })
    }
}

#[derive(sqlx::FromRow)]
struct AttemptRow {
    #[sqlx(flatten)]
    source: SourceRow,
    attempt_id: String,
    state: String,
    reason: Option<String>,
    output_sha256: Option<String>,
    output_size: Option<i64>,
    profile: Option<String>,
    converter_name: Option<String>,
    converter_version: Option<String>,
    validator_name: Option<String>,
    validator_version: Option<String>,
}

fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| repo_error("Incomplete archival attempt"))
}

impl AttemptRow {
    fn into_attempt(self, vault: VaultId, document: Id) -> Result<ArchiveAttempt> {
        let source = self.source.into_source(vault, document)?;
        let id = self.attempt_id.parse().map_err(repo_error)?;
        let completion = match self.state.as_str() {
            "queued" => {
                return Ok(ArchiveAttempt {
                    id,
                    source,
                    state: ArchiveState::Queued,
                })
            }
            "skipped" => ArchiveCompletion::Skipped(required(self.reason)?),
            "failed" => ArchiveCompletion::Failed(required(self.reason)?),
            "ready" => ArchiveCompletion::Ready(ValidatedArchive {
                file: FileIdentity {
                    sha256: required(self.output_sha256)?,
                    byte_size: required(self.output_size)?.try_into().map_err(repo_error)?,
                    mime_type: "application/pdf".into(),
                    storage_key: keys::revision_artifact(document, source.revision_id, id),
                },
                profile: required(self.profile)?,
                converter: ArchiveTool {
                    name: required(self.converter_name)?,
                    version: required(self.converter_version)?,
                },
                validator: ArchiveTool {
                    name: required(self.validator_name)?,
                    version: required(self.validator_version)?,
                },
            }),
            _ => return Err(repo_error("Invalid archival attempt state")),
        };
        Ok(ArchiveAttempt {
            id,
            source,
            state: ArchiveState::Completed(completion),
        })
    }
}

// Both databases accept numbered parameters in order. SQLite stores IDs as text.
macro_rules! on_backend {
    ($db:expr, |$pool:ident, $id:ident, $sqlite:ident| $body:block) => {
        match $db {
            Database::Sqlite($pool) => {
                let $id = |id: Id| id.to_string();
                let $sqlite = true;
                $body
            }
            Database::Postgres($pool) => {
                let $id = |id: Id| id;
                let $sqlite = false;
                $body
            }
        }
    };
}

// Match revision publication, trash and move locking. PostgreSQL reads the source
// in a separate statement after taking the locks, so it sees the committed winner.
macro_rules! write_archive {
    ($db:expr, $source:expr, |$tx:ident, $id:ident| $body:block) => {
        on_backend!($db, |pool, $id, sqlite| {
            let mut $tx = if sqlite {
                pool.begin_with("BEGIN IMMEDIATE").await
            } else {
                pool.begin().await
            }
            .map_err(repo_error)?;
            let vault = $source.vault_id;
            let document = $source.document_id;
            if !sqlite
                && sqlx::query_scalar::<_, i32>(LOCK_DOCUMENT)
                    .bind($id(document))
                    .bind($id(vault.0))
                    .fetch_optional(&mut *$tx)
                    .await
                    .map_err(repo_error)?
                    .is_none()
            {
                return Ok(false);
            }
            let current = sqlx::query_as::<_, SourceRow>(SOURCE)
                .bind($id(document))
                .bind($id(vault.0))
                .fetch_optional(&mut *$tx)
                .await
                .map_err(repo_error)?
                .map(|row| row.into_source(vault, document))
                .transpose()?;
            if current.as_ref() != Some($source) {
                return Ok(false);
            }
            $body
        })
    };
}

#[async_trait]
impl ArchivalRepository for Database {
    async fn current_source(&self, vault: VaultId, document: Id) -> Result<Option<ArchiveSource>> {
        on_backend!(self, |pool, id, _sqlite| {
            sqlx::query_as::<_, SourceRow>(SOURCE)
                .bind(id(document))
                .bind(id(vault.0))
                .fetch_optional(pool)
                .await
                .map_err(repo_error)?
                .map(|row| row.into_source(vault, document))
                .transpose()
        })
    }

    async fn current_attempt(
        &self,
        vault: VaultId,
        document: Id,
    ) -> Result<Option<ArchiveAttempt>> {
        on_backend!(self, |pool, id, _sqlite| {
            sqlx::query_as::<_, AttemptRow>(ATTEMPT)
                .bind(id(document))
                .bind(id(vault.0))
                .fetch_optional(pool)
                .await
                .map_err(repo_error)?
                .map(|row| row.into_attempt(vault, document))
                .transpose()
        })
    }

    async fn enqueue_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        expected_attempt: Option<Id>,
    ) -> Result<bool> {
        write_archive!(self, source, |tx, id| {
            let current = sqlx::query_scalar::<_, String>(
                "SELECT CAST(id AS TEXT) FROM archival_attempts WHERE revision_id = $1 AND is_current",
            ).bind(id(source.revision_id)).fetch_optional(&mut *tx).await.map_err(repo_error)?
                .map(|value| value.parse::<Id>()).transpose().map_err(repo_error)?;
            if current == Some(attempt) {
                return Ok(true);
            }
            if current != expected_attempt {
                return Ok(false);
            }
            sqlx::query("UPDATE archival_attempts SET is_current = FALSE WHERE revision_id = $1 AND is_current")
                .bind(id(source.revision_id)).execute(&mut *tx).await.map_err(repo_error)?;
            let inserted = sqlx::query(
                "INSERT INTO archival_attempts (id, document_id, revision_id, source_sha256)
                 VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
            )
            .bind(id(attempt))
            .bind(id(source.document_id))
            .bind(id(source.revision_id))
            .bind(&source.original.sha256)
            .execute(&mut *tx)
            .await
            .map_err(repo_error)?
            .rows_affected()
                == 1;
            if inserted {
                tx.commit().await.map_err(repo_error)?;
            }
            Ok(inserted)
        })
    }

    async fn complete_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        completion: &ArchiveCompletion,
    ) -> Result<bool> {
        let (state, reason, output) = match completion {
            ArchiveCompletion::Ready(output) => {
                output.validate_metadata(source, attempt)?;
                ("ready", None, Some(output))
            }
            ArchiveCompletion::Skipped(reason) => ("skipped", Some(reason.as_str()), None),
            ArchiveCompletion::Failed(reason) => ("failed", Some(reason.as_str()), None),
        };
        if reason.is_some_and(|reason| reason.trim().is_empty()) {
            return Err(Error::Validation("An archival reason is required".into()));
        }

        write_archive!(self, source, |tx, id| {
            let current = sqlx::query_as::<_, AttemptRow>(ATTEMPT)
                .bind(id(source.document_id))
                .bind(id(source.vault_id.0))
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_error)?
                .map(|row| row.into_attempt(source.vault_id, source.document_id))
                .transpose()?;
            let Some(current) = current.filter(|current| current.id == attempt) else {
                return Ok(false);
            };
            if let ArchiveState::Completed(existing) = current.state {
                return Ok(&existing == completion);
            }

            sqlx::query(
                "UPDATE archival_attempts
                SET state = $1, reason = $2, output_sha256 = $3, output_size = $4,
                    profile = $5, converter_name = $6, converter_version = $7,
                    validator_name = $8, validator_version = $9 WHERE id = $10",
            )
            .bind(state)
            .bind(reason)
            .bind(output.map(|output| output.file.sha256.as_str()))
            .bind(output.map(|output| output.file.byte_size as i64))
            .bind(output.map(|output| output.profile.as_str()))
            .bind(output.map(|output| output.converter.name.as_str()))
            .bind(output.map(|output| output.converter.version.as_str()))
            .bind(output.map(|output| output.validator.name.as_str()))
            .bind(output.map(|output| output.validator.version.as_str()))
            .bind(id(attempt))
            .execute(&mut *tx)
            .await
            .map_err(repo_error)?;
            tx.commit().await.map_err(repo_error)?;
            Ok(true)
        })
    }

    async fn retains_archive(&self, document: Id, revision: Id, attempt: Id) -> Result<bool> {
        on_backend!(self, |pool, id, sqlite| {
            let mut tx = if sqlite {
                pool.begin_with("BEGIN IMMEDIATE").await
            } else {
                pool.begin().await
            }
            .map_err(repo_error)?;
            if !sqlite {
                // Wait for an in-flight publication before checking for its commit.
                sqlx::query("SELECT id FROM documents WHERE id = $1 FOR UPDATE")
                    .bind(id(document))
                    .execute(&mut *tx)
                    .await
                    .map_err(repo_error)?;
            }
            let retained = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM archival_attempts
                 WHERE document_id = $1 AND revision_id = $2 AND id = $3 AND state = 'ready')",
            )
            .bind(id(document))
            .bind(id(revision))
            .bind(id(attempt))
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_error)?;
            tx.commit().await.map_err(repo_error)?;
            Ok(retained)
        })
    }
}
