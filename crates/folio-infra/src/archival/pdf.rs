use std::path::Path;

use serde_json::Value;

use super::{run_command, ArchiveConverter, SkipReason, Tool, MAX_PAGES};
use crate::ai::process::INFO_TIMEOUT;

impl ArchiveConverter {
    pub(super) async fn inspect_pdf(&self, work: &Path, input: &Path) -> Result<usize, SkipReason> {
        let encrypted = run_command(
            Tool::Qpdf,
            self.command(Tool::Qpdf, work)
                .arg("--is-encrypted")
                .arg(input),
            INFO_TIMEOUT,
        )
        .await?;
        match encrypted.status.code() {
            Some(0) => return Err(SkipReason::EncryptedPdf),
            Some(2) => {}
            _ => return Err(SkipReason::InvalidPdf),
        }
        let output = self
            .run(
                Tool::Qpdf,
                work,
                [
                    "--json=2".as_ref(),
                    "--json-stream-data=none".as_ref(),
                    "--json-key=qpdf".as_ref(),
                    "--json-key=pages".as_ref(),
                    input.as_os_str(),
                ],
                INFO_TIMEOUT,
            )
            .await
            .map_err(|error| match error {
                SkipReason::ToolFailed(Tool::Qpdf) => SkipReason::InvalidPdf,
                other => other,
            })?;
        inspect_structure(&output.stdout)
    }

    pub(super) async fn has_images(&self, work: &Path, input: &Path) -> Result<bool, SkipReason> {
        let output = run_command(
            Tool::PdfImages,
            self.command(Tool::PdfImages, work)
                .env("LC_ALL", "C")
                .arg("-list")
                .arg(input),
            INFO_TIMEOUT,
        )
        .await?;
        if !output.status.success() {
            return Err(SkipReason::ToolFailed(Tool::PdfImages));
        }
        let list = std::str::from_utf8(&output.stdout)
            .map_err(|_| SkipReason::ToolFailed(Tool::PdfImages))?;
        let mut lines = list.lines();
        if lines.next().and_then(|line| line.split_whitespace().next()) != Some("page")
            || !lines.next().is_some_and(|line| line.starts_with("---"))
        {
            return Err(SkipReason::ToolFailed(Tool::PdfImages));
        }
        Ok(lines.next().is_some())
    }

    pub(super) async fn has_text_on_each_page(
        &self,
        work: &Path,
        input: &Path,
        pages: usize,
    ) -> bool {
        self.run(
            Tool::PdfToText,
            work,
            [
                "-enc".as_ref(),
                "UTF-8".as_ref(),
                input.as_os_str(),
                "-".as_ref(),
            ],
            INFO_TIMEOUT,
        )
        .await
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|text| {
            let page_text = text.split_terminator('\x0c');
            page_text.clone().count() == pages
                && page_text
                    .into_iter()
                    .all(|page| page.chars().any(char::is_alphanumeric))
        })
    }
}

pub(super) fn inspect_structure(json: &[u8]) -> Result<usize, SkipReason> {
    let pdf: Value = serde_json::from_slice(json).map_err(|_| SkipReason::InvalidPdf)?;
    let (metadata, objects) = pdf
        .get("qpdf")
        .and_then(Value::as_array)
        .and_then(|entries| match entries.as_slice() {
            [metadata, objects] => Some((metadata, objects.as_object()?)),
            _ => None,
        })
        .ok_or(SkipReason::InvalidPdf)?;
    if metadata["jsonversion"] != 2 {
        return Err(SkipReason::InvalidPdf);
    }
    let trailer = objects
        .get("trailer")
        .and_then(|trailer| trailer["value"].as_object())
        .ok_or(SkipReason::InvalidPdf)?;
    if trailer.contains_key("/Encrypt") {
        return Err(SkipReason::EncryptedPdf);
    }

    // Walk actual PDF objects, including dictionaries in compressed object
    // streams. References are inspected via their own object table entries.
    check_objects(&pdf["qpdf"][1])?;
    let pages = pdf["pages"].as_array().ok_or(SkipReason::InvalidPdf)?.len();
    match pages {
        0 => Err(SkipReason::InvalidPdf),
        n if n > MAX_PAGES => Err(SkipReason::ResourceLimit),
        n => Ok(n),
    }
}

fn check_objects(object: &Value) -> Result<(), SkipReason> {
    match object {
        Value::Object(dictionary) => {
            if ["/ByteRange", "/DocMDP", "/UR3"]
                .iter()
                .any(|key| dictionary.contains_key(*key))
            {
                return Err(SkipReason::SignedPdf);
            }
            if ["/EmbeddedFiles", "/EF", "/AF", "/RF"]
                .iter()
                .any(|key| dictionary.contains_key(*key))
            {
                return Err(SkipReason::EmbeddedAttachment);
            }
            dictionary.values().try_for_each(check_objects)
        }
        Value::Array(items) => items.iter().try_for_each(check_objects),
        // Qpdf JSON v2 encodes PDF strings with a u: or b: prefix.
        // These bare names also catch names stored as indirect objects.
        Value::String(name) => match name.as_str() {
            "/Sig" | "/DocTimeStamp" => Err(SkipReason::SignedPdf),
            "/EmbeddedFile" | "/FileAttachment" | "/RichMedia" | "/Collection" => {
                Err(SkipReason::EmbeddedAttachment)
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}
