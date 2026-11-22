mod common;

use folio_core::models::{CreateDocument, DocumentFilter};
use folio_infra::{ai::pipeline::PipelineContext, config::AiConfig};
use std::process::Command;

#[tokio::test]
async fn mixed_pdf_keeps_digital_pages_and_ocr_text_searchable() {
    for tool in ["pdftotext", "pdftoppm", "tesseract"] {
        if Command::new(tool).arg("--version").output().is_err() {
            assert!(std::env::var_os("CI").is_none(), "CI must install {tool}");
            eprintln!("skipping: {tool} not installed");
            return;
        }
    }

    let (_dir, db) = common::sqlite_database().await;
    let repos = db.repositories().await;
    repos
        .settings
        .set("ocr.provider", "tesseract")
        .await
        .unwrap();
    repos.settings.set("ocr.languages", "eng").await.unwrap();

    let pipeline = PipelineContext::new(repos.settings.clone(), AiConfig::default());
    let settings = pipeline.settings().await;
    let ocr = pipeline.build_ocr(&settings).unwrap();
    let bytes = include_bytes!("fixtures/mixed-digital-scan.pdf");
    let output = ocr.extract(bytes, "application/pdf").await.unwrap();

    assert_eq!(
        output
            .pages
            .iter()
            .map(|page| page.trim())
            .collect::<Vec<_>>(),
        [
            "Page 1 digital document reference information.",
            "Page 2 digital document reference information.",
            "Cobalt Orchard 9716",
            "Page 4 digital document reference information.",
            "Page 5 digital document reference information.",
        ]
    );

    let vault = repos.vaults.create("Mixed PDF OCR", None).await.unwrap().id;
    let document = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Mixed digital and scanned PDF".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    repos
        .documents
        .complete_ocr(vault, document.id, &output.text)
        .await
        .unwrap();

    let matches = repos
        .documents
        .list(
            vault,
            Some(DocumentFilter {
                search_query: Some("Cobalt Orchard 9716".into()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        matches
            .iter()
            .map(|document| document.id)
            .collect::<Vec<_>>(),
        vec![document.id]
    );
}
