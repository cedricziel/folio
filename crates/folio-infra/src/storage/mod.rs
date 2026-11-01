//! File storage implementation

use std::path::{Path, PathBuf};

use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::instrument;
use uuid::Uuid;

use folio_core::error::{Error, Result};

/// File storage service for storing document files
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    /// Create a new file storage service
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    /// Initialize the storage directory
    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.base_path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to create storage directory: {}", e)))?;
        Ok(())
    }

    /// Store a file and return its relative path
    #[instrument(skip(self, data))]
    pub async fn store(&self, document_id: Uuid, filename: &str, data: &[u8]) -> Result<String> {
        // Create a subdirectory based on document ID to avoid filename collisions
        let dir_path = self.base_path.join(document_id.to_string());
        fs::create_dir_all(&dir_path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to create document directory: {}", e)))?;

        // Sanitize filename
        let safe_filename = sanitize_filename(filename);
        let file_path = dir_path.join(&safe_filename);

        // Write the file
        let mut file = fs::File::create(&file_path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to create file: {}", e)))?;

        file.write_all(data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write file: {}", e)))?;

        // Return relative path
        Ok(format!("{}/{}", document_id, safe_filename))
    }

    /// Read a file by its relative path
    #[instrument(skip(self))]
    pub async fn read(&self, relative_path: &str) -> Result<Vec<u8>> {
        let file_path = self.base_path.join(relative_path);

        fs::read(&file_path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to read file: {}", e)))
    }

    /// Delete a file by its relative path
    #[instrument(skip(self))]
    pub async fn delete(&self, relative_path: &str) -> Result<()> {
        let file_path = self.base_path.join(relative_path);

        // Delete the file
        if file_path.exists() {
            fs::remove_file(&file_path)
                .await
                .map_err(|e| Error::Storage(format!("Failed to delete file: {}", e)))?;
        }

        // Try to remove the parent directory if empty
        if let Some(parent) = file_path.parent() {
            if parent != self.base_path {
                let _ = fs::remove_dir(parent).await; // Ignore errors (directory might not be empty)
            }
        }

        Ok(())
    }

    /// Get the full path to a stored file
    pub fn get_full_path(&self, relative_path: &str) -> PathBuf {
        self.base_path.join(relative_path)
    }

    /// Check if a file exists
    pub async fn exists(&self, relative_path: &str) -> bool {
        let file_path = self.base_path.join(relative_path);
        file_path.exists()
    }
}

/// Sanitize a filename to prevent path traversal and other issues
fn sanitize_filename(filename: &str) -> String {
    // Get just the filename part (no directory components)
    let filename = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Replace problematic characters
    let sanitized: String = filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Ensure the filename isn't empty and doesn't start with a dot
    if sanitized.is_empty() || sanitized.starts_with('.') {
        format!("document_{}", sanitized)
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("document.pdf"), "document.pdf");
        assert_eq!(sanitize_filename("my file.pdf"), "my_file.pdf");
        assert_eq!(sanitize_filename("../../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(".hidden"), "document_.hidden");
        assert_eq!(sanitize_filename(""), "document_");
    }
}
