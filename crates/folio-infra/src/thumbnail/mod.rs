//! Thumbnail generation service

use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use tokio::fs;
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};

/// Default thumbnail dimensions
pub const THUMBNAIL_WIDTH: u32 = 300;
pub const THUMBNAIL_HEIGHT: u32 = 300;

/// Higher resolution for page viewing
pub const PAGE_WIDTH: u32 = 800;
pub const PAGE_HEIGHT: u32 = 1100;

/// Thumbnail service for generating document previews
pub struct ThumbnailService {
    storage_path: String,
    thumbnail_path: String,
}

impl ThumbnailService {
    /// Create a new thumbnail service
    pub fn new(storage_path: impl Into<String>) -> Self {
        let storage = storage_path.into();
        let thumbnail_path = format!("{}/thumbnails", storage);
        Self {
            storage_path: storage,
            thumbnail_path,
        }
    }

    /// Initialize the thumbnail directory
    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.thumbnail_path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to create thumbnail directory: {}", e)))?;
        Ok(())
    }

    /// Generate a thumbnail for a document
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
            self.generate_pdf_thumbnail(document_id, data).await?
        } else if let Some(text) = text_content {
            self.generate_text_thumbnail(text)?
        } else {
            return Err(Error::Storage(
                "Cannot generate thumbnail for this file type".into(),
            ));
        };

        // Save thumbnail
        let thumbnail_filename = format!("{}.jpg", document_id);
        let thumbnail_path = format!("{}/{}", self.thumbnail_path, thumbnail_filename);

        fs::write(&thumbnail_path, &thumbnail_data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to save thumbnail: {}", e)))?;

        debug!("Generated thumbnail: {}", thumbnail_path);
        Ok(format!("thumbnails/{}", thumbnail_filename))
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
    async fn generate_pdf_thumbnail(&self, document_id: Uuid, data: &[u8]) -> Result<Vec<u8>> {
        // Save PDF temporarily
        let temp_pdf = format!("{}/temp_{}.pdf", self.storage_path, document_id);
        let temp_output = format!("{}/temp_{}", self.storage_path, document_id);

        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        // Try pdftoppm (poppler-utils) first
        let result = Command::new("pdftoppm")
            .args([
                "-jpeg",
                "-f",
                "1",
                "-l",
                "1",
                "-scale-to",
                &THUMBNAIL_WIDTH.to_string(),
                &temp_pdf,
                &temp_output,
            ])
            .output();

        // Clean up temp PDF
        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                // pdftoppm creates file with -1 suffix
                let output_file = format!("{}-1.jpg", temp_output);
                if let Ok(thumbnail_data) = fs::read(&output_file).await {
                    let _ = fs::remove_file(&output_file).await;
                    return Ok(thumbnail_data);
                }
            }
            Ok(output) => {
                warn!(
                    "pdftoppm failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                warn!("pdftoppm not available: {}", e);
            }
        }

        // Try ImageMagick convert as fallback
        let output_file = format!("{}.jpg", temp_output);

        // Need to re-save temp PDF since we removed it
        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        let result = Command::new("convert")
            .args([
                "-density",
                "150",
                &format!("{}[0]", temp_pdf),
                "-resize",
                &format!("{}x{}", THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT),
                "-quality",
                "85",
                &output_file,
            ])
            .output();

        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                if let Ok(thumbnail_data) = fs::read(&output_file).await {
                    let _ = fs::remove_file(&output_file).await;
                    return Ok(thumbnail_data);
                }
            }
            Ok(output) => {
                warn!(
                    "ImageMagick convert failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                warn!("ImageMagick not available: {}", e);
            }
        }

        // Fallback: generate a placeholder image for PDF
        self.generate_placeholder_thumbnail("PDF", (239, 68, 68)) // Red color
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

    /// Get the path to a thumbnail
    pub fn get_thumbnail_path(&self, document_id: Uuid) -> String {
        format!("{}/thumbnails/{}.jpg", self.storage_path, document_id)
    }

    /// Check if a thumbnail exists
    pub async fn exists(&self, document_id: Uuid) -> bool {
        let path = self.get_thumbnail_path(document_id);
        Path::new(&path).exists()
    }

    /// Read a thumbnail
    pub async fn read(&self, document_id: Uuid) -> Result<Vec<u8>> {
        let path = self.get_thumbnail_path(document_id);
        fs::read(&path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to read thumbnail: {}", e)))
    }

    /// Delete a thumbnail
    pub async fn delete(&self, document_id: Uuid) -> Result<()> {
        let path = self.get_thumbnail_path(document_id);
        if Path::new(&path).exists() {
            fs::remove_file(&path)
                .await
                .map_err(|e| Error::Storage(format!("Failed to delete thumbnail: {}", e)))?;
        }
        Ok(())
    }

    /// Get the number of pages in a PDF
    pub async fn get_pdf_page_count(&self, data: &[u8]) -> Result<u32> {
        // Save PDF temporarily
        let temp_id = Uuid::new_v4();
        let temp_pdf = format!("{}/temp_{}.pdf", self.storage_path, temp_id);

        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        // Try pdfinfo first (from poppler-utils)
        let result = Command::new("pdfinfo").arg(&temp_pdf).output();

        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.starts_with("Pages:") {
                        if let Some(count_str) = line.split(':').nth(1) {
                            if let Ok(count) = count_str.trim().parse::<u32>() {
                                return Ok(count);
                            }
                        }
                    }
                }
                // Default to 1 if we can't parse
                Ok(1)
            }
            _ => {
                // Fallback: try pdftoppm to see how many pages it generates
                // This is more expensive but works as a fallback
                warn!("pdfinfo not available, assuming 1 page");
                Ok(1)
            }
        }
    }

    /// Generate a page thumbnail for a specific page
    #[instrument(skip(self, data))]
    pub async fn generate_page(&self, document_id: Uuid, data: &[u8], page: u32) -> Result<String> {
        let thumbnail_data = self
            .generate_pdf_page_thumbnail(document_id, data, page)
            .await?;

        // Save thumbnail - page 1 uses the default path for backwards compatibility
        let thumbnail_filename = if page == 1 {
            format!("{}.jpg", document_id)
        } else {
            format!("{}-p{}.jpg", document_id, page)
        };
        let thumbnail_path = format!("{}/{}", self.thumbnail_path, thumbnail_filename);

        fs::write(&thumbnail_path, &thumbnail_data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to save page thumbnail: {}", e)))?;

        debug!("Generated page {} thumbnail: {}", page, thumbnail_path);
        Ok(format!("thumbnails/{}", thumbnail_filename))
    }

    /// Generate thumbnail from a specific PDF page
    async fn generate_pdf_page_thumbnail(
        &self,
        document_id: Uuid,
        data: &[u8],
        page: u32,
    ) -> Result<Vec<u8>> {
        // Save PDF temporarily
        let temp_pdf = format!("{}/temp_{}.pdf", self.storage_path, document_id);
        let temp_output = format!("{}/temp_{}_p{}", self.storage_path, document_id, page);

        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        // Try pdftoppm (poppler-utils) first
        let page_str = page.to_string();
        let result = Command::new("pdftoppm")
            .args([
                "-jpeg",
                "-f",
                &page_str,
                "-l",
                &page_str,
                "-scale-to",
                &PAGE_WIDTH.to_string(),
                &temp_pdf,
                &temp_output,
            ])
            .output();

        // Clean up temp PDF
        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                // pdftoppm creates file with -{page} suffix
                let output_file = format!("{}-{}.jpg", temp_output, page);
                if let Ok(thumbnail_data) = fs::read(&output_file).await {
                    let _ = fs::remove_file(&output_file).await;
                    return Ok(thumbnail_data);
                }
            }
            Ok(output) => {
                warn!(
                    "pdftoppm failed for page {}: {}",
                    page,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                warn!("pdftoppm not available: {}", e);
            }
        }

        // Try ImageMagick convert as fallback
        let output_file = format!("{}.jpg", temp_output);

        // Need to re-save temp PDF since we removed it
        fs::write(&temp_pdf, data)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {}", e)))?;

        let result = Command::new("convert")
            .args([
                "-density",
                "150",
                &format!("{}[{}]", temp_pdf, page - 1), // ImageMagick uses 0-based index
                "-resize",
                &format!("{}x{}", PAGE_WIDTH, PAGE_HEIGHT),
                "-quality",
                "85",
                &output_file,
            ])
            .output();

        let _ = fs::remove_file(&temp_pdf).await;

        match result {
            Ok(output) if output.status.success() => {
                if let Ok(thumbnail_data) = fs::read(&output_file).await {
                    let _ = fs::remove_file(&output_file).await;
                    return Ok(thumbnail_data);
                }
            }
            Ok(output) => {
                warn!(
                    "ImageMagick convert failed for page {}: {}",
                    page,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                warn!("ImageMagick not available: {}", e);
            }
        }

        // Fallback: generate a placeholder image for this page
        self.generate_placeholder_thumbnail(&format!("Page {}", page), (239, 68, 68))
    }

    /// Get the path to a page thumbnail
    pub fn get_page_thumbnail_path(&self, document_id: Uuid, page: u32) -> String {
        if page == 1 {
            self.get_thumbnail_path(document_id)
        } else {
            format!(
                "{}/thumbnails/{}-p{}.jpg",
                self.storage_path, document_id, page
            )
        }
    }

    /// Check if a page thumbnail exists
    pub async fn page_exists(&self, document_id: Uuid, page: u32) -> bool {
        let path = self.get_page_thumbnail_path(document_id, page);
        Path::new(&path).exists()
    }

    /// Read a page thumbnail
    pub async fn read_page(&self, document_id: Uuid, page: u32) -> Result<Vec<u8>> {
        let path = self.get_page_thumbnail_path(document_id, page);
        fs::read(&path)
            .await
            .map_err(|e| Error::Storage(format!("Failed to read page thumbnail: {}", e)))
    }

    /// Delete all thumbnails for a document (including page thumbnails)
    pub async fn delete_all(&self, document_id: Uuid, page_count: Option<u32>) -> Result<()> {
        // Delete main thumbnail
        self.delete(document_id).await?;

        // Delete page thumbnails if we know the count
        if let Some(count) = page_count {
            for page in 2..=count {
                let path = self.get_page_thumbnail_path(document_id, page);
                if Path::new(&path).exists() {
                    let _ = fs::remove_file(&path).await;
                }
            }
        }

        Ok(())
    }
}
