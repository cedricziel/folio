//! Thumbnail generation service

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use tokio::process::Command;
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::ports::{keys, DocumentStore};

/// Default thumbnail dimensions
pub const THUMBNAIL_WIDTH: u32 = 300;
pub const THUMBNAIL_HEIGHT: u32 = 300;

/// Higher resolution for page viewing
pub const PAGE_WIDTH: u32 = 800;
pub const PAGE_HEIGHT: u32 = 1100;

/// Thumbnail service for generating document previews
pub struct ThumbnailService {
    store: Arc<dyn DocumentStore>,
}

impl ThumbnailService {
    /// Create a new thumbnail service writing through `store`
    pub fn new(store: Arc<dyn DocumentStore>) -> Self {
        Self { store }
    }

    /// Generate a thumbnail for a document and return its object key
    #[instrument(skip(self, data))]
    pub async fn generate(
        &self,
        document_id: Uuid,
        data: &[u8],
        mime_type: &str,
        text_content: Option<&str>,
    ) -> Result<String> {
        let thumbnail_data = if mime_type.starts_with("image/") {
            self.generate_image_thumbnail(data)?
        } else if mime_type == "application/pdf" {
            self.generate_pdf_thumbnail(data).await?
        } else if let Some(text) = text_content {
            self.generate_text_thumbnail(text)?
        } else {
            return Err(Error::Storage(
                "Cannot generate thumbnail for this file type".into(),
            ));
        };

        let key = keys::thumbnail(document_id);
        self.store.put(&key, thumbnail_data.into()).await?;

        debug!("Generated thumbnail: {}", key);
        Ok(key)
    }

    /// Generate thumbnail from an image
    fn generate_image_thumbnail(&self, data: &[u8]) -> Result<Vec<u8>> {
        let img = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(|e| Error::Storage(format!("Failed to read image: {}", e)))?
            .decode()
            .map_err(|e| Error::Storage(format!("Failed to decode image: {}", e)))?;

        let thumbnail = img.resize(THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT, FilterType::Lanczos3);

        let mut buffer = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Jpeg)
            .map_err(|e| Error::Storage(format!("Failed to encode thumbnail: {}", e)))?;

        Ok(buffer)
    }

    /// Generate thumbnail from a PDF (first page)
    async fn generate_pdf_thumbnail(&self, data: &[u8]) -> Result<Vec<u8>> {
        match render_pdf_page(data, 1, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT).await {
            Some(jpeg) => Ok(jpeg),
            None => self.generate_placeholder_thumbnail("PDF", (239, 68, 68)), // Red color
        }
    }

    /// Generate thumbnail from text content
    fn generate_text_thumbnail(&self, text: &str) -> Result<Vec<u8>> {
        // Create a simple text preview image
        let width = THUMBNAIL_WIDTH;
        let height = THUMBNAIL_HEIGHT;

        let mut img = RgbaImage::new(width, height);

        // Fill with light background
        for pixel in img.pixels_mut() {
            *pixel = Rgba([248, 250, 252, 255]); // bg color
        }

        // Draw a simple border
        let border_color = Rgba([226, 232, 240, 255]);
        for x in 0..width {
            img.put_pixel(x, 0, border_color);
            img.put_pixel(x, height - 1, border_color);
        }
        for y in 0..height {
            img.put_pixel(0, y, border_color);
            img.put_pixel(width - 1, y, border_color);
        }

        // Draw text lines as simple horizontal bars (simulating text)
        let text_color = Rgba([100, 116, 139, 255]);
        let preview_text: String = text.chars().take(500).collect();
        let lines: Vec<&str> = preview_text.lines().take(12).collect();

        let line_height = 18u32;
        let margin = 20u32;
        let text_width = width - (margin * 2);

        for (i, line) in lines.iter().enumerate() {
            let y = margin + (i as u32 * line_height);
            if y + 8 >= height - margin {
                break;
            }

            // Draw line as a series of small rectangles (simulating characters)
            let line_len = line.len().min((text_width / 6) as usize);
            if line_len == 0 {
                continue;
            }

            for x in margin..(margin + (line_len as u32 * 6)).min(width - margin) {
                for dy in 0..4 {
                    if y + dy < height {
                        img.put_pixel(x, y + dy, text_color);
                    }
                }
            }
        }

        // Add "TXT" label at bottom
        let label_y = height - 30;
        let label_color = Rgba([71, 85, 105, 255]);
        for x in (width / 2 - 15)..(width / 2 + 15) {
            for y in label_y..(label_y + 12) {
                if x < width && y < height {
                    img.put_pixel(x, y, label_color);
                }
            }
        }

        let dynamic_img = DynamicImage::ImageRgba8(img);
        let mut buffer = Vec::new();
        dynamic_img
            .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Jpeg)
            .map_err(|e| Error::Storage(format!("Failed to encode text thumbnail: {}", e)))?;

        Ok(buffer)
    }

    /// Generate a placeholder thumbnail with label
    fn generate_placeholder_thumbnail(&self, _label: &str, color: (u8, u8, u8)) -> Result<Vec<u8>> {
        let width = THUMBNAIL_WIDTH;
        let height = THUMBNAIL_HEIGHT;

        let mut img = RgbaImage::new(width, height);

        // Fill with light background
        for pixel in img.pixels_mut() {
            *pixel = Rgba([241, 245, 249, 255]);
        }

        // Draw colored rectangle in center (file icon)
        let icon_width = 80u32;
        let icon_height = 100u32;
        let icon_x = (width - icon_width) / 2;
        let icon_y = (height - icon_height) / 2 - 10;

        let file_color = Rgba([color.0, color.1, color.2, 255]);

        for x in icon_x..(icon_x + icon_width) {
            for y in icon_y..(icon_y + icon_height) {
                if x < width && y < height {
                    img.put_pixel(x, y, file_color);
                }
            }
        }

        // Draw folded corner
        let corner_size = 20u32;
        let corner_color = Rgba([
            color.0.saturating_sub(30),
            color.1.saturating_sub(30),
            color.2.saturating_sub(30),
            255,
        ]);
        for i in 0..corner_size {
            for j in 0..(corner_size - i) {
                let x = icon_x + icon_width - corner_size + i;
                let y = icon_y + j;
                if x < width && y < height {
                    img.put_pixel(x, y, corner_color);
                }
            }
        }

        let dynamic_img = DynamicImage::ImageRgba8(img);
        let mut buffer = Vec::new();
        dynamic_img
            .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Jpeg)
            .map_err(|e| Error::Storage(format!("Failed to encode placeholder: {}", e)))?;

        Ok(buffer)
    }

    /// Check if a thumbnail exists
    pub async fn exists(&self, document_id: Uuid) -> bool {
        self.store
            .exists(&keys::thumbnail(document_id))
            .await
            .unwrap_or(false)
    }

    /// Read a thumbnail
    pub async fn read(&self, document_id: Uuid) -> Result<Vec<u8>> {
        self.read_page(document_id, 1).await
    }

    /// Delete a thumbnail
    pub async fn delete(&self, document_id: Uuid) -> Result<()> {
        self.store.delete(&keys::thumbnail(document_id)).await
    }

    /// Get the number of pages in a PDF
    pub async fn get_pdf_page_count(&self, data: &[u8]) -> Result<u32> {
        let dir = temp_dir()?;
        let pdf = dir.path().join("input.pdf");
        write_file(&pdf, data).await?;

        match Command::new("pdfinfo").arg(&pdf).output().await {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let pages = stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("Pages:"))
                    .and_then(|count| count.trim().parse::<u32>().ok());
                // Default to 1 if we can't parse
                Ok(pages.unwrap_or(1))
            }
            _ => {
                warn!("pdfinfo not available, assuming 1 page");
                Ok(1)
            }
        }
    }

    /// Generate a page thumbnail for a specific page and return its object key
    #[instrument(skip(self, data))]
    pub async fn generate_page(&self, document_id: Uuid, data: &[u8], page: u32) -> Result<String> {
        let thumbnail_data = match render_pdf_page(data, page, PAGE_WIDTH, PAGE_HEIGHT).await {
            Some(jpeg) => jpeg,
            None => {
                self.generate_placeholder_thumbnail(&format!("Page {}", page), (239, 68, 68))?
            }
        };

        let key = keys::page(document_id, page);
        self.store.put(&key, thumbnail_data.into()).await?;

        debug!("Generated page {} thumbnail: {}", page, key);
        Ok(key)
    }

    /// Check if a page thumbnail exists
    pub async fn page_exists(&self, document_id: Uuid, page: u32) -> bool {
        self.store
            .exists(&keys::page(document_id, page))
            .await
            .unwrap_or(false)
    }

    /// Read a page thumbnail
    pub async fn read_page(&self, document_id: Uuid, page: u32) -> Result<Vec<u8>> {
        self.store.get_bytes(&keys::page(document_id, page)).await
    }

    /// Delete all thumbnails for a document (including page thumbnails)
    pub async fn delete_all(&self, document_id: Uuid) -> Result<()> {
        self.store
            .delete_prefix(&keys::document_prefix(document_id))
            .await
            .map(|_| ())
    }
}

fn temp_dir() -> Result<tempfile::TempDir> {
    tempfile::tempdir().map_err(|e| Error::Storage(format!("Failed to create temp dir: {}", e)))
}

async fn write_file(path: &Path, data: &[u8]) -> Result<()> {
    tokio::fs::write(path, data)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write temp file: {}", e)))
}

/// Render one PDF page to JPEG with poppler, falling back to ImageMagick.
/// Returns `None` when neither tool produced an image.
async fn render_pdf_page(data: &[u8], page: u32, width: u32, height: u32) -> Option<Vec<u8>> {
    let dir = temp_dir().ok()?;
    let pdf = dir.path().join("input.pdf");
    write_file(&pdf, data).await.ok()?;

    let page_str = page.to_string();
    let out_prefix = dir.path().join("out");
    let result = Command::new("pdftoppm")
        .args(["-jpeg", "-f", &page_str, "-l", &page_str, "-scale-to"])
        .arg(width.to_string())
        .arg(&pdf)
        .arg(&out_prefix)
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => {
            // pdftoppm appends a zero-padded page number to the prefix.
            if let Some(jpeg) = read_first_jpeg(dir.path()).await {
                return Some(jpeg);
            }
        }
        Ok(output) => warn!(
            "pdftoppm failed for page {}: {}",
            page,
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(e) => warn!("pdftoppm not available: {}", e),
    }

    let fallback = dir.path().join("fallback.jpg");
    let result = Command::new("convert")
        .args(["-density", "150"])
        // ImageMagick uses a 0-based page index
        .arg(format!("{}[{}]", pdf.display(), page.saturating_sub(1)))
        .args([
            "-resize",
            &format!("{}x{}", width, height),
            "-quality",
            "85",
        ])
        .arg(&fallback)
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => tokio::fs::read(&fallback).await.ok(),
        Ok(output) => {
            warn!(
                "ImageMagick convert failed for page {}: {}",
                page,
                String::from_utf8_lossy(&output.stderr)
            );
            None
        }
        Err(e) => {
            warn!("ImageMagick not available: {}", e);
            None
        }
    }
}

async fn read_first_jpeg(dir: &Path) -> Option<Vec<u8>> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("out") && name.ends_with(".jpg") {
            return tokio::fs::read(entry.path()).await.ok();
        }
    }
    None
}
