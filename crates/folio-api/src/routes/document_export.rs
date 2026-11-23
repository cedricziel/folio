//! Build a checked, revision-bound ZIP before exposing any bytes to the client.

use async_zip::{tokio::write::ZipFileWriter, Compression, ZipEntryBuilder};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use folio_core::models::{
    ArchiveCompletion, ArchiveSource, ArchiveState, FileIdentity, Id, UserId, VaultId,
};
use folio_core::ports::DocumentStore;
use futures::{AsyncWriteExt as _, StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::{
    fs::File,
    io::{AsyncSeekExt, AsyncWriteExt, BufWriter, SeekFrom},
    sync::Semaphore,
};
use tokio_util::io::ReaderStream;

use super::documents::{content_disposition, FileContent};
use crate::{
    error::{ApiError, ApiResult, ErrorResponse},
    extract::ApiPath,
    openapi::{InternalError, NotFound},
    state::AppState,
    vault::{DefaultVault, Uploader},
};

const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TEXT_BYTES: i64 = 16 * 1024 * 1024;
const MAX_METADATA_BYTES: i64 = 128 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
// Hold the slot through response streaming so slow clients also count.
static EXPORT_SLOTS: Semaphore = Semaphore::const_new(4);

const ORIGINAL: &str = "original";
const ARCHIVAL: &str = "archival.pdf";
const TEXT: &str = "text.txt";
const MANIFEST: &str = "manifest.json";

#[derive(Serialize)]
struct Entry<'a> {
    path: &'static str,
    mime_type: &'a str,
    byte_size: u64,
    sha256: String,
}

impl<'a> Entry<'a> {
    fn binary(path: &'static str, file: &'a FileIdentity) -> Self {
        Self {
            path,
            mime_type: &file.mime_type,
            byte_size: file.byte_size,
            sha256: file.sha256.clone(),
        }
    }
}

/// Download a portable ZIP for the current verified document revision
#[utoipa::path(
    get,
    path = "/api/documents/{id}/export",
    operation_id = "exportDocument",
    tag = "documents",
    params(("id" = uuid::Uuid, Path, description = "Document ID")),
    responses(
        (status = 200, description = "Portable document ZIP", body = FileContent, content_type = "application/zip"),
        NotFound,
        (status = 409, description = "The source or archival copy changed during export; retry", body = ErrorResponse),
        (status = 413, description = "The document exceeds the portable export size limit", body = ErrorResponse),
        (status = 503, description = "All portable export slots are occupied; retry", body = ErrorResponse),
        InternalError,
    )
)]
pub(crate) async fn export_document(
    DefaultVault(vault): DefaultVault,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Id>,
) -> Response {
    let mut response = build_export(&state, vault, id, user.as_ref())
        .await
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    super::file_response::harden(&mut response);
    response
}

async fn build_export(
    state: &AppState,
    vault: VaultId,
    id: Id,
    user: Option<&UserId>,
) -> ApiResult<Response> {
    let permit = EXPORT_SLOTS.try_acquire().map_err(|_| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "Portable exports are busy")
            .with_retry_after(5)
    })?;
    let source = state
        .archival
        .current_source(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document has no verified original in this vault"))?;
    let Some(document) = state
        .archival
        .export_document(&source, MAX_TEXT_BYTES, MAX_METADATA_BYTES)
        .await?
    else {
        ensure_source(state, &source).await?;
        return Err(too_large());
    };
    let attempt = state.archival.current_attempt(vault, id).await?;
    let archive = match &attempt {
        Some(attempt) if attempt.source != source => return Err(changed()),
        Some(attempt) => match &attempt.state {
            ArchiveState::Completed(ArchiveCompletion::Ready(archive)) => {
                archive
                    .validate_metadata(&source, attempt.id)
                    .map_err(export_error)?;
                Some((attempt.id, archive))
            }
            _ => None,
        },
        None => None,
    };
    let binary_bytes = source
        .original
        .byte_size
        .saturating_add(archive.map_or(0, |(_, archive)| archive.file.byte_size));
    if binary_bytes > MAX_BINARY_BYTES {
        return Err(too_large());
    }

    // An anonymous file is reclaimed when the handler or the response body
    // drops it, including on cancellation. No detached task owns the data.
    let file = File::from_std(tempfile::tempfile().map_err(export_error)?);
    let mut zip = ZipFileWriter::with_tokio(BufWriter::new(file)).force_no_zip64();
    write_binary(&mut zip, state.store.as_ref(), ORIGINAL, &source.original).await?;
    let archival = if let Some((attempt_id, archive)) = archive {
        write_binary(&mut zip, state.store.as_ref(), ARCHIVAL, &archive.file).await?;
        Some(json!({
            "file": Entry::binary(ARCHIVAL, &archive.file),
            "attempt_id": attempt_id,
            "profile": archive.profile,
            "converter": {"name": archive.converter.name, "version": archive.converter.version},
            "validator": {"name": archive.validator.name, "version": archive.validator.version},
        }))
    } else {
        None
    };
    let text = if let Some(text) = &document.content {
        let data = text.as_bytes();
        zip.write_entry_whole(entry(TEXT), data)
            .await
            .map_err(export_error)?;
        Some(Entry {
            path: TEXT,
            mime_type: "text/plain; charset=utf-8",
            byte_size: data.len() as u64,
            sha256: hex::encode(Sha256::digest(data)),
        })
    } else {
        None
    };
    let manifest = serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "document": {
            "id": id,
            "vault_id": vault,
            "title": document.title,
            "original_filename": document.original_filename,
            "document_date": document.document_date,
            "created_at": document.created_at,
            "updated_at": document.updated_at,
            "correspondent_id": document.correspondent_id,
            "document_type_id": document.document_type_id,
            "page_count": document.page_count,
            "source": document.source,
        },
        "source_revision_id": source.revision_id,
        "original": Entry::binary(ORIGINAL, &source.original),
        "archival": archival,
        "text": text,
    }))
    .map_err(export_error)?;
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(too_large());
    }
    zip.write_entry_whole(entry(MANIFEST), &manifest)
        .await
        .map_err(export_error)?;
    let mut writer = zip.close().await.map_err(export_error)?.into_inner();
    writer.flush().await.map_err(export_error)?;
    let mut file = writer.into_inner();
    let size = file.metadata().await.map_err(export_error)?.len();
    file.seek(SeekFrom::Start(0)).await.map_err(export_error)?;

    if let Some(user) = user {
        let role = state.access.effective_role(user, vault).await?;
        if role.is_none() {
            return Err(ApiError::not_found("Vault not found"));
        }
    }
    if state.archival.current_attempt(vault, id).await? != attempt {
        return Err(changed());
    }
    ensure_source(state, &source).await?;

    let stream = ReaderStream::new(file).map(move |chunk| {
        let _lease = &permit;
        chunk
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, size)
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(&format!("folio-{id}.zip")),
        )
        .body(Body::from_stream(stream))
        .map_err(export_error)
}

fn entry(name: &'static str) -> ZipEntryBuilder {
    ZipEntryBuilder::new(name.into(), Compression::Stored)
}

async fn write_binary(
    zip: &mut ZipFileWriter<BufWriter<File>>,
    store: &dyn DocumentStore,
    name: &'static str,
    identity: &FileIdentity,
) -> ApiResult<()> {
    let mut object = store
        .get(&identity.storage_key)
        .await
        .map_err(export_error)?;
    if object.size.is_some_and(|size| size != identity.byte_size) {
        return Err(export_error(
            "Stored object size differs from its verified size",
        ));
    }
    let mut entry = zip
        .write_entry_stream(entry(name))
        .await
        .map_err(export_error)?;
    let mut digest = Sha256::new();
    let mut remaining = identity.byte_size;
    while let Some(chunk) = object.stream.try_next().await.map_err(export_error)? {
        remaining = remaining
            .checked_sub(chunk.len() as u64)
            .ok_or_else(|| export_error("Stored object exceeds its verified size"))?;
        digest.update(&chunk);
        entry.write_all(&chunk).await.map_err(export_error)?;
    }
    if remaining != 0 || hex::encode(digest.finalize()) != identity.sha256 {
        return Err(export_error(
            "Stored object differs from its verified identity",
        ));
    }
    entry.close().await.map_err(export_error)
}

async fn ensure_source(state: &AppState, source: &ArchiveSource) -> ApiResult<()> {
    if state
        .archival
        .current_source(source.vault_id, source.document_id)
        .await?
        .as_ref()
        != Some(source)
    {
        return Err(changed());
    }
    Ok(())
}

fn changed() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "Document source or archival copy changed during export",
    )
    .with_code("export_changed")
}

fn too_large() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "Document exceeds the portable export size limit",
    )
    .with_code("export_too_large")
}

fn export_error(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "Portable document export failed");
    ApiError::internal("Failed to build portable document export").with_code("export_failed")
}
