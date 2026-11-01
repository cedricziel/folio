//! One behavioural contract for every database backend.
//!
//! The same assertions run against SQLite (temp file) and PostgreSQL
//! (pgvector image via testcontainers). Every repository, search and queue
//! method is exercised on both.

mod common;

use chrono::{Duration, Utc};
use folio_core::error::Error;
use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, DocumentFilter, Id,
    UpdateCorrespondent, UpdateDocument, UpdateDocumentType, UpdateTag, WebhookMethod,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::ports::{EmbeddedChunk, SearchService};
use folio_infra::{Database, JobType, PostgresSearchService, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

fn new_document(title: &str) -> CreateDocument {
    CreateDocument {
        title: title.to_string(),
        content: Some("initial".to_string()),
        original_filename: Some(format!("{title}.pdf")),
        mime_type: Some("application/pdf".to_string()),
        correspondent_id: None,
        document_type_id: None,
    }
}

fn is_duplicate(e: Error) -> bool {
    matches!(e, Error::Duplicate { .. })
}

fn is_not_found<T: std::fmt::Debug>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::NotFound { .. }))
}

async fn tags_contract(r: &Repositories) {
    let tag = r
        .tags
        .create(CreateTag {
            name: "invoice".into(),
            color: Some("#ff0000".into()),
        })
        .await
        .unwrap();
    assert_eq!(tag.name, "invoice");

    let dup = r
        .tags
        .create(CreateTag {
            name: "invoice".into(),
            color: None,
        })
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));

    r.tags
        .create(CreateTag {
            name: "alpha".into(),
            color: None,
        })
        .await
        .unwrap();

    let found = r.tags.find_by_id(tag.id).await.unwrap().unwrap();
    assert_eq!(found.color.as_deref(), Some("#ff0000"));
    assert_eq!(
        r.tags.find_by_name("invoice").await.unwrap().unwrap().id,
        tag.id
    );
    assert!(r.tags.find_by_name("missing").await.unwrap().is_none());
    assert!(r.tags.find_by_id(Id::new_v4()).await.unwrap().is_none());

    let names: Vec<_> = r
        .tags
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["alpha", "invoice"]);

    let updated = r
        .tags
        .update(
            tag.id,
            UpdateTag {
                name: Some("bill".into()),
                color: Some(None),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "bill");
    assert_eq!(updated.color, None);
    let collision = r
        .tags
        .update(
            tag.id,
            UpdateTag {
                name: Some("alpha".into()),
                color: None,
            },
        )
        .await
        .unwrap_err();
    assert!(is_duplicate(collision));
    assert!(is_not_found(
        r.tags.update(Id::new_v4(), UpdateTag::default()).await
    ));

    r.tags.delete(tag.id).await.unwrap();
    assert!(r.tags.find_by_id(tag.id).await.unwrap().is_none());
    assert!(is_not_found(r.tags.delete(tag.id).await));
}

async fn correspondents_contract(r: &Repositories) {
    let c = r
        .correspondents
        .create(CreateCorrespondent {
            name: "ACME".into(),
        })
        .await
        .unwrap();
    let dup = r
        .correspondents
        .create(CreateCorrespondent {
            name: "ACME".into(),
        })
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));
    r.correspondents
        .create(CreateCorrespondent {
            name: "Zeta".into(),
        })
        .await
        .unwrap();

    assert_eq!(
        r.correspondents
            .find_by_id(c.id)
            .await
            .unwrap()
            .unwrap()
            .name,
        "ACME"
    );
    assert_eq!(
        r.correspondents
            .find_by_name("ACME")
            .await
            .unwrap()
            .unwrap()
            .id,
        c.id
    );
    assert!(r
        .correspondents
        .find_by_name("nope")
        .await
        .unwrap()
        .is_none());
    let names: Vec<_> = r
        .correspondents
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["ACME", "Zeta"]);

    let updated = r
        .correspondents
        .update(
            c.id,
            UpdateCorrespondent {
                name: Some("ACME Inc".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "ACME Inc");
    assert!(is_not_found(
        r.correspondents
            .update(Id::new_v4(), UpdateCorrespondent::default())
            .await
    ));

    r.correspondents.delete(c.id).await.unwrap();
    assert!(is_not_found(r.correspondents.delete(c.id).await));
}

async fn document_types_contract(r: &Repositories) {
    let t = r
        .document_types
        .create(CreateDocumentType {
            name: "Invoice".into(),
        })
        .await
        .unwrap();
    let dup = r
        .document_types
        .create(CreateDocumentType {
            name: "Invoice".into(),
        })
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));

    assert_eq!(
        r.document_types
            .find_by_id(t.id)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Invoice"
    );
    assert_eq!(
        r.document_types
            .find_by_name("Invoice")
            .await
            .unwrap()
            .unwrap()
            .id,
        t.id
    );
    assert!(r
        .document_types
        .find_by_name("nope")
        .await
        .unwrap()
        .is_none());
    assert_eq!(r.document_types.list().await.unwrap().len(), 1);

    let updated = r
        .document_types
        .update(
            t.id,
            UpdateDocumentType {
                name: Some("Receipt".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Receipt");
    assert!(is_not_found(
        r.document_types
            .update(Id::new_v4(), UpdateDocumentType::default())
            .await
    ));

    r.document_types.delete(t.id).await.unwrap();
    assert!(is_not_found(r.document_types.delete(t.id).await));
}

async fn documents_contract(r: &Repositories) {
    let corr = r
        .correspondents
        .create(CreateCorrespondent {
            name: "Docs Corp".into(),
        })
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(CreateDocumentType {
            name: "Contract".into(),
        })
        .await
        .unwrap();

    let first = r.documents.create(new_document("first")).await.unwrap();
    let mut with_meta = new_document("second");
    with_meta.correspondent_id = Some(corr.id);
    with_meta.document_type_id = Some(kind.id);
    let second = r.documents.create(with_meta).await.unwrap();

    let found = r.documents.find_by_id(second.id).await.unwrap().unwrap();
    assert_eq!(found.title, "second");
    assert_eq!(found.correspondent_id, Some(corr.id));
    assert_eq!(found.document_type_id, Some(kind.id));
    assert_eq!(found.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(found.file_path, None);
    assert_eq!(found.page_count, None);
    assert!(r
        .documents
        .find_by_id(Id::new_v4())
        .await
        .unwrap()
        .is_none());

    let all = r.documents.list(None).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, second.id, "newest first");
    let by_corr = r
        .documents
        .list(Some(DocumentFilter {
            correspondent_id: Some(corr.id),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(by_corr.len(), 1);
    assert_eq!(by_corr[0].id, second.id);
    let by_type = r
        .documents
        .list(Some(DocumentFilter {
            document_type_id: Some(kind.id),
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(by_type.len(), 1);
    assert_eq!(
        r.documents
            .list(Some(DocumentFilter::default()))
            .await
            .unwrap()
            .len(),
        2
    );

    let updated = r
        .documents
        .update(
            second.id,
            UpdateDocument {
                title: Some("renamed".into()),
                content: Some("extracted text".into()),
                correspondent_id: Some(None),
                document_type_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "renamed");
    assert_eq!(updated.correspondent_id, None);
    assert_eq!(updated.document_type_id, Some(kind.id));
    let reread = r.documents.find_by_id(second.id).await.unwrap().unwrap();
    assert_eq!(reread.content.as_deref(), Some("extracted text"));
    assert_eq!(reread.correspondent_id, None);
    assert!(is_not_found(
        r.documents
            .update(Id::new_v4(), UpdateDocument::default())
            .await
    ));

    r.documents
        .set_file_path(first.id, "docs/first.pdf")
        .await
        .unwrap();
    r.documents.set_page_count(first.id, 7).await.unwrap();
    let reread = r.documents.find_by_id(first.id).await.unwrap().unwrap();
    assert_eq!(reread.file_path.as_deref(), Some("docs/first.pdf"));
    assert_eq!(reread.page_count, Some(7));
    assert!(is_not_found(
        r.documents.set_file_path(Id::new_v4(), "x").await
    ));
    assert!(is_not_found(
        r.documents.set_page_count(Id::new_v4(), 1).await
    ));

    // Tags
    let a = r
        .tags
        .create(CreateTag {
            name: "doc-a".into(),
            color: None,
        })
        .await
        .unwrap();
    let b = r
        .tags
        .create(CreateTag {
            name: "doc-b".into(),
            color: None,
        })
        .await
        .unwrap();
    r.documents.set_tags(first.id, &[a.id, b.id]).await.unwrap();
    assert_eq!(r.documents.get_tags(first.id).await.unwrap().len(), 2);
    r.documents.set_tags(first.id, &[a.id]).await.unwrap();
    assert_eq!(r.documents.get_tags(first.id).await.unwrap().len(), 1);
    r.documents.add_tag(first.id, b.id).await.unwrap();
    r.documents.add_tag(first.id, b.id).await.unwrap();
    assert_eq!(r.documents.get_tags(first.id).await.unwrap().len(), 2);
    r.documents.remove_tag(first.id, a.id).await.unwrap();
    let remaining = r.documents.get_tags(first.id).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, b.id);

    // Deleting a tag or a document cleans up associations
    r.tags.delete(b.id).await.unwrap();
    assert!(r.documents.get_tags(first.id).await.unwrap().is_empty());
    r.documents.set_tags(first.id, &[a.id]).await.unwrap();
    r.documents.delete(first.id).await.unwrap();
    assert!(r.documents.find_by_id(first.id).await.unwrap().is_none());
    assert!(is_not_found(r.documents.delete(first.id).await));
    assert!(r.tags.find_by_id(a.id).await.unwrap().is_some());

    // Deleting a correspondent keeps the document and clears the link
    let mut linked = new_document("linked");
    linked.correspondent_id = Some(corr.id);
    let linked = r.documents.create(linked).await.unwrap();
    r.correspondents.delete(corr.id).await.unwrap();
    assert_eq!(
        r.documents
            .find_by_id(linked.id)
            .await
            .unwrap()
            .unwrap()
            .correspondent_id,
        None
    );
}

async fn document_filter_contract(r: &Repositories) {
    let tax = r
        .tags
        .create(CreateTag {
            name: "filter-tax".into(),
            color: None,
        })
        .await
        .unwrap();
    let home = r
        .tags
        .create(CreateTag {
            name: "filter-home".into(),
            color: None,
        })
        .await
        .unwrap();
    let acme = r
        .correspondents
        .create(CreateCorrespondent {
            name: "Filter Acme".into(),
        })
        .await
        .unwrap();

    let mut invoice = new_document("Filter invoice");
    invoice.content = Some("Total due: 100 EUR for March".into());
    let invoice = r.documents.create(invoice).await.unwrap();
    let mut letter = new_document("Filter letter");
    letter.content = Some("Dear Sir".into());
    let letter = r.documents.create(letter).await.unwrap();
    r.documents.add_tag(invoice.id, tax.id).await.unwrap();
    r.documents.add_tag(invoice.id, home.id).await.unwrap();
    r.documents.add_tag(letter.id, home.id).await.unwrap();
    r.documents
        .update(
            invoice.id,
            UpdateDocument {
                correspondent_id: Some(Some(acme.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let ids = |docs: Vec<folio_core::models::Document>| {
        docs.into_iter().map(|d| d.id).collect::<Vec<_>>()
    };
    let list =
        |filter: DocumentFilter| async move { r.documents.list(Some(filter)).await.unwrap() };

    // Every term must match, case-insensitively, in any order and field
    let by_terms = list(DocumentFilter {
        search_query: Some("100 INVOICE".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(ids(by_terms), vec![invoice.id]);
    // LIKE wildcards in user input match literally
    let wildcard = list(DocumentFilter {
        search_query: Some("%".into()),
        ..Default::default()
    })
    .await;
    assert!(wildcard.is_empty());

    // A document must carry all requested tags; repeated ids count once
    let all_tags = list(DocumentFilter {
        tag_ids: Some(vec![tax.id, home.id]),
        ..Default::default()
    })
    .await;
    assert_eq!(ids(all_tags), vec![invoice.id]);
    let repeated = list(DocumentFilter {
        tag_ids: Some(vec![tax.id, tax.id]),
        ..Default::default()
    })
    .await;
    assert_eq!(ids(repeated), vec![invoice.id]);
    let one_tag = list(DocumentFilter {
        tag_ids: Some(vec![home.id]),
        ..Default::default()
    })
    .await;
    assert_eq!(one_tag.len(), 2);

    // Filters combine
    let combined = list(DocumentFilter {
        tag_ids: Some(vec![home.id]),
        correspondent_id: Some(acme.id),
        search_query: Some("march".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(ids(combined), vec![invoice.id]);
    let none = list(DocumentFilter {
        tag_ids: Some(vec![tax.id]),
        correspondent_id: Some(Id::new_v4()),
        ..Default::default()
    })
    .await;
    assert!(none.is_empty());

    // created_after is inclusive, created_before exclusive
    let stored = r.documents.find_by_id(invoice.id).await.unwrap().unwrap();
    let at = stored.created_at;
    let has = |docs: &[folio_core::models::Document]| docs.iter().any(|d| d.id == invoice.id);
    let from = list(DocumentFilter {
        created_after: Some(at),
        ..Default::default()
    })
    .await;
    assert!(has(&from));
    let until_exact = list(DocumentFilter {
        created_before: Some(at),
        ..Default::default()
    })
    .await;
    assert!(!has(&until_exact));
    let until_later = list(DocumentFilter {
        created_before: Some(at + Duration::seconds(1)),
        ..Default::default()
    })
    .await;
    assert!(has(&until_later));
    let future = list(DocumentFilter {
        created_after: Some(Utc::now() + Duration::days(1)),
        ..Default::default()
    })
    .await;
    assert!(future.is_empty());
}

async fn settings_contract(r: &Repositories) {
    let seeded = r.settings.get("ocr.provider").await.unwrap().unwrap();
    assert_eq!(seeded.value, "tesseract");
    assert!(r.settings.get("ai.provider").await.unwrap().is_none());
    assert!(r.settings.get("nope.nope").await.unwrap().is_none());

    let set = r.settings.set("test.key", "one").await.unwrap();
    assert_eq!(set.value, "one");
    let set = r.settings.set("test.key", "two").await.unwrap();
    assert_eq!(set.value, "two");
    assert_eq!(
        r.settings.get("test.key").await.unwrap().unwrap().value,
        "two"
    );

    r.settings
        .set_many(&[
            ("test.a".to_string(), "1".to_string()),
            ("test.b".to_string(), "2".to_string()),
            ("test.key".to_string(), "three".to_string()),
        ])
        .await
        .unwrap();
    let by_prefix = r.settings.list_by_prefix("test.").await.unwrap();
    let keys: Vec<_> = by_prefix.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["test.a", "test.b", "test.key"]);
    assert_eq!(by_prefix[2].value, "three");
    assert!(r.settings.list().await.unwrap().len() >= 12);
    assert!(!r.settings.list_by_prefix("ai.").await.unwrap().is_empty());

    r.settings.delete("test.a").await.unwrap();
    assert!(r.settings.get("test.a").await.unwrap().is_none());
    r.settings.delete("test.a").await.unwrap();
}

fn chunk(index: u32, text: &str, embedding: &[f32]) -> EmbeddedChunk {
    EmbeddedChunk {
        index,
        text: text.to_string(),
        embedding: embedding.to_vec(),
    }
}

async fn search_contract(
    documents: &dyn folio_core::ports::DocumentRepository,
    search: &dyn SearchService,
) {
    const M: &str = "test:model";
    let near = documents.create(new_document("near")).await.unwrap();
    let mid = documents.create(new_document("mid")).await.unwrap();
    let far = documents.create(new_document("far")).await.unwrap();

    // Several chunks per document: the best chunk decides the document's score
    search
        .store_chunks(
            near.id,
            M,
            &[
                chunk(0, "off topic", &[0.0, 0.0, 1.0]),
                chunk(1, "the hit", &[1.0, 0.0, 0.0]),
            ],
        )
        .await
        .unwrap();
    search
        .store_chunks(mid.id, M, &[chunk(0, "middle", &[1.0, 1.0, 0.0])])
        .await
        .unwrap();
    search
        .store_chunks(far.id, M, &[chunk(0, "far away", &[0.0, 0.0, 1.0])])
        .await
        .unwrap();

    let results = search.search(M, &[1.0, 0.1, 0.0], 10).await.unwrap();
    let ids: Vec<_> = results.iter().map(|r| r.document_id).collect();
    assert_eq!(ids, [near.id, mid.id, far.id]);
    assert_eq!(results[0].snippet.as_deref(), Some("the hit"));
    assert!(results[0].score > results[1].score && results[1].score > results[2].score);
    assert!(results[0].score > 0.9);

    assert_eq!(
        search.search(M, &[1.0, 0.1, 0.0], 2).await.unwrap().len(),
        2
    );

    // Vectors of another model are never compared
    assert!(search
        .search("other:model", &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .is_empty());

    // Switching the active model flags the old vectors stale; switching back restores them
    assert_eq!(search.mark_stale_except("other:model").await.unwrap(), 3);
    assert!(search
        .search(M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(search.mark_stale_except(M).await.unwrap(), 0);
    assert_eq!(
        search.search(M, &[1.0, 0.1, 0.0], 10).await.unwrap().len(),
        3
    );

    // Storing again replaces the document's chunks
    search
        .store_chunks(far.id, M, &[chunk(0, "now close", &[1.0, 0.1, 0.0])])
        .await
        .unwrap();
    let top = search.search(M, &[1.0, 0.1, 0.0], 1).await.unwrap();
    assert_eq!(top[0].document_id, far.id);

    search.delete_embedding(far.id).await.unwrap();
    let ids: Vec<_> = search
        .search(M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert_eq!(ids, [near.id, mid.id]);

    // Deleting the document removes its chunks
    documents.delete(near.id).await.unwrap();
    let ids: Vec<_> = search
        .search(M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert_eq!(ids, [mid.id]);
    documents.delete(mid.id).await.unwrap();
}

async fn jobs_contract(r: &Repositories) {
    let doc = Id::new_v4();
    let other = Id::new_v4();
    let jobs = &r.jobs;

    assert!(jobs.dequeue().await.unwrap().is_none());
    assert_eq!(jobs.pending_count().await.unwrap(), 0);

    let first = jobs
        .enqueue(JobType::ExtractText { document_id: doc })
        .await
        .unwrap();
    let second = jobs
        .enqueue(JobType::GenerateThumbnail { document_id: other })
        .await
        .unwrap();
    let delayed = jobs
        .enqueue_delayed(
            JobType::GenerateEmbedding { document_id: doc },
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(jobs.pending_count().await.unwrap(), 3);

    let active = jobs.find_active_jobs_for_document(doc).await.unwrap();
    assert_eq!(active.len(), 2);
    assert!(active.iter().any(|j| j.id == first));
    assert!(active.iter().any(|j| j.id == delayed));
    assert!(!jobs.has_failed_jobs_for_document(doc).await.unwrap());

    // Oldest due job first; the delayed one is not due
    let claimed = jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(claimed.id, first);
    assert_eq!(claimed.attempts, 1);
    assert!(matches!(claimed.job_type, JobType::ExtractText { document_id } if document_id == doc));
    assert_eq!(claimed.status.to_string(), "processing");
    assert!(claimed.started_at.is_some());
    assert_eq!(jobs.dequeue().await.unwrap().unwrap().id, second);
    assert!(jobs.dequeue().await.unwrap().is_none());

    // Processing jobs still count as active; completing removes them
    assert_eq!(
        jobs.find_active_jobs_for_document(other)
            .await
            .unwrap()
            .len(),
        1
    );
    jobs.complete(second).await.unwrap();
    assert!(jobs
        .find_active_jobs_for_document(other)
        .await
        .unwrap()
        .is_empty());

    // Retry puts it back with a delay
    jobs.fail(first, "boom", true).await.unwrap();
    assert!(jobs.dequeue().await.unwrap().is_none());
    assert!(!jobs.has_failed_jobs_for_document(doc).await.unwrap());
    assert_eq!(jobs.pending_count().await.unwrap(), 2);

    // Permanent failure
    jobs.fail(first, "fatal", false).await.unwrap();
    assert!(jobs.has_failed_jobs_for_document(doc).await.unwrap());
    assert!(!jobs.has_failed_jobs_for_document(other).await.unwrap());

    // Stuck processing jobs are reset; recent ones are left alone
    let stuck = jobs
        .enqueue(JobType::ProcessDocument { document_id: other })
        .await
        .unwrap();
    assert_eq!(jobs.dequeue().await.unwrap().unwrap().id, stuck);
    assert_eq!(jobs.reset_stuck(60).await.unwrap(), 0);
    assert_eq!(jobs.reset_stuck(-1).await.unwrap(), 1);
    assert_eq!(jobs.dequeue().await.unwrap().unwrap().id, stuck);

    // Cleanup only removes old completed jobs
    jobs.complete(stuck).await.unwrap();
    assert_eq!(jobs.cleanup(1).await.unwrap(), 0);
    assert_eq!(jobs.cleanup(-1).await.unwrap(), 2);
}

async fn concurrent_dequeue_contract(r: &Repositories) {
    for _ in 0..8 {
        r.jobs
            .enqueue(JobType::GenerateThumbnail {
                document_id: Id::new_v4(),
            })
            .await
            .unwrap();
    }
    let mut handles = Vec::new();
    for _ in 0..8 {
        let jobs = r.jobs.clone();
        handles.push(tokio::spawn(async move { jobs.dequeue().await.unwrap() }));
    }
    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.await.unwrap().expect("a job for every worker").id);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "no job is handed out twice");
}

fn workflow_input(name: &str, trigger: WorkflowTrigger) -> WorkflowInput {
    WorkflowInput {
        name: name.to_string(),
        enabled: true,
        trigger,
        conditions: WorkflowConditions {
            mime_type: Some("application/pdf".into()),
            ..Default::default()
        },
        actions: vec![WorkflowAction::Webhook {
            url: "http://192.168.1.10:5678/hook".into(),
            method: WebhookMethod::Post,
            headers: Default::default(),
            secret: Some("s3cret".into()),
        }],
    }
}

async fn workflows_contract(r: &Repositories) {
    let uploaded = WorkflowTrigger::DocumentUploaded;
    let wf = r
        .workflows
        .create(workflow_input("Notify", uploaded))
        .await
        .unwrap();
    let fetched = r.workflows.find_by_id(wf.id).await.unwrap().unwrap();
    assert_eq!(fetched.name, "Notify");
    assert_eq!(fetched.conditions, wf.conditions);
    assert_eq!(fetched.actions, wf.actions);
    assert!(r
        .workflows
        .find_by_id(Id::new_v4())
        .await
        .unwrap()
        .is_none());

    // Invalid workflows never reach the table
    let mut bad = workflow_input("Bad", uploaded);
    bad.actions = vec![];
    assert!(matches!(
        r.workflows.create(bad).await,
        Err(Error::Validation(_))
    ));

    let mut off = workflow_input("Off", uploaded);
    off.enabled = false;
    let off = r.workflows.create(off).await.unwrap();
    r.workflows
        .create(workflow_input(
            "Other trigger",
            WorkflowTrigger::DocumentDeleted,
        ))
        .await
        .unwrap();
    let enabled: Vec<_> = r
        .workflows
        .list_enabled(uploaded)
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    assert_eq!(enabled, vec![wf.id]);
    assert_eq!(r.workflows.list().await.unwrap().len(), 3);

    let mut changed = workflow_input("Renamed", uploaded);
    changed.enabled = false;
    let updated = r.workflows.update(wf.id, changed).await.unwrap();
    assert_eq!(updated.name, "Renamed");
    assert!(!updated.enabled);
    assert!(is_not_found(
        r.workflows
            .update(Id::new_v4(), workflow_input("x", uploaded))
            .await
    ));

    // Runs: round trip, ordering, update
    let doc = Id::new_v4();
    let run = WorkflowRun::new(off.id, uploaded, Some(doc), r#"{"a":  1}"#.into());
    assert!(r.workflows.create_run(&run).await.unwrap());
    let mut stored = r.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        stored.payload, r#"{"a":  1}"#,
        "payload is kept byte for byte"
    );
    assert_eq!(stored.document_id, Some(doc));
    stored.attempts = 2;
    stored.completed_actions = 1;
    stored.last_error = Some("HTTP 500".into());
    stored.response_status = Some(500);
    r.workflows.update_run(&stored).await.unwrap();
    let reread = r.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(
        (
            reread.attempts,
            reread.completed_actions,
            reread.response_status
        ),
        (2, 1, Some(500))
    );
    assert_eq!(reread.last_error.as_deref(), Some("HTTP 500"));

    // Once-per-document events collapse to a single run, even concurrently
    let once = WorkflowRun::new(off.id, uploaded, Some(doc), "{}".into());
    assert!(!r.workflows.create_run(&once).await.unwrap());
    let race_doc = Id::new_v4();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let workflows = r.workflows.clone();
        let run = WorkflowRun::new(off.id, uploaded, Some(race_doc), "{}".into());
        handles.push(tokio::spawn(async move {
            workflows.create_run(&run).await.unwrap()
        }));
    }
    let mut inserted = 0;
    for handle in handles {
        inserted += usize::from(handle.await.unwrap());
    }
    assert_eq!(inserted, 1);

    // Other events may repeat
    for _ in 0..2 {
        let again = WorkflowRun::new(
            off.id,
            WorkflowTrigger::DocumentUpdated,
            Some(doc),
            "{}".into(),
        );
        assert!(r.workflows.create_run(&again).await.unwrap());
    }
    let runs = r.workflows.list_runs(off.id, 100).await.unwrap();
    assert_eq!(runs.len(), 4);
    assert_eq!(r.workflows.list_runs(off.id, 2).await.unwrap().len(), 2);
    assert!(runs.windows(2).all(|w| w[0].created_at >= w[1].created_at));

    // Deleting a workflow removes its runs
    r.workflows.delete(off.id).await.unwrap();
    assert!(r.workflows.find_run(run.id).await.unwrap().is_none());
    assert!(is_not_found(r.workflows.delete(off.id).await));
}

async fn auth_contract(r: &Repositories) {
    use folio_auth::{AccessToken, AuthCode, OAuthClient, RefreshToken, Session, User};
    let auth = &r.auth;
    let user = |name: &str| User {
        id: Id::new_v4().to_string(),
        username: name.to_string(),
        password_hash: "hash".into(),
        created_at: 1,
        email: None,
        display_name: None,
        is_admin: false,
        auth_source: "local".into(),
        external_id: None,
    };

    assert_eq!(auth.count_users().await.unwrap(), 0);
    // Simultaneous first logins (setup form and external providers): the
    // emptiness check is atomic with the insert, so exactly one becomes owner.
    let external = |i: usize| User {
        auth_source: "forward".into(),
        external_id: Some(format!("ext{i}")),
        ..user(&format!("ext{i}"))
    };
    let mut tasks = tokio::task::JoinSet::new();
    {
        let auth = r.auth.clone();
        let owner = user("Owner");
        tasks.spawn(async move { auth.create_first_user(owner).await.map(|u| u.is_some()) });
    }
    for i in 0..16 {
        let auth = r.auth.clone();
        let u = external(i);
        tasks.spawn(async move { auth.create_external_user(u, true).await.map(|()| true) });
    }
    while let Some(done) = tasks.join_next().await {
        done.unwrap().unwrap();
    }
    let mut admins = 0;
    for name in ["Owner"]
        .into_iter()
        .map(String::from)
        .chain((0..16).map(|i| format!("ext{i}")))
    {
        if let Some(u) = auth.find_user_by_username(&name).await.unwrap() {
            admins += i32::from(u.is_admin);
        }
    }
    assert_eq!(admins, 1, "exactly one first user is admin");
    let owner = match auth.find_user_by_username("Owner").await.unwrap() {
        Some(owner) => owner,
        None => {
            auth.create_user(user("Owner")).await.unwrap();
            auth.find_user_by_username("Owner").await.unwrap().unwrap()
        }
    };
    assert!(auth
        .create_first_user(user("second"))
        .await
        .unwrap()
        .is_none());
    // Later external users stay plain, and a repeated identity conflicts.
    auth.create_external_user(external(100), true)
        .await
        .unwrap();
    let late = auth
        .find_user_by_external("forward", "ext100")
        .await
        .unwrap()
        .unwrap();
    assert!(!late.is_admin);
    assert!(matches!(
        auth.create_external_user(external(100), true).await,
        Err(folio_auth::AuthError::Conflict)
    ));
    assert_eq!(auth.count_users().await.unwrap(), 18);
    let by_name = auth.find_user_by_username("oWnEr").await.unwrap().unwrap();
    assert_eq!(by_name.id, owner.id);
    assert_eq!(by_name.username, "Owner");
    assert!(auth
        .find_user_by_username("nobody")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        auth.find_user_by_id(&owner.id)
            .await
            .unwrap()
            .unwrap()
            .username,
        "Owner"
    );

    // External users: linked by (source, external id), usernames stay unique
    let ext = |name: &str, ext_id: &str| User {
        auth_source: "forward".into(),
        external_id: Some(ext_id.into()),
        password_hash: "!".into(),
        email: Some(format!("{name}@example.com")),
        ..user(name)
    };
    let alice = ext("alice", "alice");
    auth.create_user(alice.clone()).await.unwrap();
    assert!(matches!(
        auth.create_user(ext("ALICE", "other")).await,
        Err(folio_auth::AuthError::Conflict)
    ));
    assert!(matches!(
        auth.create_user(ext("alice2", "alice")).await,
        Err(folio_auth::AuthError::Conflict)
    ));
    let found = auth
        .find_user_by_external("forward", "alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, alice.id);
    assert_eq!(found.email.as_deref(), Some("alice@example.com"));
    assert!(!found.is_admin);
    assert!(auth
        .find_user_by_external("oidc", "alice")
        .await
        .unwrap()
        .is_none());
    auth.update_user_profile(&alice.id, None, Some("Alice A."), true)
        .await
        .unwrap();
    let updated = auth.find_user_by_id(&alice.id).await.unwrap().unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("Alice A."));
    assert_eq!(updated.email, None);
    assert!(updated.is_admin);

    let names = |groups: Vec<folio_auth::Group>| -> Vec<String> {
        groups.into_iter().map(|g| g.name).collect()
    };
    auth.set_external_groups(
        &alice.id,
        &["family".into(), "admins".into(), "family".into()],
    )
    .await
    .unwrap();
    let groups = auth.user_groups(&alice.id).await.unwrap();
    assert!(groups
        .iter()
        .all(|g| g.source == folio_auth::GroupSource::External));
    assert_eq!(names(groups), ["admins", "family"]);
    auth.set_external_groups(&alice.id, &["kids".into()])
        .await
        .unwrap();
    assert_eq!(names(auth.user_groups(&alice.id).await.unwrap()), ["kids"]);
    auth.set_external_groups(&alice.id, &[]).await.unwrap();
    assert!(auth.user_groups(&alice.id).await.unwrap().is_empty());

    // Simultaneous syncs for one user (parallel first requests) must all
    // succeed and leave exactly one of the requested sets behind.
    let sets: Vec<Vec<String>> = vec![
        vec!["a".into(), "b".into()],
        vec!["b".into(), "c".into()],
        vec!["a".into()],
        vec!["c".into(), "d".into(), "a".into()],
    ];
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..24 {
        let auth = r.auth.clone();
        let id = alice.id.clone();
        let set = sets[i % sets.len()].clone();
        tasks.spawn(async move { auth.set_external_groups(&id, &set).await });
    }
    while let Some(done) = tasks.join_next().await {
        done.unwrap().expect("concurrent group sync failed");
    }
    let mut left = names(auth.user_groups(&alice.id).await.unwrap());
    left.sort();
    assert!(
        sets.iter().any(|set| {
            let mut set = set.clone();
            set.sort();
            set == left
        }),
        "mixed result: {left:?}"
    );
    auth.set_external_groups(&alice.id, &[]).await.unwrap();

    let far = i64::MAX / 2;
    auth.create_session(Session {
        id_hash: "sess".into(),
        user_id: owner.id.clone(),
        expires_at: far,
    })
    .await
    .unwrap();
    assert_eq!(
        auth.find_session("sess").await.unwrap().unwrap().user_id,
        owner.id
    );
    auth.delete_session("sess").await.unwrap();
    assert!(auth.find_session("sess").await.unwrap().is_none());

    auth.insert_client(OAuthClient {
        client_id: "client".into(),
        client_name: "Claude".into(),
        redirect_uris: vec!["https://a.example/cb".into(), "app:/cb".into()],
        created_at: 1,
    })
    .await
    .unwrap();
    let client = auth.find_client("client").await.unwrap().unwrap();
    assert_eq!(client.redirect_uris, ["https://a.example/cb", "app:/cb"]);
    assert!(auth.find_client("other").await.unwrap().is_none());

    let code = |hash: &str, expires_at| AuthCode {
        code_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        redirect_uri: "https://a.example/cb".into(),
        code_challenge: "challenge".into(),
        scopes: vec!["documents:read".into(), "documents:write".into()],
        audience: "https://folio.example/mcp".into(),
        expires_at,
    };
    auth.insert_code(code("c1", far)).await.unwrap();
    let taken = auth.take_code("c1").await.unwrap().unwrap();
    assert_eq!(taken.scopes, ["documents:read", "documents:write"]);
    assert_eq!(taken.audience, "https://folio.example/mcp");
    assert!(
        auth.take_code("c1").await.unwrap().is_none(),
        "codes are single-use"
    );

    let access = |hash: &str, expires_at| AccessToken {
        token_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        scopes: vec!["documents:read".into()],
        audience: "aud".into(),
        expires_at,
    };
    let refresh = |hash: &str, expires_at| RefreshToken {
        token_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        scopes: vec!["documents:read".into()],
        audience: "aud".into(),
        expires_at,
    };
    auth.insert_access_token(access("a1", far)).await.unwrap();
    assert_eq!(
        auth.find_access_token("a1").await.unwrap().unwrap().scopes,
        ["documents:read"]
    );
    auth.insert_refresh_token(refresh("r1", far)).await.unwrap();
    assert!(auth.take_refresh_token("r1").await.unwrap().is_some());
    assert!(
        auth.take_refresh_token("r1").await.unwrap().is_none(),
        "rotation consumes"
    );
    auth.insert_refresh_token(refresh("r2", far)).await.unwrap();
    auth.revoke_token("a1").await.unwrap();
    auth.revoke_token("r2").await.unwrap();
    auth.revoke_token("unknown").await.unwrap();
    assert!(auth.find_access_token("a1").await.unwrap().is_none());
    assert!(auth.take_refresh_token("r2").await.unwrap().is_none());

    auth.insert_access_token(access("old", 5)).await.unwrap();
    auth.insert_code(code("oldcode", 5)).await.unwrap();
    auth.purge_expired(10).await.unwrap();
    assert!(auth.find_access_token("old").await.unwrap().is_none());
    assert!(auth.take_code("oldcode").await.unwrap().is_none());
}

async fn run_contract(db: &Database) {
    db.migrate().await.unwrap();
    // Migrating twice is a no-op
    db.migrate().await.unwrap();
    let repos = db.repositories().await;

    tags_contract(&repos).await;
    correspondents_contract(&repos).await;
    document_types_contract(&repos).await;
    documents_contract(&repos).await;
    document_filter_contract(&repos).await;
    settings_contract(&repos).await;
    search_contract(repos.documents.as_ref(), repos.search.as_ref()).await;
    workflows_contract(&repos).await;
    jobs_contract(&repos).await;
    concurrent_dequeue_contract(&repos).await;
    auth_contract(&repos).await;
}

#[tokio::test]
async fn sqlite_satisfies_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("folio.db").display());
    let db = Database::connect(&url).await.unwrap();
    run_contract(&db).await;
}

#[tokio::test]
async fn postgres_satisfies_the_contract() {
    let started = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await;
    let Some(container) = common::container_or_skip("PostgreSQL (pgvector/pgvector:pg17)", started)
    else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let db = Database::connect(&url).await.unwrap();
    run_contract(&db).await;

    let Database::Postgres(pool) = &db else {
        panic!("expected a PostgreSQL database");
    };
    // The pgvector image must actually exercise the pgvector path
    assert!(PostgresSearchService::new(pool.clone())
        .await
        .uses_pgvector());

    // The Rust cosine fallback satisfies the same search contract
    let repos = db.repositories().await;
    let fallback = PostgresSearchService::without_pgvector(pool.clone());
    search_contract(repos.documents.as_ref(), &fallback).await;
}

#[tokio::test]
async fn postgres_without_the_vector_extension_falls_back() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL (postgres:17-alpine)", started)
    else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let db = Database::connect(&url).await.unwrap();
    run_contract(&db).await;

    let Database::Postgres(pool) = &db else {
        panic!("expected a PostgreSQL database");
    };
    assert!(!PostgresSearchService::new(pool.clone())
        .await
        .uses_pgvector());
}
