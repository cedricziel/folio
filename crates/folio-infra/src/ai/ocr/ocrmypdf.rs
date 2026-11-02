use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use image::ImageFormat;
use tokio::process::Command;

use folio_core::error::{Error, Result};
use folio_core::ports::OcrService;

use super::{tesseract, PageEngine, PageOcr, PageText};
use crate::ai::pdf::PageFormat;
use crate::ai::registry::ProviderContext;

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    Ok(Arc::new(PageOcr(OcrMyPdf {
        languages: ctx.settings.ocr.languages.trim().to_string(),
        tesseract: tesseract::build(ctx)?,
    })))
}

struct OcrMyPdf {
    languages: String,
    tesseract: Arc<dyn OcrService>,
}

#[async_trait]
impl PageEngine for OcrMyPdf {
    fn engine_id(&self) -> &'static str {
        "ocrmypdf"
    }

    fn page_format(&self) -> PageFormat {
        PageFormat::Png
    }

    async fn recognize(&self, data: &[u8], _: &str) -> Result<PageText> {
        let image = image::load_from_memory(data)
            .map_err(|e| Error::AiService(format!("Unsupported OCR image: {e}")))?;
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|e| Error::AiService(format!("Failed to convert OCR image: {e}")))?;
        let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
        let input = dir.path().join("input.png");
        let sidecar = dir.path().join("text.txt");
        tokio::fs::write(&input, png)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write OCR image: {e}")))?;

        let temporary_path = dir.path().canonicalize().map_err(|e| {
            Error::Storage(format!("Failed to resolve OCR temporary directory: {e}"))
        })?;
        let output = Command::new("ocrmypdf")
            .kill_on_drop(true)
            .env("TMPDIR", temporary_path)
            .args([
                "--output-type",
                "pdf",
                "--optimize",
                "0",
                "--image-dpi",
                "300",
                "--jobs",
                "1",
                "--language",
                &self.languages,
                "--sidecar",
            ])
            .arg(&sidecar)
            .arg(&input)
            .arg(dir.path().join("output.pdf"))
            .output()
            .await
            .map_err(tool_error)?;
        if !output.status.success() {
            return Err(Error::AiService(format!(
                "ocrmypdf failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let text = tokio::fs::read_to_string(sidecar)
            .await
            .map_err(|e| Error::AiService(format!("Failed to read OCRmyPDF text: {e}")))?;
        Ok(PageText {
            text,
            confidence: None,
        })
    }

    async fn health(&self) -> Result<()> {
        let output = Command::new("ocrmypdf")
            .kill_on_drop(true)
            .arg("--version")
            .output()
            .await
            .map_err(tool_error)?;
        if !output.status.success() {
            return Err(Error::AiService(format!(
                "ocrmypdf --version failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        self.tesseract.health().await
    }
}

fn tool_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::AiService("ocrmypdf not found; install ocrmypdf".into())
    } else {
        Error::AiService(format!("failed to run ocrmypdf: {error}"))
    }
}
