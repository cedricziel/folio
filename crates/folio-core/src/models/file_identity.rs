use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileIdentity {
    pub sha256: String,
    pub byte_size: u64,
    pub mime_type: String,
    pub storage_key: String,
}

impl FileIdentity {
    /// Compute identity from server-held bytes, never from client checksum/size assertions.
    pub fn from_bytes(
        storage_key: impl Into<String>,
        mime_type: impl Into<String>,
        bytes: &[u8],
    ) -> Self {
        Self {
            sha256: hex::encode(Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
            mime_type: mime_type.into(),
            storage_key: storage_key.into(),
        }
    }
}
