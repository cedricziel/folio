//! PDF helpers shared by OCR providers: embedded text layer and page rendering

use std::path::Path;

use std::time::{Duration, Instant};

use tokio::process::Command;

use crate::ai::process;

use folio_core::error::{Error, Result};

/// A page needs at least this many letters or digits to count as having text
const MIN_ALNUM_PER_PAGE: usize = 20;

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

pub(super) fn has_meaningful_text(page: &str) -> bool {
    page.chars()
        .filter(|c| c.is_alphanumeric())
        .take(MIN_ALNUM_PER_PAGE)
        .count()
        == MIN_ALNUM_PER_PAGE
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

const BASE_LIMIT: Duration = Duration::from_secs(60);
const PER_PAGE_LIMIT: Duration = Duration::from_secs(6);
/// Page counts above this are treated as this many, so a hostile or corrupt
/// PDF cannot earn an unbounded limit
const MAX_PAGES: u32 = 2000;
/// Assumed size when `pdfinfo` cannot tell, so a big scan is not cut short
const UNKNOWN_PAGE_COUNT: u32 = 200;

/// Time a poppler run over a whole PDF may take: 60 s plus 6 s per page
///
/// A slow NAS renders a page in 1 to 2 s, so this leaves a few times the
/// expected time while still ending a stuck run.
fn limit_for_pages(pages: u32) -> Duration {
    BASE_LIMIT + PER_PAGE_LIMIT * pages.min(MAX_PAGES)
}

async fn limit_for(pdf: &Path) -> Duration {
    let pages = process::output_within(Command::new("pdfinfo").arg(pdf), process::INFO_TIMEOUT)
        .await
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find_map(|l| l.strip_prefix("Pages:")?.trim().parse::<u32>().ok())
        })
        .unwrap_or(UNKNOWN_PAGE_COUNT);
    limit_for_pages(pages)
}

/// Extract one string per PDF page, retaining empty pages.
pub async fn text_layer(data: &[u8]) -> Result<Vec<String>> {
    let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
    let pdf = dir.path().join("in.pdf");
    tokio::fs::write(&pdf, data)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {e}")))?;

    let limit = limit_for(&pdf).await;
    let output = process::output_within(
        Command::new("pdftotext")
            .args(["-enc", "UTF-8"])
            .arg(&pdf)
            .arg("-"),
        limit,
    )
    .await
    .map_err(|e| tool_error("pdftotext", e))?;

    if !output.status.success() {
        return Err(Error::AiService(format!(
            "pdftotext failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    // pdftotext terminates each page with a form feed, even if it has no text.
    let Some(pages) = text.strip_suffix('\x0c') else {
        return Ok(Vec::new());
    };
    Ok(pages.split('\x0c').map(str::to_string).collect())
}

/// Render the requested sorted page numbers, or every page when none are specified.
pub(super) async fn render_pages(
    data: &[u8],
    dpi: u32,
    format: PageFormat,
    page_numbers: Option<&[u32]>,
) -> Result<Vec<Vec<u8>>> {
    let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
    let pdf = dir.path().join("in.pdf");
    tokio::fs::write(&pdf, data)
        .await
        .map_err(|e| Error::Storage(format!("Failed to write temp PDF: {e}")))?;

    let (limit, ranges) = match page_numbers {
        Some(pages) => (
            limit_for_pages(pages.len().min(MAX_PAGES as usize) as u32),
            pages
                .chunk_by(|a, b| a.checked_add(1) == Some(*b))
                .map(|run| Some((run[0], run[run.len() - 1])))
                .collect::<Vec<_>>(),
        ),
        None => (limit_for(&pdf).await, vec![None]),
    };

    let deadline = Instant::now() + limit;
    let prefix = dir.path().join("page");
    for range in ranges {
        let mut command = Command::new("pdftoppm");
        command.args([format.flag(), "-r", &dpi.to_string()]);
        if let Some((first, last)) = range {
            command.args(["-f", &first.to_string(), "-l", &last.to_string()]);
        }
        let output = process::output_within(
            command.arg(&pdf).arg(&prefix),
            deadline.saturating_duration_since(Instant::now()),
        )
        .await
        .map_err(|e| tool_error("pdftoppm", e))?;
        if !output.status.success() {
            return Err(Error::AiService(format!(
                "PDF conversion failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }

    let mut files = page_files(dir.path(), format.extension())?;
    files.sort_by_key(|(number, _)| *number);
    if files.is_empty() {
        return Err(Error::AiService(
            "PDF conversion produced no images".to_string(),
        ));
    }

    if page_numbers.is_some_and(|pages| files.iter().map(|(n, _)| *n).ne(pages.iter().copied())) {
        return Err(Error::AiService(
            "PDF conversion did not produce the requested pages".into(),
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

    #[test]
    fn limit_grows_with_page_count() {
        assert_eq!(limit_for_pages(1), Duration::from_secs(66));
        // 250 pages at 1.5 s each take 375 s; the limit must leave room
        assert!(limit_for_pages(250) > Duration::from_secs(375 * 2));
    }

    #[test]
    fn limit_has_a_ceiling_for_absurd_page_counts() {
        assert!(limit_for_pages(u32::MAX) <= Duration::from_secs(60 * 60 * 6));
        assert_eq!(limit_for_pages(MAX_PAGES + 1), limit_for_pages(MAX_PAGES));
    }

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
    fn only_pages_with_enough_letters_or_digits_have_meaningful_text() {
        assert!(!has_meaningful_text(""));
        assert!(!has_meaningful_text("p. 1"));
        assert!(has_meaningful_text(
            "The quick brown fox jumps over the lazy dog"
        ));
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
        let pages = render_pages(&tiny_pdf("Hello"), 72, PageFormat::Png, None)
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(&pages[0][..4], b"\x89PNG");
    }
    #[tokio::test]
    async fn render_pages_accepts_disjoint_ranges() {
        if !tool_available("pdftoppm") {
            assert!(std::env::var_os("CI").is_none(), "CI must install pdftoppm");
            eprintln!("skipping: pdftoppm not installed");
            return;
        }
        let pdf = include_bytes!("../../tests/fixtures/mixed-digital-scan.pdf");
        let images = render_pages(pdf, 72, PageFormat::Png, Some(&[1, 3, 4]))
            .await
            .unwrap();
        assert_eq!(images.len(), 3);
    }
}
