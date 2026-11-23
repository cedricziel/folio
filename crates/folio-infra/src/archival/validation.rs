use std::path::Path;
use std::process::ExitStatus;

use roxmltree::{Document, Node};

use super::{run_command, ArchiveConverter, SkipReason, Tool};
use crate::ai::process::TOOL_TIMEOUT;

impl ArchiveConverter {
    /// A successful nonconformance report permits conversion of the source.
    /// An unusable report or a validator failure aborts the attempt.
    pub(super) async fn validate(
        &self,
        work: &Path,
        input: &Path,
        size: u64,
    ) -> Result<Option<String>, SkipReason> {
        let output = run_command(
            Tool::VeraPdf,
            self.command(Tool::VeraPdf, work)
                .args([
                    "--format",
                    "xml",
                    "--flavour",
                    "2b",
                    "--maxfailuresdisplayed",
                    "1",
                ])
                .arg(input),
            TOOL_TIMEOUT,
        )
        .await?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            return Err(SkipReason::ToolFailed(Tool::VeraPdf));
        }
        parse_report(output.status, &output.stdout, size)
    }
}

pub(super) fn parse_report(
    status: ExitStatus,
    bytes: &[u8],
    size: u64,
) -> Result<Option<String>, SkipReason> {
    let invalid = SkipReason::InvalidValidationReport;
    let xml = std::str::from_utf8(bytes).map_err(|_| SkipReason::InvalidValidationReport)?;
    let doc = Document::parse(xml).map_err(|_| SkipReason::InvalidValidationReport)?;
    let root = doc.root_element();
    if !root.has_tag_name("report") {
        return Err(invalid);
    }
    let batch = only_child(root, "batchSummary").ok_or(SkipReason::InvalidValidationReport)?;
    if !has_attributes(
        batch,
        &[
            ("totalJobs", "1"),
            ("failedToParse", "0"),
            ("encrypted", "0"),
            ("outOfMemory", "0"),
            ("veraExceptions", "0"),
        ],
    ) {
        return Err(invalid);
    }
    let totals =
        only_child(batch, "validationReports").ok_or(SkipReason::InvalidValidationReport)?;
    let job = only_child(root, "jobs")
        .and_then(|jobs| only_child(jobs, "job"))
        .ok_or(SkipReason::InvalidValidationReport)?;
    let item_size = only_child(job, "item")
        .and_then(|item| item.attribute("size"))
        .and_then(|size| size.parse::<u64>().ok());
    let result = only_child(job, "validationReport").ok_or(SkipReason::InvalidValidationReport)?;
    if item_size != Some(size)
        || result.attribute("profileName") != Some("PDF/A-2b validation profile")
        || result
            .attribute("jobEndStatus")
            .is_some_and(|status| status != "normal")
    {
        return Err(invalid);
    }

    match result.attribute("isCompliant") {
        Some("false")
            if has_attributes(
                totals,
                &[
                    ("compliant", "0"),
                    ("nonCompliant", "1"),
                    ("failedJobs", "0"),
                ],
            ) && matches!(status.code(), Some(0 | 1)) =>
        {
            Ok(None)
        }
        Some("true")
            if has_attributes(
                totals,
                &[
                    ("compliant", "1"),
                    ("nonCompliant", "0"),
                    ("failedJobs", "0"),
                ],
            ) && status.success() =>
        {
            let build =
                only_child(root, "buildInformation").ok_or(SkipReason::InvalidValidationReport)?;
            let version = build
                .children()
                .find(|node| {
                    node.has_tag_name("releaseDetails") && node.attribute("id") == Some("core")
                })
                .and_then(|release| release.attribute("version"))
                .filter(|version| {
                    !version.is_empty()
                        && version.len() <= 128
                        && version
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
                })
                .ok_or(SkipReason::InvalidValidationReport)?;
            Ok(Some(version.to_owned()))
        }
        _ => Err(invalid),
    }
}

fn only_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    let mut children = node.children().filter(|child| child.has_tag_name(name));
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

fn has_attributes(node: Node<'_, '_>, attributes: &[(&str, &str)]) -> bool {
    attributes
        .iter()
        .all(|(name, value)| node.attribute(*name) == Some(*value))
}
