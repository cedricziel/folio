use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command as StdCommand, ExitStatus};

use super::*;
use crate::ai::pdf::tests::tiny_pdf;

const MIXED: &[u8] = include_bytes!("../../tests/fixtures/mixed-digital-scan.pdf");

fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn fake_report(size: u64, profile: &str, compliant: bool) -> String {
    let passed = usize::from(compliant);
    let failed = usize::from(!compliant);
    format!(
        r#"<report>
  <buildInformation><releaseDetails id="core" version="1.30.4"/></buildInformation>
  <jobs><job><item size="{size}"/><validationReport
    profileName="{profile} validation profile" isCompliant="{compliant}"/></job></jobs>
  <batchSummary totalJobs="1" failedToParse="0" encrypted="0" outOfMemory="0" veraExceptions="0">
    <validationReports compliant="{passed}" nonCompliant="{failed}" failedJobs="0">1</validationReports>
  </batchSummary>
</report>"#
    )
}

#[test]
fn validation_requires_an_explicit_consistent_report_for_this_profile_and_size() {
    let status = ExitStatus::from_raw(0);
    let report = fake_report(120, PROFILE, true);
    assert_eq!(
        validation::parse_report(status, report.as_bytes(), 120),
        Ok(Some("1.30.4".into()))
    );
    for report in [
        "<report/>".to_owned(),
        fake_report(120, "PDF/A-3b", true),
        fake_report(121, PROFILE, true),
        report.replace("failedToParse=\"0\"", "failedToParse=\"1\""),
        report.replace("failedJobs=\"0\"", "failedJobs=\"1\""),
        format!("{report}<report/>"),
    ] {
        assert_eq!(
            validation::parse_report(status, report.as_bytes(), 120),
            Err(SkipReason::InvalidValidationReport)
        );
    }
    assert_eq!(
        validation::parse_report(ExitStatus::from_raw(1 << 8), report.as_bytes(), 120),
        Err(SkipReason::InvalidValidationReport)
    );
}

#[test]
fn structural_detection_distinguishes_pdf_names_from_document_text() {
    let inspect = |object: serde_json::Value| {
        pdf::inspect_structure(
            &serde_json::to_vec(&serde_json::json!({
                "qpdf": [{"jsonversion": 2}, {
                    "trailer": {"value": {"/Root": "1 0 R"}},
                    "obj:1 0 R": object
                }],
                "pages": [{}]
            }))
            .unwrap(),
        )
    };
    assert_eq!(
        inspect(serde_json::json!({"value":{" /Note": "u:/Sig /EmbeddedFiles /Encrypt"}})),
        Ok(1)
    );
    assert_eq!(
        inspect(serde_json::json!({"value":{"/FT":"/Sig"}})),
        Err(SkipReason::SignedPdf)
    );
    assert_eq!(
        inspect(serde_json::json!({"stream":{"dict":{"/Type":"/EmbeddedFile"}}})),
        Err(SkipReason::EmbeddedAttachment)
    );
}

#[tokio::test]
async fn refuses_missing_or_failed_validation_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ToolPaths {
        qpdf: executable(
            dir.path(),
            "qpdf",
            r#"if [ "$1" = "--is-encrypted" ]; then exit 2; fi
echo '{"qpdf":[{"jsonversion":2},{"trailer":{"value":{}}}],"pages":[{}]}'"#,
        ),
        ocrmypdf: executable(
            dir.path(),
            "ocrmypdf",
            r#"if [ "$1" = "--version" ]; then echo 14.0.0; exit; fi
for output do :; done
printf %%PDF-fake > "$output"
printf %s "$TMPDIR" > "$(dirname "$0")/scratch""#,
        ),
        verapdf: dir.path().join("missing"),
        ..ToolPaths::default()
    };
    let mut source = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(1, 1)
        .write_to(&mut source, image::ImageFormat::Png)
        .unwrap();
    let source = source.get_ref();
    let converter = ArchiveConverter::with_tools("eng", paths.clone());
    assert_eq!(
        converter.convert(source, "image/png").await,
        Err(SkipReason::ToolUnavailable(Tool::VeraPdf))
    );

    let rejected = fake_report(9, PROFILE, false);
    for (script, expected) in [
        (
            format!("cat <<'REPORT'\n{rejected}\nREPORT\nexit 1"),
            SkipReason::NotConformant,
        ),
        (
            "echo '<report/>'; exit 0".into(),
            SkipReason::InvalidValidationReport,
        ),
        (
            "echo failure >&2; exit 2".into(),
            SkipReason::ToolFailed(Tool::VeraPdf),
        ),
    ] {
        let mut paths = paths.clone();
        paths.verapdf = executable(dir.path(), "verapdf", &script);
        let converter = ArchiveConverter::with_tools("eng", paths);
        assert_eq!(converter.convert(source, "image/png").await, Err(expected));
        let scratch = std::fs::read_to_string(dir.path().join("scratch")).unwrap();
        assert!(!Path::new(&scratch).exists());
    }
    for source in [b"%PDF-1.7".as_slice(), b"\x89PNG\r\n\x1a\n%PDF-1.7"] {
        assert_eq!(
            converter.convert(source, "image/png").await,
            Err(SkipReason::UnsupportedMediaType)
        );
    }
}

#[tokio::test]
async fn missing_converter_and_flooding_tool_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut converter = ArchiveConverter::with_tools(
        "eng",
        ToolPaths {
            ocrmypdf: dir.path().join("missing"),
            ..ToolPaths::default()
        },
    );
    let source = b"\xff\xd8\xff";
    assert_eq!(
        converter.convert(source, "image/jpeg").await,
        Err(SkipReason::ToolUnavailable(Tool::OcrMyPdf))
    );
    converter.tools.ocrmypdf = executable(dir.path(), "ocrmypdf", "exec yes");
    assert_eq!(
        converter.convert(source, "image/jpeg").await,
        Err(SkipReason::ResourceLimit)
    );
}

#[tokio::test]
async fn oversized_scratch_workspace_aborts_conversion_and_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let ocrmypdf = executable(
        dir.path(),
        "ocrmypdf",
        r#"if [ "$1" = "--version" ]; then echo 14.0.0; exit; fi
printf %s "$TMPDIR" > "$(dirname "$0")/scratch"
for i in 1 2 3 4 5; do
    dd if=/dev/zero of="scratch-$i" bs=1 count=1 seek=120000000 2>/dev/null
done
exec sleep 30"#,
    );
    let converter = ArchiveConverter::with_tools(
        "eng",
        ToolPaths {
            ocrmypdf,
            ..ToolPaths::default()
        },
    );
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        converter.convert(b"\xff\xd8\xff", "image/jpeg"),
    )
    .await
    .expect("scratch limit must stop the child promptly");
    assert_eq!(result, Err(SkipReason::ResourceLimit));
    let scratch = std::fs::read_to_string(dir.path().join("scratch")).unwrap();
    assert!(!Path::new(&scratch).exists());
}

fn real_tools_available(tools: &[&str]) -> bool {
    for tool in tools {
        if StdCommand::new(tool).arg("--version").output().is_err() {
            assert!(
                std::env::var_os("FOLIO_REQUIRE_ARCHIVAL_TOOLS").is_none(),
                "{tool} is required"
            );
            eprintln!("skipping archival integration test: {tool} is unavailable");
            return false;
        }
    }
    true
}

fn fixture_command(command: &mut StdCommand) -> Vec<u8> {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[tokio::test]
async fn real_archival_tools_keep_digital_text_and_ocr_a_scan() {
    if !real_tools_available(&[
        "qpdf",
        "pdftotext",
        "pdfimages",
        "pdftoppm",
        "ocrmypdf",
        "verapdf",
    ]) {
        return;
    }
    let converter = ArchiveConverter::new("eng");
    let archive = converter.convert(MIXED, "application/pdf").await.unwrap();
    assert_eq!(archive.profile, PROFILE);
    assert!(archive.ocrmypdf_version.is_some());
    assert!(!archive.verapdf_version.is_empty());
    let pages = crate::ai::pdf::text_layer(&archive.bytes).await.unwrap();
    assert_eq!(pages.len(), 5);
    assert!(pages[0].contains("Page 1 digital document reference information"));
    assert!(pages[2].contains("Cobalt Orchard 9716"), "{pages:?}");

    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("mixed.pdf");
    let archived = dir.path().join("archived.pdf");
    std::fs::write(&original, MIXED).unwrap();
    std::fs::write(&archived, &archive.bytes).unwrap();
    let images = fixture_command(
        StdCommand::new("pdfimages")
            .args(["-list", "-f", "1", "-l", "1"])
            .arg(&archived),
    );
    assert_eq!(String::from_utf8_lossy(&images).lines().count(), 2);

    let digital = converter
        .convert(&tiny_pdf("Searchable digital page"), "application/pdf")
        .await
        .unwrap();
    let reused = converter
        .convert(&digital.bytes, "application/pdf")
        .await
        .unwrap();
    assert_eq!(reused.bytes, digital.bytes);
    assert_eq!(reused.ocrmypdf_version, None);

    for (flag, ext, mime) in [
        ("-png", "png", "image/png"),
        ("-jpeg", "jpg", "image/jpeg"),
        ("-tiff", "tif", "image/tiff"),
    ] {
        let image = dir.path().join("scan");
        fixture_command(
            StdCommand::new("pdftoppm")
                .args(["-r", "150", "-f", "3", "-l", "3", "-singlefile", flag])
                .arg(&original)
                .arg(&image),
        );
        let bytes = std::fs::read(image.with_extension(ext)).unwrap();
        let archive = converter.convert(&bytes, mime).await.unwrap();
        let text = crate::ai::pdf::text_layer(&archive.bytes).await.unwrap();
        assert!(text[0].contains("Cobalt Orchard 9716"), "{mime}: {text:?}");
    }

    let mut transparent = image::open(dir.path().join("scan.png"))
        .unwrap()
        .into_rgba8();
    for pixel in transparent.pixels_mut() {
        if *pixel == image::Rgba([255, 255, 255, 255]) {
            *pixel = image::Rgba([0, 0, 0, 0]);
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    transparent
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let archive = converter
        .convert(bytes.get_ref(), "image/png")
        .await
        .unwrap();
    let text = crate::ai::pdf::text_layer(&archive.bytes).await.unwrap();
    assert!(text[0].contains("Cobalt Orchard 9716"), "{text:?}");
}

#[tokio::test]
async fn real_archival_tools_ocr_a_scan_that_shares_a_page_with_digital_text() {
    if !real_tools_available(&["qpdf", "pdftotext", "pdfimages", "ocrmypdf", "verapdf"]) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.pdf");
    let overlay = dir.path().join("overlay.pdf");
    let mixed = dir.path().join("mixed.pdf");
    let partly_searchable = dir.path().join("partly-searchable.pdf");
    std::fs::write(&original, MIXED).unwrap();
    std::fs::write(&overlay, tiny_pdf("Digital page header 2036")).unwrap();
    fixture_command(
        StdCommand::new("qpdf")
            .arg(&original)
            .arg("--overlay")
            .arg(&overlay)
            .args(["--to=3", "--"])
            .arg(&mixed),
    );
    fixture_command(
        StdCommand::new("ocrmypdf")
            .args([
                "--skip-text",
                "--output-type",
                "pdfa-2",
                "--optimize",
                "0",
                "--pdfa-image-compression",
                "lossless",
                "--jobs",
                "1",
            ])
            .arg(&mixed)
            .arg(&partly_searchable),
    );

    let source = std::fs::read(&partly_searchable).unwrap();
    let before = crate::ai::pdf::text_layer(&source).await.unwrap();
    assert!(before[2].contains("Digital page header 2036"), "{before:?}");
    assert!(!before[2].contains("Cobalt Orchard 9716"), "{before:?}");
    let converter = ArchiveConverter::new("eng");
    assert!(converter
        .validate(dir.path(), &partly_searchable, source.len() as u64)
        .await
        .unwrap()
        .is_some());

    let archive = converter.convert(&source, "application/pdf").await.unwrap();
    assert_eq!(archive.profile, PROFILE);
    assert!(archive.ocrmypdf_version.is_some());
    let after = crate::ai::pdf::text_layer(&archive.bytes).await.unwrap();
    assert_eq!(
        after[2].matches("Digital page header 2036").count(),
        1,
        "{after:?}"
    );
    assert!(after[2].contains("Cobalt Orchard 9716"), "{after:?}");
}

#[tokio::test]
async fn real_qpdf_detects_compressed_signature_and_attachment_and_password() {
    if !real_tools_available(&["qpdf"]) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    std::fs::write(&source, tiny_pdf("Archive fixture")).unwrap();
    let encrypted = dir.path().join("encrypted.pdf");
    fixture_command(
        StdCommand::new("qpdf")
            .arg(&source)
            .args(["--encrypt", "userpassword", "ownerpassword", "256", "--"])
            .arg(&encrypted),
    );

    let objects = fixture_command(
        StdCommand::new("qpdf")
            .args(["--json=2", "--json-key=qpdf"])
            .arg(&source),
    );
    let mut objects: serde_json::Value = serde_json::from_slice(&objects).unwrap();
    // Inline the attachment data so qpdf only needs to open the PDF and JSON fixture.
    let mut attachment_objects = objects.clone();
    let filespec = attachment_objects["qpdf"][0]["maxobjectid"]
        .as_u64()
        .unwrap()
        + 1;
    let embedded_file = filespec + 1;
    attachment_objects["qpdf"][1]["obj:1 0 R"]["value"]["/Names"] = serde_json::json!({
        "/EmbeddedFiles": {"/Names": ["u:attachment.txt", format!("{filespec} 0 R")]}
    });
    attachment_objects["qpdf"][1][format!("obj:{filespec} 0 R")] = serde_json::json!({
        "value": {
            "/Type": "/Filespec",
            "/F": "u:attachment.txt",
            "/EF": {"/F": format!("{embedded_file} 0 R")}
        }
    });
    attachment_objects["qpdf"][1][format!("obj:{embedded_file} 0 R")] = serde_json::json!({
        "stream": {
            "dict": {"/Type": "/EmbeddedFile"},
            "data": "Y29udGVudHM="
        }
    });
    let attached = dir.path().join("attached.pdf");
    compressed_pdf_with_json(&source, &attached, &attachment_objects);

    // A synthetic signature dictionary tests structural detection. Its
    // signature does not need to verify to prevent destructive conversion.
    objects["qpdf"][1]["obj:1 0 R"]["value"]["/Perms"] = serde_json::json!({
        "/DocMDP": {"/Type":"/Sig", "/ByteRange":[0,100,200,100], "/Contents":"b:00"}
    });
    let signed = dir.path().join("signed.pdf");
    compressed_pdf_with_json(&source, &signed, &objects);

    let converter = ArchiveConverter::new("eng");
    for (path, reason) in [
        (attached, SkipReason::EmbeddedAttachment),
        (encrypted, SkipReason::EncryptedPdf),
        (signed, SkipReason::SignedPdf),
    ] {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            converter.convert(&bytes, "application/pdf").await,
            Err(reason)
        );
    }
}

fn compressed_pdf_with_json(source: &Path, output: &Path, objects: &serde_json::Value) {
    let update = output.with_extension("json");
    std::fs::write(&update, serde_json::to_vec(objects).unwrap()).unwrap();
    fixture_command(
        StdCommand::new("qpdf")
            .arg(source)
            .arg(format!("--update-from-json={}", update.display()))
            .arg("--object-streams=generate")
            .arg(output),
    );
}
