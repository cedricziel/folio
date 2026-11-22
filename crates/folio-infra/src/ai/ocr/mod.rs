//! OCR providers
//!
//! To add a provider, write a module here that implements [`OcrService`]
//! (or [`PageEngine`] for engines that recognise one image at a time) and add
//! one `register` line to [`builtin`].

mod anthropic_vision;
mod none;
mod ocrmypdf;
mod ollama_vision;
mod openai_vision;
mod tesseract;

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
        .register("ocrmypdf", ocrmypdf::build)
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

impl<E: PageEngine> PageOcr<E> {
    async fn extract_pdf(&self, data: &[u8]) -> Result<OcrOutput> {
        let mut pages = match pdf::text_layer(data).await {
            Ok(pages) => pages,
            Err(error) => {
                warn!("Could not read PDF text layer: {error}");
                Vec::new()
            }
        };
        let needs_ocr = pages
            .iter()
            .enumerate()
            .filter(|(_, page)| !pdf::has_meaningful_text(page))
            .map(|(index, _)| index as u32 + 1)
            .collect::<Vec<_>>();
        let has_page_map = !pages.is_empty();
        if has_page_map && needs_ocr.is_empty() {
            return Ok(OcrOutput::from_pages("pdftotext", pages));
        }

        let format = self.0.page_format();
        let images = pdf::render_pages(
            data,
            PDF_RENDER_DPI,
            format,
            has_page_map.then_some(needs_ocr.as_slice()),
        )
        .await?;
        let mut confidences = Vec::new();
        for (index, image) in images.into_iter().enumerate() {
            let page = self.0.recognize(&image, format.mime_type()).await?;
            confidences.extend(page.confidence);
            if has_page_map {
                pages[needs_ocr[index] as usize - 1] = page.text;
            } else {
                pages.push(page.text);
            }
        }

        let mut output = OcrOutput::from_pages(self.0.engine_id(), pages);
        if !confidences.is_empty() {
            output.confidence = Some(confidences.iter().sum::<f32>() / confidences.len() as f32);
        }
        Ok(output)
    }
}

#[async_trait]
impl<E: PageEngine> OcrService for PageOcr<E> {
    async fn extract(&self, data: &[u8], mime_type: &str) -> Result<OcrOutput> {
        if mime_type == "application/pdf" {
            return self.extract_pdf(data).await;
        }

        let engine = self.0.engine_id();
        if mime_type.starts_with("image/") {
            let page = self.0.recognize(data, mime_type).await?;
            let mut output = OcrOutput::from_pages(engine, vec![page.text]);
            output.confidence = page.confidence;
            return Ok(output);
        }

        debug!("{engine}: nothing to recognise in {mime_type}");
        Ok(OcrOutput {
            engine: engine.to_string(),
            ..Default::default()
        })
    }

    async fn health(&self) -> Result<()> {
        self.0.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::pdf::tests::{tiny_pdf, tool_available};
    use folio_core::error::Error;
    use std::sync::Arc;

    struct Failing;

    #[async_trait]
    impl PageEngine for Failing {
        fn engine_id(&self) -> &'static str {
            "failing"
        }

        async fn recognize(&self, _: &[u8], _: &str) -> Result<PageText> {
            Err(Error::AiService("engine must not run".into()))
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    struct Fixed(&'static str);

    #[async_trait]
    impl PageEngine for Fixed {
        fn engine_id(&self) -> &'static str {
            "fixed"
        }

        async fn recognize(&self, _: &[u8], _: &str) -> Result<PageText> {
            Ok(PageText {
                text: self.0.to_string(),
                confidence: None,
            })
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingEngine(std::sync::atomic::AtomicUsize);

    #[async_trait]
    impl PageEngine for Arc<CountingEngine> {
        fn engine_id(&self) -> &'static str {
            "counting"
        }

        async fn recognize(&self, _: &[u8], _: &str) -> Result<PageText> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(PageText {
                text: "scan result".into(),
                confidence: Some(0.75),
            })
        }

        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn mixed_pdf_sends_only_the_scan_to_ocr_in_page_order() {
        for tool in ["pdftotext", "pdftoppm"] {
            if !tool_available(tool) {
                assert!(std::env::var_os("CI").is_none(), "CI must install {tool}");
                eprintln!("skipping: {tool} not installed");
                return;
            }
        }
        let engine = Arc::new(CountingEngine::default());
        let ocr = PageOcr(engine.clone());
        let pdf = include_bytes!("../../../tests/fixtures/mixed-digital-scan.pdf");
        let output = ocr.extract(pdf, "application/pdf").await.unwrap();
        assert_eq!(engine.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(output.pages.len(), 5);
        assert!(output.pages[1].starts_with("Page 2 "));
        assert_eq!(output.pages[2], "scan result");
        assert!(output.pages[3].starts_with("Page 4 "));
        assert_eq!(output.engine, "counting");
        assert_eq!(output.confidence, Some(0.75));
    }

    #[tokio::test]
    async fn text_layer_skips_the_engine() {
        if !tool_available("pdftotext") {
            eprintln!("skipping: pdftotext not installed");
            return;
        }
        let ocr = PageOcr(Failing);
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
        for tool in ["pdftotext", "pdftoppm"] {
            if !tool_available(tool) {
                eprintln!("skipping: {tool} not installed");
                return;
            }
        }
        let ocr = PageOcr(Fixed("scanned"));
        let out = ocr.extract(&tiny_pdf(""), "application/pdf").await.unwrap();
        assert_eq!(out.engine, "fixed");
    }

    #[tokio::test]
    async fn non_pdf_goes_straight_to_the_engine() {
        let ocr = PageOcr(Fixed("img"));
        assert_eq!(ocr.extract_text(b"x", "image/png").await.unwrap(), "img");
    }

    #[tokio::test]
    async fn ocrmypdf_reads_a_rendered_pdf_locally() {
        let mut ctx = crate::ai::registry::ProviderContext {
            settings: folio_core::models::PipelineSettings::default(),
            http: reqwest::Client::new(),
        };
        let ocr = builtin().create("ocrmypdf", &ctx).unwrap();
        if !tool_available("ocrmypdf") {
            assert!(std::env::var_os("CI").is_none(), "CI must install ocrmypdf");
            eprintln!("skipping: ocrmypdf not installed");
            return;
        }
        ocr.health().await.unwrap();
        let out = ocr
            .extract(
                include_bytes!("../../../tests/fixtures/mixed-digital-scan.pdf"),
                "application/pdf",
            )
            .await
            .unwrap();
        assert_eq!(out.engine, "ocrmypdf");
        assert!(out.text.contains("Cobalt Orchard 9716"), "{}", out.text);
        ctx.settings.ocr.languages = "eng+folio_missing_language".into();
        let unavailable = builtin().create("ocrmypdf", &ctx).unwrap();
        assert!(unavailable
            .health()
            .await
            .unwrap_err()
            .to_string()
            .contains("folio_missing_language"));
    }
}
