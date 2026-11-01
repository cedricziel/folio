//! Local OCR through the `tesseract` command line tool

use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use image::ImageFormat;
use tokio::process::Command;

use folio_core::error::{Error, Result};
use folio_core::ports::OcrService;

use super::{PageEngine, PageOcr, PageText};
use crate::ai::pdf::PageFormat;
use crate::ai::registry::ProviderContext;

pub fn build(ctx: &ProviderContext) -> Result<Arc<dyn OcrService>> {
    let languages = ctx.settings.ocr.languages.trim();
    if languages.is_empty()
        || languages.starts_with('+')
        || !languages
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+')
    {
        return Err(Error::AiService(format!(
            "Invalid OCR languages '{languages}'; use tesseract codes joined by '+', e.g. deu+eng"
        )));
    }
    Ok(Arc::new(PageOcr(Tesseract {
        languages: languages.to_string(),
    })))
}

struct Tesseract {
    languages: String,
}

#[async_trait]
impl PageEngine for Tesseract {
    fn engine_id(&self) -> &'static str {
        "tesseract"
    }

    fn page_format(&self) -> PageFormat {
        PageFormat::Png
    }

    async fn recognize(&self, image: &[u8], mime_type: &str) -> Result<PageText> {
        let dir = tempfile::tempdir().map_err(|e| Error::Storage(e.to_string()))?;
        let (bytes, extension) = prepare_image(image, mime_type)?;
        let path = dir.path().join(format!("page.{extension}"));
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp image: {e}")))?;

        let output = Command::new("tesseract")
            .arg(&path)
            .arg("stdout")
            .args(["-l", &self.languages, "tsv"])
            .output()
            .await
            .map_err(tool_error)?;

        if !output.status.success() {
            return Err(Error::AiService(format!(
                "tesseract failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        Ok(parse_tsv(&String::from_utf8_lossy(&output.stdout)))
    }

    async fn health(&self) -> Result<()> {
        let output = Command::new("tesseract")
            .arg("--list-langs")
            .output()
            .await
            .map_err(tool_error)?;
        if !output.status.success() {
            return Err(Error::AiService(
                "tesseract --list-langs failed".to_string(),
            ));
        }

        let installed = String::from_utf8_lossy(&output.stdout);
        let missing: Vec<&str> = self
            .languages
            .split('+')
            .filter(|lang| !installed.lines().any(|l| l.trim() == *lang))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(Error::AiService(format!(
                "tesseract language data missing: {}",
                missing.join(", ")
            )))
        }
    }
}

fn tool_error(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::AiService("tesseract not found; install tesseract-ocr".to_string())
    } else {
        Error::AiService(format!("failed to run tesseract: {e}"))
    }
}

/// Pass formats tesseract reads natively through; re-encode everything else as PNG
fn prepare_image(image: &[u8], mime_type: &str) -> Result<(Vec<u8>, &'static str)> {
    match mime_type {
        "image/png" => Ok((image.to_vec(), "png")),
        "image/jpeg" => Ok((image.to_vec(), "jpg")),
        "image/tiff" => Ok((image.to_vec(), "tif")),
        _ => {
            let decoded = image::load_from_memory(image)
                .map_err(|e| Error::AiService(format!("Unsupported image: {e}")))?;
            let mut png = Vec::new();
            decoded
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .map_err(|e| Error::AiService(format!("Failed to convert image: {e}")))?;
            Ok((png, "png"))
        }
    }
}

/// Rebuild reading-order text and mean word confidence from tesseract's TSV output
fn parse_tsv(tsv: &str) -> PageText {
    let mut text = String::new();
    let mut last: Option<(&str, &str, &str)> = None;
    let mut confidence_sum = 0.0f32;
    let mut words = 0u32;

    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.splitn(12, '\t').collect();
        if cols.len() < 12 || cols[0] != "5" || cols[11].trim().is_empty() {
            continue;
        }

        let position = (cols[2], cols[3], cols[4]);
        match last {
            None => {}
            Some((block, par, _)) if block != position.0 || par != position.1 => {
                text.push_str("\n\n")
            }
            Some(prev) if prev != position => text.push('\n'),
            Some(_) => text.push(' '),
        }
        text.push_str(cols[11].trim());
        last = Some(position);

        if let Ok(conf) = cols[10].parse::<f32>() {
            if conf >= 0.0 {
                confidence_sum += conf;
                words += 1;
            }
        }
    }

    PageText {
        text,
        confidence: (words > 0).then(|| confidence_sum / words as f32 / 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio_core::models::PipelineSettings;
    use image::{GrayImage, Luma};

    fn ctx(languages: &str) -> ProviderContext {
        let mut settings = PipelineSettings::default();
        settings.ocr.languages = languages.to_string();
        ProviderContext {
            settings,
            http: reqwest::Client::new(),
        }
    }

    #[test]
    fn rejects_malformed_language_lists() {
        assert!(build(&ctx("deu+eng")).is_ok());
        assert!(build(&ctx("")).is_err());
        assert!(build(&ctx("--help")).is_err());
        assert!(build(&ctx("eng; rm")).is_err());
    }

    #[test]
    fn tsv_is_rebuilt_into_lines_and_paragraphs() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n\
5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t96.5\tHello\n\
5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t90\tworld\n\
5\t1\t1\t1\t2\t1\t0\t0\t10\t10\t94\tsecond\n\
5\t1\t2\t1\t1\t1\t0\t0\t10\t10\t98\tNew\n";
        let page = parse_tsv(tsv);
        assert_eq!(page.text, "Hello world\nsecond\n\nNew");
        let confidence = page.confidence.unwrap();
        assert!((confidence - 0.945).abs() < 0.001, "{confidence}");
    }

    #[test]
    fn empty_tsv_gives_empty_text_and_no_confidence() {
        let page = parse_tsv("level\tpage_num\n");
        assert!(page.text.is_empty());
        assert!(page.confidence.is_none());
    }

    const GLYPHS: [(char, [&str; 7]); 5] = [
        (
            'F',
            [
                "#####", "#....", "#....", "####.", "#....", "#....", "#....",
            ],
        ),
        (
            'O',
            [
                ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
            ],
        ),
        (
            'L',
            [
                "#....", "#....", "#....", "#....", "#....", "#....", "#####",
            ],
        ),
        (
            'I',
            [
                "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
            ],
        ),
        (
            ' ',
            [
                ".....", ".....", ".....", ".....", ".....", ".....", ".....",
            ],
        ),
    ];

    /// Render `text` with a blocky 5x7 font; only the letters F, O, L, I are available
    fn render(text: &str) -> Vec<u8> {
        const SCALE: u32 = 14;
        let (margin, glyph_w, glyph_h) = (60, 6 * SCALE, 7 * SCALE);
        let width = margin * 2 + text.len() as u32 * glyph_w;
        let mut img = GrayImage::from_pixel(width, margin * 2 + glyph_h, Luma([255]));
        for (i, ch) in text.chars().enumerate() {
            let rows = GLYPHS.iter().find(|(c, _)| *c == ch).expect("glyph").1;
            for (y, row) in rows.iter().enumerate() {
                for (x, cell) in row.chars().enumerate() {
                    if cell != '#' {
                        continue;
                    }
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            img.put_pixel(
                                margin + i as u32 * glyph_w + x as u32 * SCALE + dx,
                                margin + y as u32 * SCALE + dy,
                                Luma([0]),
                            );
                        }
                    }
                }
            }
        }
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        png
    }

    #[tokio::test]
    async fn recognises_text_in_a_generated_png() {
        if !crate::ai::pdf::tests::tool_available("tesseract") {
            eprintln!("skipping: tesseract not installed");
            return;
        }
        let ocr = build(&ctx("eng")).unwrap();
        ocr.health().await.unwrap();

        let out = ocr
            .extract(&render("FOLIO FILL"), "image/png")
            .await
            .unwrap();

        assert_eq!(out.engine, "tesseract");
        assert!(out.text.contains("FOLIO"), "recognised: {:?}", out.text);
        assert!(out.confidence.unwrap() > 0.5);
    }

    #[tokio::test]
    async fn health_reports_missing_language_data() {
        if !crate::ai::pdf::tests::tool_available("tesseract") {
            eprintln!("skipping: tesseract not installed");
            return;
        }
        let err = build(&ctx("eng+zzz")).unwrap().health().await.unwrap_err();
        assert!(err.to_string().contains("zzz"));
    }
}
