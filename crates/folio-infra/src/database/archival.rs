use async_trait::async_trait;
use chrono::Utc;

use folio_core::models::{
    ArchiveAttempt, ArchiveCompletion, ArchiveSource, ArchiveState, ArchiveTool, FileIdentity, Id,
    ValidatedArchive, VaultId,
};
use folio_core::ports::ArchivalRepository;
use folio_core::{Error, Result};

use super::Database;
use crate::queue::{self, Job, JobType};

const ARCHIVE_CLEANUP_RETRY_SECONDS: i64 = 300;

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
    ", CAST(a.id AS TEXT) AS attempt_id, a.state, a.reason, a.output_storage_key,
          a.output_sha256, a.output_size,
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
    output_storage_key: Option<String>,
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
                    storage_key: required(self.output_storage_key)?,
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
    ($db:expr, |$pool:ident, $id:ident, $sqlite:ident, $insert:ident| $body:block) => {
        match $db {
            Database::Sqlite($pool) => {
                let $id = |id: Id| id.to_string();
                let $sqlite = true;
                #[allow(unused_imports)]
                use queue::insert_job_sqlite as $insert;
                $body
            }
            Database::Postgres($pool) => {
                let $id = |id: Id| id;
                let $sqlite = false;
                #[allow(unused_imports)]
                use queue::insert_job_postgres as $insert;
                $body
            }
        }
    };
}

// Match revision publication, trash and move locking. PostgreSQL reads the source
// in a separate statement after taking the locks, so it sees the committed winner.
macro_rules! write_archive {
    ($db:expr, $source:expr, |$tx:ident, $id:ident, $insert:ident, $sqlite:ident| $body:block) => {
        on_backend!($db, |pool, $id, $sqlite, $insert| {
            let mut $tx = if $sqlite {
                pool.begin_with("BEGIN IMMEDIATE").await
            } else {
                pool.begin().await
            }
            .map_err(repo_error)?;
            let vault = $source.vault_id;
            let document = $source.document_id;
            if !$sqlite
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
            // Serialize admission/publication with vault delete and restore too.
            // Read the source again after the lock to observe their committed state.
            if !$sqlite {
                sqlx::query("SELECT id FROM vaults WHERE id = $1 FOR SHARE")
                    .bind($id(vault.0))
                    .execute(&mut *$tx)
                    .await
                    .map_err(repo_error)?;
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
        on_backend!(self, |pool, id, _sqlite, _insert| {
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
        on_backend!(self, |pool, id, _sqlite, _insert| {
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
        write_archive!(self, source, |tx, id, insert_job, _sqlite| {
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
                insert_job(
                    &mut tx,
                    JobType::GenerateArchive {
                        vault_id: source.vault_id,
                        document_id: source.document_id,
                        source_revision_id: source.revision_id,
                        attempt_id: attempt,
                    },
                    chrono::Utc::now(),
                    queue::DEFAULT_MAX_ATTEMPTS,
                    None,
                )
                .await?;
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

        write_archive!(self, source, |tx, id, insert_job, _sqlite| {
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
                SET state = $1, reason = $2, output_storage_key = $3, output_sha256 = $4, output_size = $5,
                    profile = $6, converter_name = $7, converter_version = $8,
                    validator_name = $9, validator_version = $10 WHERE id = $11",
            )
            .bind(state)
            .bind(reason)
            .bind(output.map(|output| output.file.storage_key.as_str()))
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
            if output.is_some() {
                sqlx::query("DELETE FROM archive_output_cleanup WHERE attempt_id = $1")
                    .bind(id(attempt))
                    .execute(&mut *tx)
                    .await
                    .map_err(repo_error)?;
            }
            tx.commit().await.map_err(repo_error)?;
            Ok(true)
        })
    }

    async fn retains_archive(&self, document: Id, revision: Id, attempt: Id) -> Result<bool> {
        on_backend!(self, |pool, id, sqlite, _insert| {
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

impl Database {
    /// Discover verified originals without an attempt. A batch is safe to
    /// replay because admission compares the source and current attempt.
    pub(crate) async fn archive_candidates(
        &self,
        vault: Option<VaultId>,
        limit: u32,
    ) -> Result<Vec<(VaultId, Id)>> {
        const QUERY: &str = concat!(
            "SELECT CAST(d.vault_id AS TEXT), CAST(d.id AS TEXT)",
            live_original!(),
            " WHERE (d.vault_id = $1 OR $1 IS NULL)
              AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL AND NOT d.upload_pending
              AND (LOWER(TRIM(f.mime_type)) IN ('application/pdf', 'image/jpeg', 'image/png', 'image/tiff')
                   OR LOWER(TRIM(f.mime_type)) LIKE 'application/pdf;%' OR LOWER(TRIM(f.mime_type)) LIKE 'application/pdf %;%'
                   OR LOWER(TRIM(f.mime_type)) LIKE 'image/jpeg;%' OR LOWER(TRIM(f.mime_type)) LIKE 'image/jpeg %;%'
                   OR LOWER(TRIM(f.mime_type)) LIKE 'image/png;%' OR LOWER(TRIM(f.mime_type)) LIKE 'image/png %;%'
                   OR LOWER(TRIM(f.mime_type)) LIKE 'image/tiff;%' OR LOWER(TRIM(f.mime_type)) LIKE 'image/tiff %;%')
              AND NOT EXISTS (SELECT 1 FROM archival_attempts a
                  WHERE a.revision_id = r.id AND a.is_current)
              ORDER BY d.created_at, d.id LIMIT $2",
        );
        on_backend!(self, |pool, id, _sqlite, _insert| {
            let rows: Vec<(String, String)> = sqlx::query_as(QUERY)
                .bind(vault.map(|vault| id(vault.0)))
                .bind(i64::from(limit))
                .fetch_all(pool)
                .await
                .map_err(repo_error)?;
            rows.into_iter()
                .map(|(vault, document)| {
                    Ok((
                        VaultId(vault.parse().map_err(repo_error)?),
                        document.parse().map_err(repo_error)?,
                    ))
                })
                .collect()
        })
    }

    pub(crate) async fn reserve_archive_output(
        &self,
        source: &ArchiveSource,
        job: &Job,
    ) -> Result<bool> {
        let attempt = job.id;
        write_archive!(self, source, |tx, id, _insert, sqlite| {
            // Keep the claim live through staging. PostgreSQL recovery locks
            // the document before the job, the same order as this transaction.
            macro_rules! renew_claim {
                ($clock:literal) => {
                    concat!("UPDATE jobs SET lease_expires_at = ", $clock, " + $1
                         WHERE id = $2 AND claim_id = $3 AND job_type = 'generate_archive'
                         AND status = 'processing' AND lease_expires_at > ", $clock)
                };
            }
            let query = if sqlite {
                renew_claim!("unixepoch()")
            } else {
                renew_claim!("floor(extract(epoch FROM clock_timestamp()))::bigint")
            };
            let owns_claim = sqlx::query(query)
                .bind(queue::LEASE_SECONDS)
                .bind(id(attempt))
                .bind(job.claim_id.map(id))
                .execute(&mut *tx)
                .await
                .map_err(repo_error)?
                .rows_affected()
                == 1;
            if !owns_claim {
                return Ok(false);
            }
            let reserved = sqlx::query(
                "INSERT INTO archive_output_cleanup (attempt_id, document_id, revision_id, next_check_at)
                 SELECT id, document_id, revision_id, $1 FROM archival_attempts
                 WHERE id = $2 AND document_id = $3 AND revision_id = $4 AND is_current
                  AND state = 'queued'
                 ON CONFLICT (attempt_id) DO NOTHING",
            )
            .bind(Utc::now().timestamp())
            .bind(id(attempt))
            .bind(id(source.document_id))
            .bind(id(source.revision_id))
            .execute(&mut *tx)
            .await
            .map_err(repo_error)?
            .rows_affected()
                == 1;
            tx.commit().await.map_err(repo_error)?;
            Ok(reserved)
        })
    }

    /// Retire attempts whose queue delivery exhausted its retries, was
    /// quarantined, or was removed. A terminal attempt cannot publish.
    pub(crate) async fn retire_interrupted_archives(&self) -> Result<()> {
        on_backend!(self, |pool, _id, _sqlite, _insert| {
            sqlx::query(
                "UPDATE archival_attempts SET state = 'failed', reason = 'Archival job interrupted'
                 WHERE state = 'queued' AND NOT EXISTS (
                     SELECT 1 FROM jobs j WHERE j.id = archival_attempts.id
                     AND j.job_type = 'generate_archive' AND j.status IN ('pending', 'processing')
                 )",
            )
            .execute(pool)
            .await
            .map_err(repo_error)?;
            Ok(())
        })
    }

    /// An acknowledged create has finished, even if we crash before publication.
    pub(crate) async fn settle_archive_write(&self, attempt: Id) -> Result<()> {
        on_backend!(self, |pool, id, _sqlite, _insert| {
            sqlx::query(
                "UPDATE archive_output_cleanup SET write_settled = TRUE WHERE attempt_id = $1",
            )
            .bind(id(attempt))
            .execute(pool)
            .await
            .map_err(repo_error)?;
            Ok(())
        })
    }

    /// Defer each selected key before touching storage. Oldest due keys come
    /// first, and a failed check or a crashed repair cannot pin the first batch.
    pub(crate) async fn take_archive_cleanup_batch(&self) -> Result<Vec<ArchiveOutputCleanup>> {
        let now = Utc::now().timestamp();
        on_backend!(self, |pool, _id, _sqlite, _insert| {
            let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
                "UPDATE archive_output_cleanup SET next_check_at = $1
                 WHERE next_check_at <= $2 AND attempt_id IN (
                     SELECT c.attempt_id FROM archive_output_cleanup c
                     WHERE c.next_check_at <= $2
                       AND NOT EXISTS (SELECT 1 FROM archival_attempts a
                           WHERE a.id = c.attempt_id AND a.state = 'queued')
                       AND NOT EXISTS (SELECT 1 FROM jobs j
                           WHERE j.id = c.attempt_id AND j.status = 'processing')
                     ORDER BY c.next_check_at, c.attempt_id LIMIT 32
                 )
                 RETURNING CAST(document_id AS TEXT), CAST(revision_id AS TEXT),
                           CAST(attempt_id AS TEXT), write_settled",
            )
            .bind(now + ARCHIVE_CLEANUP_RETRY_SECONDS)
            .bind(now)
            .fetch_all(pool)
            .await
            .map_err(repo_error)?;
            rows.into_iter()
                .map(|(document, revision, attempt, write_settled)| {
                    Ok(ArchiveOutputCleanup {
                        document: document.parse().map_err(repo_error)?,
                        revision: revision.parse().map_err(repo_error)?,
                        attempt: attempt.parse().map_err(repo_error)?,
                        write_settled,
                    })
                })
                .collect()
        })
    }

    pub(crate) async fn acknowledge_archive_cleanup(&self, attempt: Id) -> Result<()> {
        on_backend!(self, |pool, id, _sqlite, _insert| {
            sqlx::query(
                "DELETE FROM archive_output_cleanup WHERE attempt_id = $1 AND write_settled",
            )
            .bind(id(attempt))
            .execute(pool)
            .await
            .map_err(repo_error)?;
            Ok(())
        })
    }
}

pub(crate) struct ArchiveOutputCleanup {
    pub document: Id,
    pub revision: Id,
    pub attempt: Id,
    pub write_settled: bool,
}
