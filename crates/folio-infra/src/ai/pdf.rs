//! PDF helpers shared by OCR providers: embedded text layer and page rendering

use std::path::Path;

use tokio::process::Command;

use folio_core::error::{Error, Result};

/// A page needs at least this many letters or digits to count as having text
const MIN_ALNUM_PER_PAGE: usize = 20;

/// Share of pages that must carry text before the text layer replaces OCR
const MIN_TEXT_PAGE_RATIO: f32 = 0.8;

/// Image format produced by [`render_pages`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFormat {
    Png,
    Jpeg,
}

impl PageFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Png => "-png",
            Self::Jpeg => "-jpeg",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

/// Whether the text layer is substantial enough to skip OCR
pub fn has_meaningful_text(pages: &[String]) -> bool {
    if pages.is_empty() {
        return false;
    }
    let with_text = pages
        .iter()
        .filter(|p| p.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_ALNUM_PER_PAGE)
        .count();
    with_text as f32 / pages.len() as f32 >= MIN_TEXT_PAGE_RATIO
}

fn tool_error(tool: &str, err: std::io::Error) -> Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        Error::AiService(format!(
            "{tool} not found; install poppler-utils to process PDFs"
        ))
    } else {
        Error::AiService(format!("failed to run {tool}: {err}"))
    }
}

/// Extract the embedded text layer, one string per page
///
/// Returns an empty vector when the PDF has no pages with text.
pub async fn text_layer(data: &[u8]) -> Result<Vec<String>> {
    let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
    let pdf = dir.path().join("in.pdf");
    tokio::fs::write(&pdf, data)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {e}")))?;

    let output = Command::new("pdftotext")
        .kill_on_drop(true)
        .args(["-enc", "UTF-8"])
        .arg(&pdf)
        .arg("-")
        .output()
        .await
        .map_err(|e| tool_error("pdftotext", e))?;

    if !output.status.success() {
        return Err(Error::AiService(format!(
            "pdftotext failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut pages: Vec<String> = text.split('\x0c').map(str::to_string).collect();
    // pdftotext terminates every page with a form feed, leaving one empty tail.
    if pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    Ok(pages)
}

/// Render every page of a PDF to an image at the given resolution
pub async fn render_pages(data: &[u8], dpi: u32, format: PageFormat) -> Result<Vec<Vec<u8>>> {
    let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
    let pdf = dir.path().join("in.pdf");
    tokio::fs::write(&pdf, data)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {e}")))?;

    let output = Command::new("pdftoppm")
        .kill_on_drop(true)
        .args([format.flag(), "-r", &dpi.to_string()])
        .arg(&pdf)
        .arg(dir.path().join("page"))
        .output()
        .await
        .map_err(|e| tool_error("pdftoppm", e))?;

    if !output.status.success() {
        return Err(Error::AiService(format!(
            "PDF conversion failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut files = page_files(dir.path(), format.extension())?;
    files.sort_by_key(|(number, _)| *number);
    if files.is_empty() {
        return Err(Error::AiService(
            "PDF conversion produced no images".to_string(),
        ));
    }

    let mut images = Vec::with_capacity(files.len());
    for (_, path) in files {
        images.push(
            tokio::fs::read(&path)
                .await
                .map_err(|e| Error::Storage(format!("Failed to read page image: {e}")))?,
        );
    }
    Ok(images)
}

/// Collect `page-<n>.<ext>` files as `(n, path)`
fn page_files(dir: &Path, extension: &str) -> Result<Vec<(u32, std::path::PathBuf)>> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Storage(e.to_string()))?;
    Ok(entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let number = name
                .strip_prefix("page-")?
                .strip_suffix(&format!(".{extension}"))?
                .parse::<u32>()
                .ok()?;
            Some((number, path))
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tool_available(tool: &str) -> bool {
        std::process::Command::new(tool).arg("-v").output().is_ok()
    }

    /// Minimal one-page PDF whose text layer holds `text`
    pub fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 18 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];

        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn sparse_text_layer_is_not_meaningful() {
        assert!(!has_meaningful_text(&[]));
        assert!(!has_meaningful_text(&["  \n".to_string()]));
        assert!(!has_meaningful_text(&["p. 1".to_string()]));
    }

    #[test]
    fn full_text_layer_is_meaningful() {
        let page = "The quick brown fox jumps over the lazy dog".to_string();
        assert!(has_meaningful_text(&[page.clone(), page]));
    }

    #[test]
    fn one_text_page_among_many_scanned_pages_still_needs_ocr() {
        let text = "The quick brown fox jumps over the lazy dog".to_string();
        let mut pages = vec![String::new(); 9];
        pages.push(text);
        assert!(!has_meaningful_text(&pages));
    }

    #[tokio::test]
    async fn text_layer_reads_embedded_text() {
        if !tool_available("pdftotext") {
            eprintln!("skipping: pdftotext not installed");
            return;
        }
        let pdf = tiny_pdf("Invoice number 4711 for Folio");
        let pages = text_layer(&pdf).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].contains("Invoice number 4711"), "{pages:?}");
    }

    #[tokio::test]
    async fn render_pages_returns_one_png_per_page() {
        if !tool_available("pdftoppm") {
            eprintln!("skipping: pdftoppm not installed");
            return;
        }
        let pages = render_pages(&tiny_pdf("Hello"), 72, PageFormat::Png)
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(&pages[0][..4], b"\x89PNG");
    }
}
