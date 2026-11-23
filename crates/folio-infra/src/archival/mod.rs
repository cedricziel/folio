//! Generate a document-level PDF/A-2b copy without modifying the source.
//! Only veraPDF can authorize returning a copy for publication.

mod pdf;
mod png;
mod validation;

use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::ai::process;

pub const PROFILE: &str = "PDF/A-2b";
pub(crate) const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SCRATCH_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCRATCH_ENTRIES: u64 = 10_000;
const MAX_TOOL_OUTPUT: u64 = 8 * 1024 * 1024;
const MAX_PAGES: usize = 500;

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedArchive {
    pub bytes: Vec<u8>,
    pub profile: &'static str,
    /// Absent when the source bytes passed validation and were reused.
    pub ocrmypdf_version: Option<String>,
    pub verapdf_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Qpdf,
    PdfToText,
    PdfImages,
    OcrMyPdf,
    VeraPdf,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkipReason {
    #[error("unsupported source media type")]
    UnsupportedMediaType,
    #[error("document exceeds the conversion limits")]
    ResourceLimit,
    #[error("source PDF could not be inspected safely")]
    InvalidPdf,
    #[error("source PDF is encrypted")]
    EncryptedPdf,
    #[error("source PDF contains a signature")]
    SignedPdf,
    #[error("source PDF contains an attachment")]
    EmbeddedAttachment,
    #[error("{0:?} is unavailable")]
    ToolUnavailable(Tool),
    #[error("{0:?} exceeded its time limit")]
    ToolTimeout(Tool),
    #[error("{0:?} failed")]
    ToolFailed(Tool),
    #[error("veraPDF returned an invalid or incomplete validation report")]
    InvalidValidationReport,
    #[error("PDF/A-2b validation rejected the converted copy")]
    NotConformant,
    #[error("converter did not produce a usable PDF")]
    InvalidOutput,
    #[error("could not access temporary conversion storage")]
    TemporaryStorage,
}

#[derive(Debug, Clone)]
pub struct ArchiveConverter {
    languages: String,
    tools: ToolPaths,
}

#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub qpdf: PathBuf,
    pub pdftotext: PathBuf,
    pub pdfimages: PathBuf,
    pub ocrmypdf: PathBuf,
    pub verapdf: PathBuf,
}

impl Default for ToolPaths {
    fn default() -> Self {
        Self {
            qpdf: "qpdf".into(),
            pdftotext: "pdftotext".into(),
            pdfimages: "pdfimages".into(),
            ocrmypdf: "ocrmypdf".into(),
            verapdf: "verapdf".into(),
        }
    }
}

impl ArchiveConverter {
    pub fn new(languages: impl Into<String>) -> Self {
        Self::with_tools(languages, ToolPaths::default())
    }

    pub fn with_tools(languages: impl Into<String>, tools: ToolPaths) -> Self {
        Self {
            languages: languages.into(),
            tools,
        }
    }

    pub async fn convert(&self, source: &[u8], mime: &str) -> Result<ValidatedArchive, SkipReason> {
        let input_name = match mime.split(';').next().unwrap_or_default().trim() {
            mime if mime.eq_ignore_ascii_case("application/pdf") => "source.pdf",
            mime if mime.eq_ignore_ascii_case("image/jpeg") => "source.jpg",
            mime if mime.eq_ignore_ascii_case("image/png") => "source.png",
            mime if mime.eq_ignore_ascii_case("image/tiff") => "source.tiff",
            _ => return Err(SkipReason::UnsupportedMediaType),
        };
        if source.len() as u64 > MAX_FILE_BYTES {
            return Err(SkipReason::ResourceLimit);
        }

        if input_name != "source.pdf" && !unambiguous_image(input_name, source) {
            return Err(SkipReason::UnsupportedMediaType);
        }

        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .map_err(|_| SkipReason::TemporaryStorage)?;
        let work = dir
            .path()
            .canonicalize()
            .map_err(|_| SkipReason::TemporaryStorage)?;
        let input = work.join(input_name);
        let output = work.join("archive.pdf");
        tokio::fs::write(&input, source)
            .await
            .map_err(|_| SkipReason::TemporaryStorage)?;

        if input_name == "source.png" {
            let png_path = input.clone();
            tokio::task::spawn_blocking(move || png::flatten_alpha(&png_path))
                .await
                .map_err(|_| SkipReason::TemporaryStorage)??;
        }

        let (source_pages, raster_pages) = if input_name == "source.pdf" {
            let pages = self.inspect_pdf(&work, &input).await?;
            let raster_pages = self.raster_pages(&work, &input, pages).await?;
            let validation = self.validate(&work, &input, source.len() as u64).await?;
            if let Some(version) = validation.filter(|_| raster_pages.is_empty()) {
                if self.has_text_on_each_page(&work, &input, pages).await {
                    return Ok(ValidatedArchive {
                        bytes: source.to_vec(),
                        profile: PROFILE,
                        ocrmypdf_version: None,
                        verapdf_version: version,
                    });
                }
            }
            (Some(pages), raster_pages)
        } else {
            (None, Default::default())
        };

        let version = self
            .run(Tool::OcrMyPdf, &work, ["--version"], process::INFO_TIMEOUT)
            .await?;
        let version = String::from_utf8_lossy(&version.stdout);
        let version = version.trim();
        if version.is_empty() || version.len() > 128 || version.chars().any(char::is_control) {
            return Err(SkipReason::ToolFailed(Tool::OcrMyPdf));
        }

        let mut command = self.command(Tool::OcrMyPdf, &work);
        command.args([
            "--output-type",
            "pdfa-2",
            "--optimize",
            "0",
            "--pdfa-image-compression",
            "lossless",
            "--jobs",
            "1",
            "--tesseract-timeout",
            "300",
            "--language",
            &self.languages,
        ]);
        if raster_pages.is_empty() {
            command.arg("--skip-text");
        } else {
            let pages = raster_pages
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(",");
            // Whole-document redo can duplicate existing text on image-free pages.
            command.args(["--redo-ocr", "--pages", &pages]);
        }
        if source_pages.is_none() {
            command.args(["--image-dpi", "300"]);
        }
        let converted = tokio::select! {
            output = run_command(
                Tool::OcrMyPdf,
                command.arg(&input).arg(&output),
                process::TOOL_TIMEOUT,
            ) => output?,
            error = monitor_scratch(&work) => return Err(error),
        };
        check_scratch(&work).await?;
        if !converted.status.success() {
            return Err(SkipReason::ToolFailed(Tool::OcrMyPdf));
        }

        let metadata = tokio::fs::metadata(&output)
            .await
            .map_err(|_| SkipReason::InvalidOutput)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(SkipReason::InvalidOutput);
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(SkipReason::ResourceLimit);
        }
        let pages = self.inspect_pdf(&work, &output).await?;
        if source_pages.is_some_and(|expected| expected != pages) {
            return Err(SkipReason::InvalidOutput);
        }
        let verapdf_version = self
            .validate(&work, &output, metadata.len())
            .await?
            .ok_or(SkipReason::NotConformant)?;

        let mut bytes = Vec::new();
        tokio::fs::File::open(&output)
            .await
            .map_err(|_| SkipReason::TemporaryStorage)?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| SkipReason::TemporaryStorage)?;
        if bytes.len() as u64 != metadata.len() {
            return Err(SkipReason::InvalidOutput);
        }
        Ok(ValidatedArchive {
            bytes,
            profile: PROFILE,
            ocrmypdf_version: Some(version.to_owned()),
            verapdf_version,
        })
    }

    fn command(&self, tool: Tool, work: &Path) -> Command {
        let path = match tool {
            Tool::Qpdf => &self.tools.qpdf,
            Tool::PdfToText => &self.tools.pdftotext,
            Tool::PdfImages => &self.tools.pdfimages,
            Tool::OcrMyPdf => &self.tools.ocrmypdf,
            Tool::VeraPdf => &self.tools.verapdf,
        };
        let mut command = Command::new(path);
        command
            .current_dir(work)
            .env("TMPDIR", work)
            .env("OMP_THREAD_LIMIT", "1");
        if tool == Tool::VeraPdf {
            command.env(
                "JAVA_TOOL_OPTIONS",
                format!("-Xmx512m -Djava.io.tmpdir=\"{}\"", work.display()),
            );
        }
        // SAFETY: the closure uses only async-signal-safe resource-limit calls.
        // Ghostscript, Tesseract and other descendants inherit the capped limits.
        unsafe {
            command.pre_exec(|| {
                let mut bounds = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(libc::RLIMIT_FSIZE, &mut bounds) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let cap = MAX_FILE_BYTES as libc::rlim_t;
                bounds.rlim_cur = bounds.rlim_cur.min(cap);
                bounds.rlim_max = bounds.rlim_max.min(cap);
                if libc::setrlimit(libc::RLIMIT_FSIZE, &bounds) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command
    }

    async fn run(
        &self,
        tool: Tool,
        work: &Path,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
        timeout: Duration,
    ) -> Result<Output, SkipReason> {
        let output = run_command(tool, self.command(tool, work).args(args), timeout).await?;
        if !output.status.success() {
            return Err(SkipReason::ToolFailed(tool));
        }
        Ok(output)
    }
}

async fn monitor_scratch(work: &Path) -> SkipReason {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Err(error) = check_scratch(work).await {
            return error;
        }
    }
}

async fn check_scratch(work: &Path) -> Result<(), SkipReason> {
    let mut directories = vec![work.to_path_buf()];
    let mut bytes = 0u64;
    let mut count = 0;
    while let Some(directory) = directories.pop() {
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            result => result.map_err(|_| SkipReason::TemporaryStorage)?,
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|_| SkipReason::TemporaryStorage)?
        {
            let metadata = match entry.metadata().await {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                result => result.map_err(|_| SkipReason::TemporaryStorage)?,
            };
            count += 1;
            bytes = bytes.saturating_add(metadata.len());
            if count > MAX_SCRATCH_ENTRIES || bytes > MAX_SCRATCH_BYTES {
                return Err(SkipReason::ResourceLimit);
            }
            if metadata.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

async fn run_command(
    tool: Tool,
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, SkipReason> {
    process::bounded_output(command, timeout, MAX_TOOL_OUTPUT)
        .await
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => SkipReason::ToolUnavailable(tool),
            io::ErrorKind::TimedOut => SkipReason::ToolTimeout(tool),
            io::ErrorKind::FileTooLarge => SkipReason::ResourceLimit,
            _ => SkipReason::ToolFailed(tool),
        })
}

#[cfg(test)]
mod tests;

fn unambiguous_image(input_name: &str, source: &[u8]) -> bool {
    // OCRmyPDF looks for a PDF header anywhere in the first 1024 bytes,
    // so an image prefix alone is insufficient to identify the input path.
    if source[..source.len().min(1024)]
        .windows(5)
        .any(|window| window == b"%PDF-")
    {
        return false;
    }
    match input_name {
        "source.jpg" => source.starts_with(b"\xff\xd8\xff"),
        "source.png" => source.starts_with(b"\x89PNG\r\n\x1a\n"),
        "source.tiff" => source.starts_with(b"II*\0") || source.starts_with(b"MM\0*"),
        _ => false,
    }
}
