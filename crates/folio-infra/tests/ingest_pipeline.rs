//! Pre-ingest rules, de-duplication, tags, and source-aware workflows on real SQLite.

mod common;

use common::ingest::{file_item, mail_item, Harness};
use folio_core::models::{
    IngestOutcome, SourceConditions, SourceKind, TextMatch, WorkflowAction, WorkflowConditions,
    WorkflowInput, WorkflowTrigger,
};
use folio_core::DocumentEvent;

#[tokio::test]
async fn imports_with_source_tags_and_the_stored_file() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(doc) = h.ingest(file_item(b"one", &["Insurance", "2025"])).await
    else {
        panic!("imported")
    };

    let stored = h.documents.find_by_id(doc.id).await.unwrap().unwrap();
    assert_eq!(stored.source.kind, SourceKind::Folder);
    assert_eq!(
        stored.source.folder.unwrap().relative_path,
        "Insurance/x.pdf"
    );
    let mut tags: Vec<_> = h
        .documents
        .get_tags(doc.id)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    tags.sort();
    assert_eq!(tags, ["2025", "Insurance"]);
    assert!(h.dir.join(stored.file_path.unwrap()).exists());
}

#[tokio::test]
async fn the_same_content_is_not_imported_twice_until_its_document_is_gone() {
    let h = Harness::new().await;
    let IngestOutcome::Imported(first) = h.ingest(file_item(b"same", &[])).await else {
        panic!("imported")
    };
    assert!(matches!(
        h.ingest(file_item(b"same", &["Other"])).await,
        IngestOutcome::Duplicate
    ));
    assert_eq!(h.documents.list(None).await.unwrap().len(), 1);

    h.documents.delete(first.id).await.unwrap();
    assert!(matches!(
        h.ingest(file_item(b"same", &[])).await,
        IngestOutcome::Imported(_)
    ));
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
    assert_eq!(h.documents.list(None).await.unwrap().len(), 1);

    // Skipped content is not remembered: lifting the rule lets it in.
    for workflow in h.workflows.list().await.unwrap() {
        h.workflows.delete(workflow.id).await.unwrap();
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
        h.workflows.create(WorkflowInput {
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
        })
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
    let runs = h
        .engine
        .handle(&DocumentEvent::Created {
            document_id: doc.id,
        })
        .await
        .unwrap();

    assert_eq!(runs.len(), 1, "the folder rule must not match a mail");
    let payload: serde_json::Value = serde_json::from_str(&runs[0].payload).unwrap();
    let source = &payload["document"]["source"];
    assert_eq!(source["kind"], "email");
    assert_eq!(source["email"]["subject"], "Your Invoice");
    assert_eq!(source["email"]["attachment_filename"], "inv-42.pdf");
}
