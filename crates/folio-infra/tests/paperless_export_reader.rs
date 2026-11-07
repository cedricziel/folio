use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeZone, Utc};
use folio_core::ports::{
    digest_original, ChecksumAlgorithm, PaperlessDocument, PaperlessFieldType, PaperlessFieldValue,
    PaperlessReadError, PaperlessReader, PaperlessRecord, PaperlessTag, PaperlessVersion,
};
use folio_infra::import::PaperlessFsReader;
use futures::{StreamExt, TryStreamExt};

const FIXTURES: [&str; 4] = ["v2_20", "v3_4", "v3_4_split", "v3_4_folder_prefix"];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/paperless")
        .join(name)
}

async fn scan_with(reader: &PaperlessFsReader) -> Vec<PaperlessRecord> {
    reader.scan().await.unwrap().try_collect().await.unwrap()
}

async fn scan(name: &str) -> Vec<PaperlessRecord> {
    scan_with(&PaperlessFsReader::open(&fixture(name)).unwrap()).await
}

fn documents(records: &[PaperlessRecord]) -> Vec<&PaperlessDocument> {
    records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Document(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn tags(records: &[PaperlessRecord]) -> Vec<&PaperlessTag> {
    records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Tag(t) => Some(t),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn reads_the_version_from_metadata() {
    let v2 = PaperlessFsReader::open(&fixture("v2_20"))
        .unwrap()
        .source_info()
        .await
        .unwrap();
    assert_eq!(v2.version, PaperlessVersion::parse("2.20.15").unwrap());
    assert!(v2.version.is_supported());

    let v3 = PaperlessFsReader::open(&fixture("v3_4"))
        .unwrap()
        .source_info()
        .await
        .unwrap();
    assert_eq!(v3.version.to_string(), "3.4.2");
}

#[tokio::test]
async fn v2_export_yields_typed_records() {
    let records = scan("v2_20").await;

    let names: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Correspondent(c) => Some(("correspondent", c.id, c.name.as_str())),
            PaperlessRecord::DocumentType(t) => Some(("type", t.id, t.name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        [
            ("correspondent", 1, "ACME Corp"),
            ("correspondent", 2, "City Utilities"),
            ("type", 1, "Invoice"),
            ("type", 2, "Contract"),
        ]
    );

    let tags = tags(&records);
    let tag = |id| tags.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tags.len(), 5);
    assert!(tag(1).is_inbox_tag);
    assert_eq!(tag(3).name, "Invoices");
    assert_eq!(tag(3).parent, Some(2));
    assert_eq!(tag(5).name, "Invoices");
    assert_eq!(tag(5).parent, Some(4));
    assert_eq!(tag(5).color, "#fb9a99");

    let fields: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::CustomField(f) => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(
        fields.iter().map(|f| f.data_type).collect::<Vec<_>>(),
        [
            PaperlessFieldType::String,
            PaperlessFieldType::LongText,
            PaperlessFieldType::Url,
            PaperlessFieldType::Date,
            PaperlessFieldType::Boolean,
            PaperlessFieldType::Integer,
            PaperlessFieldType::Float,
            PaperlessFieldType::Monetary,
            PaperlessFieldType::Select,
            PaperlessFieldType::DocumentLink,
        ]
    );
    let amount = fields.iter().find(|f| f.name == "Amount").unwrap();
    assert_eq!(amount.default_currency.as_deref(), Some("EUR"));
    let priority = fields.iter().find(|f| f.name == "Priority").unwrap();
    assert_eq!(
        priority
            .select_options
            .iter()
            .map(|o| (o.id.as_deref(), o.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (Some("hJ3kQ9pLmN2x"), "High"),
            (Some("aB4cD5eF6gH7"), "Low")
        ]
    );

    let values: BTreeMap<_, _> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::CustomFieldInstance(i) if i.document == 1 => {
                let field = fields.iter().find(|f| f.id == i.field).unwrap();
                Some((
                    field.name.as_str(),
                    i.values.value(field.data_type).unwrap(),
                ))
            }
            _ => None,
        })
        .collect();
    assert_eq!(values.len(), 10);
    assert_eq!(
        values["Amount"],
        PaperlessFieldValue::Monetary("EUR123.45".into())
    );
    assert_eq!(values["Rating"], PaperlessFieldValue::Float(4.5));
    assert_eq!(
        values["Priority"],
        PaperlessFieldValue::Select("hJ3kQ9pLmN2x".into())
    );
    assert_eq!(
        values["Related"],
        PaperlessFieldValue::DocumentLink(vec![2, 99])
    );
    assert_eq!(
        values["Due date"],
        PaperlessFieldValue::Date(NaiveDate::from_ymd_opt(2024, 2, 15).unwrap())
    );

    let notes: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Note(n) => Some(n),
            _ => None,
        })
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].document, 1);
    assert_eq!(notes[0].body, "Paid by bank transfer on 2024-02-10.");
    assert!(!notes[0].deleted);
    assert_eq!(
        notes[0].created,
        Utc.with_ymd_and_hms(2024, 2, 10, 18, 22, 1).unwrap() + chrono::Duration::milliseconds(500)
    );

    let docs = documents(&records);
    assert_eq!(docs.len(), 2);
    let invoice = docs[0];
    assert_eq!(invoice.id, 1);
    assert_eq!(invoice.title, "Invoice 42");
    assert_eq!(
        invoice.created,
        NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
    );
    assert_eq!(
        invoice.added,
        Utc.with_ymd_and_hms(2024, 1, 15, 9, 15, 0).unwrap() + chrono::Duration::milliseconds(250)
    );
    assert_eq!(invoice.correspondent, Some(1));
    assert_eq!(invoice.document_type, Some(1));
    assert_eq!(invoice.tags, [2, 3]);
    assert_eq!(invoice.archive_serial_number, Some(1042));
    assert_eq!(invoice.mime_type, "application/pdf");
    assert_eq!(invoice.original_filename.as_deref(), Some("0000001.pdf"));
    assert_eq!(invoice.checksum.algorithm, ChecksumAlgorithm::Md5);
    assert_eq!(invoice.size, Some(28));
    assert!(invoice.content.starts_with("ACME Corp"));
    assert!(!invoice.trashed);
    assert_eq!(invoice.version_of, None);
    assert_eq!(docs[1].tags, [1, 4, 5]);
}

#[tokio::test]
async fn counts_skipped_models_after_a_scan() {
    let reader = PaperlessFsReader::open(&fixture("v2_20")).unwrap();
    assert!(reader.skipped_models().is_empty());
    scan_with(&reader).await;

    let expected: BTreeMap<String, u64> = [
        ("auth.group", 1),
        ("auth.permission", 2),
        ("auth.user", 2),
        ("contenttypes.contenttype", 1),
        ("documents.storagepath", 1),
        ("documents.uisettings", 1),
        ("documents.workflow", 1),
        ("documents.workflowaction", 1),
        ("documents.workflowtrigger", 1),
        ("guardian.userobjectpermission", 1),
        ("mfa.authenticator", 1),
        ("paperless.applicationconfiguration", 1),
        ("paperless_mail.mailaccount", 1),
        ("paperless_mail.mailrule", 1),
        ("socialaccount.socialaccount", 1),
        ("socialaccount.socialtoken", 1),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    assert_eq!(reader.skipped_models(), expected);

    let v3 = PaperlessFsReader::open(&fixture("v3_4")).unwrap();
    scan_with(&v3).await;
    assert_eq!(v3.skipped_models()["documents.sharelink"], 1);
}

#[tokio::test]
async fn secrets_never_reach_a_record() {
    for name in FIXTURES {
        let dump = format!("{:?}", scan(name).await);
        assert!(!dump.contains("FIXTURE-SECRET"), "{name} leaked a secret");
        assert!(
            !dump.contains("pbkdf2_sha256"),
            "{name} leaked a password hash"
        );
    }
}

#[tokio::test]
async fn v3_export_carries_trash_deleted_notes_and_versions() {
    let records = scan("v3_4").await;
    let docs = documents(&records);
    assert_eq!(
        docs.iter().map(|d| d.id).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );

    let doc = |id| *docs.iter().find(|d| d.id == id).unwrap();
    assert!(doc(3).trashed);
    assert!(!doc(1).trashed);
    assert_eq!(doc(4).version_of, None);
    let version = doc(5).version_of.as_ref().unwrap();
    assert_eq!(version.root_document, 4);
    assert_eq!(version.index, Some(1));
    assert_eq!(version.label.as_deref(), Some("signed"));

    assert_eq!(doc(1).checksum.algorithm, ChecksumAlgorithm::Sha256);
    assert_eq!(doc(4).checksum.algorithm, ChecksumAlgorithm::Md5);

    let deleted: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            PaperlessRecord::Note(n) => Some((n.id, n.deleted)),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, [(1, false), (2, true), (3, false)]);
}

#[tokio::test]
async fn modified_since_returns_everything_from_an_export() {
    let reader = PaperlessFsReader::open(&fixture("v3_4")).unwrap();
    let since = Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap();
    let records: Vec<_> = reader
        .modified_since(since)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(documents(&records).len(), 5);
}

#[tokio::test]
async fn lists_every_document_id() {
    let reader = PaperlessFsReader::open(&fixture("v3_4_split")).unwrap();
    let ids = reader.list_document_ids().await.unwrap();
    assert_eq!(
        ids.ids().iter().copied().collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
}

fn canonical(records: Vec<PaperlessRecord>) -> Vec<String> {
    let mut out: Vec<String> = records
        .into_iter()
        .map(|mut r| {
            if let PaperlessRecord::Document(d) = &mut r {
                d.original_handle = d
                    .original_handle
                    .trim_start_matches("originals/")
                    .to_string();
            }
            serde_json::to_string(&r).unwrap()
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn split_and_folder_prefix_layouts_yield_the_same_records() {
    let single = canonical(scan("v3_4").await);
    assert_eq!(canonical(scan("v3_4_split").await), single);
    assert_eq!(canonical(scan("v3_4_folder_prefix").await), single);
}

#[tokio::test]
async fn originals_match_their_checksums() {
    for name in FIXTURES {
        let reader = PaperlessFsReader::open(&fixture(name)).unwrap();
        for doc in documents(&scan_with(&reader).await) {
            let original = reader.open_original(doc).await.unwrap();
            let size = original.size;
            let digests = digest_original(original.reader, doc.checksum.algorithm)
                .await
                .unwrap();
            assert_eq!(digests.source, doc.checksum.value, "{name}: doc {}", doc.id);
            assert_eq!(size, Some(digests.size));
            assert_eq!(doc.size, Some(digests.size));
        }
    }
}

#[tokio::test]
async fn missing_and_malformed_files_are_typed_errors() {
    let dir = tempfile::tempdir().unwrap();
    let reader = PaperlessFsReader::open(dir.path()).unwrap();
    assert!(matches!(
        reader.source_info().await,
        Err(PaperlessReadError::Missing { .. })
    ));
    assert!(matches!(
        reader.scan().await.err(),
        Some(PaperlessReadError::Missing { .. })
    ));

    std::fs::write(dir.path().join("metadata.json"), "{\"version\": 3}").unwrap();
    assert!(matches!(
        reader.source_info().await,
        Err(PaperlessReadError::Malformed { .. })
    ));

    std::fs::write(
        dir.path().join("manifest.json"),
        "[{\"model\": \"documents.tag\", \"pk\": 1, ",
    )
    .unwrap();
    let results: Vec<_> = reader.scan().await.unwrap().collect::<Vec<_>>().await;
    assert!(matches!(
        results.last(),
        Some(Err(PaperlessReadError::Malformed { .. }))
    ));

    std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
    let results: Vec<_> = reader.scan().await.unwrap().collect::<Vec<_>>().await;
    assert!(matches!(
        results.as_slice(),
        [Err(PaperlessReadError::Malformed { .. })]
    ));
}

#[test]
fn open_fails_for_a_missing_directory() {
    assert!(matches!(
        PaperlessFsReader::open(Path::new("/definitely/not/here")).err(),
        Some(PaperlessReadError::Missing { .. })
    ));
}
