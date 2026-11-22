use bytes::Bytes;

use crate::models::{
    FileIdentity, Id, RevisionPublication, RevisionPublicationResult, RevisionPublicationSource,
    VaultId,
};
use crate::ports::{keys, DocumentRepository, DocumentStore};
use crate::{Error, Result};

pub struct NewRevisionOriginal {
    pub key: Id,
    pub expected_current: Id,
    pub filename: String,
    pub mime_type: String,
    pub bytes: Bytes,
}

/// Stage a replacement under a fresh key, verify the stored bytes, and publish conditionally.
pub async fn replace_document_original(
    documents: &dyn DocumentRepository,
    store: &dyn DocumentStore,
    vault: VaultId,
    document: Id,
    upload: NewRevisionOriginal,
) -> Result<RevisionPublicationResult> {
    if documents.find_by_id(vault, document).await?.is_none() {
        return Err(Error::NotFound {
            entity_type: "Document",
            id: document.to_string(),
        });
    }

    let revision = Id::new_v4();
    let key = keys::revision_original(document, revision);
    let expected = FileIdentity::from_bytes(&key, &upload.mime_type, &upload.bytes);
    if !store.put_if_absent(&key, upload.bytes).await? {
        return Err(Error::Conflict(
            "Replacement staging key already exists".into(),
        ));
    }

    let result = async {
        let identity = store
            .get(&key)
            .await?
            .file_identity(&key, &upload.mime_type)
            .await?;
        if identity != expected {
            return Err(Error::Storage(
                "Replacement bytes differ after storage".into(),
            ));
        }
        documents
            .publish_revision(
                vault,
                document,
                &RevisionPublication {
                    key: upload.key,
                    expected_current: upload.expected_current,
                    source: RevisionPublicationSource::Replacement {
                        revision,
                        filename: upload.filename,
                        identity,
                    },
                },
            )
            .await
    }
    .await;

    if !result.as_ref().is_ok_and(|result| result.created) {
        match documents
            .retains_revision_original(document, revision)
            .await
        {
            Ok(false) => {
                if let Err(error) = store.delete(&key).await {
                    tracing::warn!(%document, %revision, %error, "Could not remove staged revision");
                }
            }
            Ok(true) => {}
            Err(error) => {
                tracing::warn!(%document, %revision, %error, "Could not confirm staged revision ownership");
            }
        }
    }

    result
}
