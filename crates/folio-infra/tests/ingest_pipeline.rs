//! Pre-ingest rules, de-duplication, tags, and source-aware workflows on real SQLite.

mod common;

use common::ingest::{file_item, mail_item, Harness};
use folio_core::models::{
    IngestOutcome, LedgerEntry, SourceConditions, SourceKind, TextMatch, WorkflowAction,
    WorkflowConditions, WorkflowInput, WorkflowTrigger,
};
use folio_core::services::content_key;

struct GatedImporter {
    inner: std::sync::Arc<dyn folio_core::ports::DocumentImporter>,
    started: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
    first: std::sync::atomic::AtomicBool,
    fail_first: bool,
}

#[async_trait::async_trait]
impl folio_core::ports::DocumentImporter for GatedImporter {
    async fn import(
        &self,
        vault: folio_core::models::VaultId,
        upload: folio_core::services::NewUpload,
    ) -> folio_core::Result<folio_core::models::Document> {
        if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.started.notify_one();
            self.release.notified().await;
            if self.fail_first {
                return Err(folio_core::Error::Repository(
                    "synthetic import failure".into(),
                ));
            }
        }
        self.inner.import(vault, upload).await
    }
}

async fn gated_harness(
    fail_first: bool,
) -> (
    Harness,
    std::sync::Arc<tokio::sync::Notify>,
    std::sync::Arc<tokio::sync::Notify>,
) {
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let h = Harness::with_importer(|inner| {
        std::sync::Arc::new(GatedImporter {
            inner,
            started: started.clone(),
            release: release.clone(),
            first: std::sync::atomic::AtomicBool::new(true),
            fail_first,
        })
    })
    .await;
    (h, started, release)
}

fn start_gated_import(h: &Harness) -> tokio::task::JoinHandle<folio_core::Result<IngestOutcome>> {
    use folio_core::ports::VaultIngest;
    let pipeline = h.pipeline.clone();
    let vault = h.vault;
    let source = folio_core::models::Id::new_v4();
    tokio::spawn(async move {
        pipeline
            .ingest(vault, source, file_item(b"concurrent", &[]))
            .await
    })
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(5), future)
        .await
        .expect("ingest operation timed out")
}

#[tokio::test]
async fn concurrent_import_is_busy_until_the_owner_commits_then_duplicate() {
    let (h, started, release) = gated_harness(false).await;
    let owner = start_gated_import(&h);
    bounded(started.notified()).await;

    assert!(matches!(
        bounded(h.ingest(file_item(b"concurrent", &[]))).await,
        IngestOutcome::Busy
    ));
    release.notify_one();
    assert!(matches!(
        bounded(owner).await.unwrap().unwrap(),
        IngestOutcome::Imported(_)
    ));
    assert!(matches!(
        bounded(h.ingest(file_item(b"concurrent", &[]))).await,
        IngestOutcome::Duplicate
    ));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_import_can_retry_after_the_owner_fails() {
    let (h, started, release) = gated_harness(true).await;
    let owner = start_gated_import(&h);
    bounded(started.notified()).await;

    assert!(matches!(
        bounded(h.ingest(file_item(b"concurrent", &[]))).await,
        IngestOutcome::Busy
    ));
    release.notify_one();
    assert!(bounded(owner).await.unwrap().is_err());
    assert!(matches!(
        bounded(h.ingest(file_item(b"concurrent", &[]))).await,
        IngestOutcome::Imported(_)
    ));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn cancelling_the_owner_keeps_the_pending_claim_busy() {
    let (h, started, _release) = gated_harness(false).await;
    let owner = start_gated_import(&h);
    bounded(started.notified()).await;
    owner.abort();
    assert!(bounded(owner).await.unwrap_err().is_cancelled());

    assert!(matches!(
        bounded(h.ingest(file_item(b"concurrent", &[]))).await,
        IngestOutcome::Busy
    ));
    let retained = h
        .ledger
        .find(h.vault, &content_key(b"concurrent"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.outcome, "claimed");
    assert_eq!(retained.document_id, None);
    assert!(h.documents.list(h.vault, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn imports_with_source_tags_and_the_stored_file() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(doc) = h.ingest(file_item(b"one", &["Insurance", "2025"])).await
    else {
        panic!("imported")
    };

    let stored = h
        .documents
        .find_by_id(h.vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.source.kind, SourceKind::Folder);
    assert_eq!(
        stored.source.folder.unwrap().relative_path,
        "Insurance/x.pdf"
    );
    let mut tags: Vec<_> = h
        .documents
        .get_tags(h.vault, doc.id)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    tags.sort();
    assert_eq!(tags, ["2025", "Insurance"]);
    assert!(h.store.exists(&stored.file_path.unwrap()).await.unwrap());
}

#[tokio::test]
async fn the_same_content_is_deduplicated_until_its_document_is_permanently_deleted() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(first) = h.ingest(file_item(b"same", &[])).await else {
        panic!("imported")
    };
    assert!(matches!(
        h.ingest(file_item(b"same", &["Other"])).await,
        IngestOutcome::Duplicate
    ));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);

    h.documents.delete(h.vault, first.id).await.unwrap();
    assert!(matches!(
        h.ingest(file_item(b"same", &[])).await,
        IngestOutcome::Duplicate
    ));
    assert_eq!(h.documents.list_trash(h.vault).await.unwrap().len(), 1);
    assert!(h.documents.list(h.vault, None).await.unwrap().is_empty());

    h.documents.restore(h.vault, first.id).await.unwrap();
    assert!(matches!(
        h.ingest(file_item(b"same", &[])).await,
        IngestOutcome::Duplicate
    ));
    assert_eq!(
        h.documents.list(h.vault, None).await.unwrap()[0].id,
        first.id
    );
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);

    h.documents.delete(h.vault, first.id).await.unwrap();
    h.documents.request_purge(h.vault, first.id).await.unwrap();
    assert!(matches!(
        h.ingest(file_item(b"same", &[])).await,
        IngestOutcome::Duplicate
    ));
    folio_core::ports::purge_document(h.store.as_ref(), first.id)
        .await
        .unwrap();
    h.documents.finalize_purge(h.vault, first.id).await.unwrap();

    let IngestOutcome::Imported(reimported) = h.ingest(file_item(b"same", &[])).await else {
        panic!("permanently deleted content may be imported again");
    };
    assert_ne!(reimported.id, first.id);
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn pending_content_is_busy_and_preserves_the_claim() {
    let h = Harness::new().await;
    let key = content_key(b"pending");
    let claim = LedgerEntry {
        key: key.clone(),
        source_id: h.source_id,
        document_id: None,
        outcome: "claimed".into(),
        created_at: chrono::Utc::now(),
    };
    assert!(h.ledger.insert(h.vault, &claim).await.unwrap());

    let outcome = h.ingest(file_item(b"pending", &[])).await;

    assert!(matches!(outcome, IngestOutcome::Busy));
    assert!(h.documents.list(h.vault, None).await.unwrap().is_empty());
    let retained = h.ledger.find(h.vault, &key).await.unwrap().unwrap();
    assert_eq!(retained.outcome, "claimed");
    assert_eq!(retained.created_at, claim.created_at);
    assert_eq!(retained.document_id, None);
}

#[tokio::test]
async fn old_pending_content_is_busy_until_recovery_can_prove_it_is_abandoned() {
    let h = Harness::new().await;
    let key = content_key(b"old pending");
    let claim = LedgerEntry {
        key: key.clone(),
        source_id: h.source_id,
        document_id: None,
        outcome: "claimed".into(),
        created_at: chrono::Utc::now() - chrono::Duration::hours(1),
    };
    assert!(h.ledger.insert(h.vault, &claim).await.unwrap());

    let outcome = h.ingest(file_item(b"old pending", &[])).await;

    assert!(matches!(outcome, IngestOutcome::Busy));
    assert!(h.documents.list(h.vault, None).await.unwrap().is_empty());
    let retained = h.ledger.find(h.vault, &key).await.unwrap().unwrap();
    assert_eq!(retained.created_at, claim.created_at);
    assert_eq!(retained.document_id, None);
}

#[tokio::test]
async fn a_pending_claim_with_a_stored_document_is_still_busy() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(document) = h.ingest(file_item(b"not committed", &[])).await else {
        panic!("imported")
    };
    let key = content_key(b"not committed");
    h.ledger
        .complete(h.vault, &key, "claimed", Some(document.id))
        .await
        .unwrap();

    let outcome = h.ingest(file_item(b"not committed", &[])).await;

    assert!(matches!(outcome, IngestOutcome::Busy));
    let retained = h.ledger.find(h.vault, &key).await.unwrap().unwrap();
    assert_eq!(retained.outcome, "claimed");
    assert_eq!(retained.document_id, Some(document.id));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn committed_content_without_its_original_is_busy_and_preserves_the_record() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(document) = h.ingest(file_item(b"missing original", &[])).await
    else {
        panic!("imported")
    };
    h.store
        .delete(&document.file_path.clone().unwrap())
        .await
        .unwrap();

    let outcome = h.ingest(file_item(b"missing original", &[])).await;

    assert!(matches!(outcome, IngestOutcome::Busy));
    let retained = h
        .ledger
        .find(h.vault, &content_key(b"missing original"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.outcome, "imported");
    assert_eq!(retained.document_id, Some(document.id));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn pre_ingest_rules_discard_matching_mail_before_import() {
    let h = Harness::new().await;
    h.rule(
        "Ignore newsletters",
        SourceConditions {
            kind: Some(SourceKind::Email),
            email_from: Some(TextMatch::Glob("*@newsletter.test*".into())),
            ..Default::default()
        },
    )
    .await;
    h.rule(
        "Ignore receipts by subject",
        SourceConditions {
            email_subject: Some(TextMatch::Regex(r"(?i)^receipt\b".into())),
            ..Default::default()
        },
    )
    .await;

    let outcome = h
        .ingest(mail_item(
            "Deals <deals@newsletter.test>",
            "Sale",
            "a.pdf",
            b"a",
        ))
        .await;
    assert!(
        matches!(&outcome, IngestOutcome::Skipped { rule } if rule == "Ignore newsletters"),
        "{outcome:?}"
    );
    assert!(matches!(
        h.ingest(mail_item("shop@x.test", "Receipt 7", "b.pdf", b"b"))
            .await,
        IngestOutcome::Skipped { .. }
    ));
    assert!(matches!(
        h.ingest(mail_item("bank@x.test", "Statement", "c.pdf", b"c"))
            .await,
        IngestOutcome::Imported(_)
    ));
    assert_eq!(h.documents.list(h.vault, None).await.unwrap().len(), 1);

    // Skipped content is not remembered: lifting the rule lets it in.
    for workflow in h.workflows.list(h.vault).await.unwrap() {
        h.workflows.delete(h.vault, workflow.id).await.unwrap();
    }
    assert!(matches!(
        h.ingest(mail_item("deals@newsletter.test", "Sale", "a.pdf", b"a"))
            .await,
        IngestOutcome::Imported(_)
    ));
}

#[tokio::test]
async fn uploaded_workflows_see_the_source_in_conditions_and_payload() {
    let h = Harness::new().await;
    let create = |source: SourceConditions| {
        h.workflows.create(
            h.vault,
            WorkflowInput {
                name: "w".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentUploaded,
                conditions: WorkflowConditions {
                    source,
                    ..Default::default()
                },
                actions: vec![WorkflowAction::Webhook {
                    url: "http://localhost:1/hook".into(),
                    method: Default::default(),
                    headers: Default::default(),
                    secret: None,
                }],
            },
        )
    };
    create(SourceConditions {
        email_from: Some(TextMatch::Contains("billing@".into())),
        email_attachment_glob: Some("inv-*.pdf".into()),
        email_mailbox_glob: Some("INBOX".into()),
        email_to: Some(TextMatch::Glob("me@*".into())),
        email_subject: Some(TextMatch::Contains("invoice".into())),
        ..Default::default()
    })
    .await
    .unwrap();
    create(SourceConditions {
        kind: Some(SourceKind::Folder),
        ..Default::default()
    })
    .await
    .unwrap();

    let IngestOutcome::Imported(doc) = h
        .ingest(mail_item(
            "Billing <billing@shop.test>",
            "Your Invoice",
            "inv-42.pdf",
            b"inv",
        ))
        .await
    else {
        panic!("imported")
    };
    let runs = h.handle_created(doc.id).await;

    assert_eq!(runs.len(), 1, "the folder rule must not match a mail");
    let payload: serde_json::Value = serde_json::from_str(&runs[0].payload).unwrap();
    let source = &payload["document"]["source"];
    assert_eq!(source["kind"], "email");
    assert_eq!(source["email"]["subject"], "Your Invoice");
    assert_eq!(source["email"]["attachment_filename"], "inv-42.pdf");
}
