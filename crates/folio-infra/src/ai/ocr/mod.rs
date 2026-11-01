//! OCR providers
//!
//! To add a provider, write a module here that implements [`OcrService`]
//! (or [`PageEngine`] for engines that recognise one image at a time) and add
//! one `register` line to [`builtin`].

mod anthropic_vision;
mod none;
mod ollama_vision;
mod openai_vision;
mod tesseract;

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use folio_core::error::Result;
use folio_core::ports::{OcrOutput, OcrService};

use super::pdf::{self, PageFormat};
use super::registry::OcrRegistry;

const PDF_RENDER_DPI: u32 = 300;

/// Instruction sent to vision models
pub(crate) const VISION_PROMPT: &str =
    "Extract all text from this document image. Return only the extracted text, nothing else.";

/// Registry with every built-in OCR provider
pub fn builtin() -> OcrRegistry {
    let mut r = OcrRegistry::new("OCR");
    r.register("none", none::build)
        .register("tesseract", tesseract::build)
        .register("openai-vision", openai_vision::build)
        .register("anthropic-vision", anthropic_vision::build)
        .register("ollama-vision", ollama_vision::build);
    r
}

/// Text recognised on one page or image
pub(crate) struct PageText {
    pub text: String,
    pub confidence: Option<f32>,
}

/// An engine that recognises text in a single image
#[async_trait]
pub(crate) trait PageEngine: Send + Sync + 'static {
    fn engine_id(&self) -> &'static str;

    /// Format used when a PDF page is rendered for this engine
    fn page_format(&self) -> PageFormat {
        PageFormat::Jpeg
    }

    async fn recognize(&self, image: &[u8], mime_type: &str) -> Result<PageText>;

    async fn health(&self) -> Result<()>;
}

/// Turns a [`PageEngine`] into an [`OcrService`] that handles images and PDFs
pub(crate) struct PageOcr<E>(pub E);

#[async_trait]
impl<E: PageEngine> OcrService for PageOcr<E> {
    async fn extract(&self, data: &[u8], mime_type: &str) -> Result<OcrOutput> {
        let engine = self.0.engine_id();

        let images: Vec<(Vec<u8>, &str)> = if mime_type == "application/pdf" {
            let format = self.0.page_format();
            pdf::render_pages(data, PDF_RENDER_DPI, format)
                .await?
                .into_iter()
                .map(|img| (img, format.mime_type()))
                .collect()
        } else if mime_type.starts_with("image/") {
            vec![(data.to_vec(), mime_type)]
        } else {
            debug!("{engine}: nothing to recognise in {mime_type}");
            return Ok(OcrOutput {
                engine: engine.to_string(),
                ..Default::default()
            });
        };

        let mut pages = Vec::with_capacity(images.len());
        let mut confidences = Vec::new();
        for (image, mime) in &images {
            let page = self.0.recognize(image, mime).await?;
            confidences.extend(page.confidence);
            pages.push(page.text);
        }

        let mut output = OcrOutput::from_pages(engine, pages);
        if !confidences.is_empty() {
            output.confidence = Some(confidences.iter().sum::<f32>() / confidences.len() as f32);
        }
        Ok(output)
    }

    async fn health(&self) -> Result<()> {
        self.0.health().await
    }
}

/// Uses a PDF's embedded text layer when it is substantial, otherwise defers to `inner`
pub struct TextLayerFirst {
    inner: Arc<dyn OcrService>,
}

impl TextLayerFirst {
    pub fn new(inner: Arc<dyn OcrService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl OcrService for TextLayerFirst {
    async fn extract(&self, data: &[u8], mime_type: &str) -> Result<OcrOutput> {
        if mime_type == "application/pdf" {
            match pdf::text_layer(data).await {
                Ok(pages) if pdf::has_meaningful_text(&pages) => {
                    debug!("Using embedded PDF text layer, skipping OCR");
                    return Ok(OcrOutput::from_pages("pdftotext", pages));
                }
                Ok(_) => {}
                Err(e) => warn!("Could not read PDF text layer: {e}"),
            }
        }
        self.inner.extract(data, mime_type).await
    }

    async fn health(&self) -> Result<()> {
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::pdf::tests::{tiny_pdf, tool_available};
    use folio_core::error::Error;

    struct Failing;

    #[async_trait]
    impl OcrService for Failing {
        async fn extract(&self, _: &[u8], _: &str) -> Result<OcrOutput> {
            Err(Error::AiService("engine must not run".into()))
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    struct Fixed(&'static str);

    #[async_trait]
    impl OcrService for Fixed {
        async fn extract(&self, _: &[u8], _: &str) -> Result<OcrOutput> {
            Ok(OcrOutput::from_pages("fixed", vec![self.0.to_string()]))
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn text_layer_skips_the_engine() {
        if !tool_available("pdftotext") {
            eprintln!("skipping: pdftotext not installed");
            return;
        }
        let ocr = TextLayerFirst::new(Arc::new(Failing));
        let out = ocr
            .extract(
                &tiny_pdf("Invoice number 4711 for Folio"),
                "application/pdf",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, "pdftotext");
        assert!(out.text.contains("Invoice number 4711"));
    }

    #[tokio::test]
    async fn pdf_without_text_falls_through_to_the_engine() {
        if !tool_available("pdftotext") {
            eprintln!("skipping: pdftotext not installed");
            return;
        }
        let ocr = TextLayerFirst::new(Arc::new(Fixed("scanned")));
        let out = ocr.extract(&tiny_pdf(""), "application/pdf").await.unwrap();
        assert_eq!(out.engine, "fixed");
    }

    #[tokio::test]
    async fn non_pdf_goes_straight_to_the_engine() {
        let ocr = TextLayerFirst::new(Arc::new(Fixed("img")));
        assert_eq!(ocr.extract_text(b"x", "image/png").await.unwrap(), "img");
    }
}
