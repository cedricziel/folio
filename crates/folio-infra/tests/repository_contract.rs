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
use folio_core::ports::SearchService;
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

async fn settings_contract(r: &Repositories) {
    let seeded = r.settings.get("ai.provider").await.unwrap().unwrap();
    assert_eq!(seeded.value, "local");
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

async fn search_contract(
    documents: &dyn folio_core::ports::DocumentRepository,
    search: &dyn SearchService,
) {
    let near = documents.create(new_document("near")).await.unwrap();
    let mid = documents.create(new_document("mid")).await.unwrap();
    let far = documents.create(new_document("far")).await.unwrap();

    search
        .store_embedding(near.id, &[1.0, 0.0, 0.0])
        .await
        .unwrap();
    search
        .store_embedding(mid.id, &[1.0, 1.0, 0.0])
        .await
        .unwrap();
    search
        .store_embedding(far.id, &[0.0, 0.0, 1.0])
        .await
        .unwrap();

    let results = search.search(&[1.0, 0.1, 0.0], 10).await.unwrap();
    let ids: Vec<_> = results.iter().map(|r| r.document_id).collect();
    assert_eq!(ids, [near.id, mid.id, far.id]);
    assert!(results[0].score > results[1].score && results[1].score > results[2].score);
    assert!(results[0].score > 0.9);

    assert_eq!(search.search(&[1.0, 0.1, 0.0], 2).await.unwrap().len(), 2);

    // Storing again replaces the vector
    search
        .store_embedding(far.id, &[1.0, 0.1, 0.0])
        .await
        .unwrap();
    let top = search.search(&[1.0, 0.1, 0.0], 1).await.unwrap();
    assert_eq!(top[0].document_id, far.id);

    search.delete_embedding(far.id).await.unwrap();
    let ids: Vec<_> = search
        .search(&[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert_eq!(ids, [near.id, mid.id]);

    // Deleting the document removes its embedding
    documents.delete(near.id).await.unwrap();
    let ids: Vec<_> = search
        .search(&[1.0, 0.1, 0.0], 10)
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

async fn run_contract(db: &Database) {
    db.migrate().await.unwrap();
    // Migrating twice is a no-op
    db.migrate().await.unwrap();
    let repos = db.repositories().await;

    tags_contract(&repos).await;
    correspondents_contract(&repos).await;
    document_types_contract(&repos).await;
    documents_contract(&repos).await;
    settings_contract(&repos).await;
    search_contract(repos.documents.as_ref(), repos.search.as_ref()).await;
    workflows_contract(&repos).await;
    jobs_contract(&repos).await;
    concurrent_dequeue_contract(&repos).await;
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
