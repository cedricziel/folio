//! One behavioural contract for every database backend.
//!
//! The same assertions run against SQLite (temp file) and PostgreSQL
//! (pgvector image via testcontainers). Every repository, search and queue
//! method is exercised on both.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use folio_core::error::Error;
use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, DocumentFilter, Id,
    UpdateCorrespondent, UpdateDocument, UpdateDocumentType, UpdateTag, VaultId, WebhookMethod,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::models::{
    DocumentSource, EmailOrigin, IngestionSourceInput, LedgerEntry, SourceConfig, SourceKind,
    SourceStatus,
};
use folio_core::ports::{EmbeddedChunk, ProcessingQueue, SearchService};
use folio_core::services::{DispatcherConfig, EventConsumer, OutboxDispatcher};
use folio_core::{DocumentEvent, EventOrigin, OutboxEvent};
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
        ..Default::default()
    }
}

fn is_duplicate(e: Error) -> bool {
    matches!(e, Error::Duplicate { .. })
}

fn is_not_found<T: std::fmt::Debug>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::NotFound { .. }))
}

async fn tags_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "invoice".into(),
                color: Some("#ff0000".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(tag.name, "invoice");
    assert_eq!(tag.vault_id, vault);

    let dup = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "invoice".into(),
                color: None,
            },
        )
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));

    r.tags
        .create(
            vault,
            CreateTag {
                name: "alpha".into(),
                color: None,
            },
        )
        .await
        .unwrap();

    let found = r.tags.find_by_id(vault, tag.id).await.unwrap().unwrap();
    assert_eq!(found.color.as_deref(), Some("#ff0000"));
    assert_eq!(found.vault_id, vault);
    assert_eq!(
        r.tags
            .find_by_name(vault, "invoice")
            .await
            .unwrap()
            .unwrap()
            .id,
        tag.id
    );
    assert!(r
        .tags
        .find_by_name(vault, "missing")
        .await
        .unwrap()
        .is_none());
    assert!(r
        .tags
        .find_by_id(vault, Id::new_v4())
        .await
        .unwrap()
        .is_none());

    let names: Vec<_> = r
        .tags
        .list(vault)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["alpha", "invoice"]);

    let updated = r
        .tags
        .update(
            vault,
            tag.id,
            UpdateTag {
                name: Some("bill".into()),
                color: Some(None),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "bill");
    assert_eq!(updated.vault_id, vault);
    assert_eq!(updated.color, None);
    let collision = r
        .tags
        .update(
            vault,
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
        r.tags
            .update(vault, Id::new_v4(), UpdateTag::default())
            .await
    ));

    r.tags.delete(vault, tag.id).await.unwrap();
    assert!(r.tags.find_by_id(vault, tag.id).await.unwrap().is_none());
    assert!(is_not_found(r.tags.delete(vault, tag.id).await));
}

async fn correspondents_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let c = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "ACME".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(c.vault_id, vault);
    let dup = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "ACME".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));
    r.correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Zeta".into(),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        r.correspondents
            .find_by_id(vault, c.id)
            .await
            .unwrap()
            .unwrap()
            .name,
        "ACME"
    );
    assert_eq!(
        r.correspondents
            .find_by_name(vault, "ACME")
            .await
            .unwrap()
            .unwrap()
            .id,
        c.id
    );
    assert!(r
        .correspondents
        .find_by_name(vault, "nope")
        .await
        .unwrap()
        .is_none());
    let names: Vec<_> = r
        .correspondents
        .list(vault)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["ACME", "Zeta"]);

    let updated = r
        .correspondents
        .update(
            vault,
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
            .update(vault, Id::new_v4(), UpdateCorrespondent::default())
            .await
    ));

    r.correspondents.delete(vault, c.id).await.unwrap();
    assert!(is_not_found(r.correspondents.delete(vault, c.id).await));
}

async fn document_types_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let t = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Invoice".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(t.vault_id, vault);
    let dup = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Invoice".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(is_duplicate(dup));

    assert_eq!(
        r.document_types
            .find_by_id(vault, t.id)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Invoice"
    );
    assert_eq!(
        r.document_types
            .find_by_name(vault, "Invoice")
            .await
            .unwrap()
            .unwrap()
            .id,
        t.id
    );
    assert!(r
        .document_types
        .find_by_name(vault, "nope")
        .await
        .unwrap()
        .is_none());
    assert_eq!(r.document_types.list(vault).await.unwrap().len(), 1);

    let updated = r
        .document_types
        .update(
            vault,
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
            .update(vault, Id::new_v4(), UpdateDocumentType::default())
            .await
    ));

    r.document_types.delete(vault, t.id).await.unwrap();
    assert!(is_not_found(r.document_types.delete(vault, t.id).await));
}

async fn documents_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let corr = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Docs Corp".into(),
            },
        )
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Contract".into(),
            },
        )
        .await
        .unwrap();

    let first = r
        .documents
        .create(vault, new_document("first"))
        .await
        .unwrap();
    let mut with_meta = new_document("second");
    with_meta.correspondent_id = Some(corr.id);
    with_meta.document_type_id = Some(kind.id);
    let second = r.documents.create(vault, with_meta).await.unwrap();

    let found = r
        .documents
        .find_by_id(vault, second.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.title, "second");
    assert_eq!(found.correspondent_id, Some(corr.id));
    assert_eq!(found.document_type_id, Some(kind.id));
    assert_eq!(found.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(found.file_path, None);
    assert_eq!(found.page_count, None);
    assert!(r
        .documents
        .find_by_id(vault, Id::new_v4())
        .await
        .unwrap()
        .is_none());

    let all = r.documents.list(vault, None).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, second.id, "newest first");
    let by_corr = r
        .documents
        .list(
            vault,
            Some(DocumentFilter {
                correspondent_id: Some(corr.id),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(by_corr.len(), 1);
    assert_eq!(by_corr[0].id, second.id);
    let by_type = r
        .documents
        .list(
            vault,
            Some(DocumentFilter {
                document_type_id: Some(kind.id),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    assert_eq!(by_type.len(), 1);
    assert_eq!(
        r.documents
            .list(vault, Some(DocumentFilter::default()))
            .await
            .unwrap()
            .len(),
        2
    );

    let updated = r
        .documents
        .update(
            vault,
            second.id,
            UpdateDocument {
                title: Some("renamed".into()),
                content: Some("extracted text".into()),
                correspondent_id: Some(None),
                document_type_id: None,
                tag_ids: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "renamed");
    assert_eq!(updated.correspondent_id, None);
    assert_eq!(updated.document_type_id, Some(kind.id));
    let reread = r
        .documents
        .find_by_id(vault, second.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reread.content.as_deref(), Some("extracted text"));
    assert_eq!(reread.correspondent_id, None);
    assert!(is_not_found(
        r.documents
            .update(vault, Id::new_v4(), UpdateDocument::default())
            .await
    ));

    r.documents
        .set_file_path(vault, first.id, "docs/first.pdf")
        .await
        .unwrap();
    r.documents
        .set_page_count(vault, first.id, 7)
        .await
        .unwrap();
    let reread = r
        .documents
        .find_by_id(vault, first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reread.file_path.as_deref(), Some("docs/first.pdf"));
    assert_eq!(reread.page_count, Some(7));
    assert!(is_not_found(
        r.documents.set_file_path(vault, Id::new_v4(), "x").await
    ));
    assert!(is_not_found(
        r.documents.set_page_count(vault, Id::new_v4(), 1).await
    ));

    // Tags
    let a = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "doc-a".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let b = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "doc-b".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    r.documents
        .set_tags(vault, first.id, &[a.id, b.id])
        .await
        .unwrap();
    assert_eq!(
        r.documents.get_tags(vault, first.id).await.unwrap().len(),
        2
    );
    r.documents
        .set_tags(vault, first.id, &[a.id])
        .await
        .unwrap();
    assert_eq!(
        r.documents.get_tags(vault, first.id).await.unwrap().len(),
        1
    );
    r.documents.add_tag(vault, first.id, b.id).await.unwrap();
    r.documents.add_tag(vault, first.id, b.id).await.unwrap();
    assert_eq!(
        r.documents.get_tags(vault, first.id).await.unwrap().len(),
        2
    );
    r.documents.remove_tag(vault, first.id, a.id).await.unwrap();
    let remaining = r.documents.get_tags(vault, first.id).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, b.id);

    // Deleting a tag cleans up its associations
    r.tags.delete(vault, b.id).await.unwrap();
    assert!(r
        .documents
        .get_tags(vault, first.id)
        .await
        .unwrap()
        .is_empty());
    r.documents
        .set_tags(vault, first.id, &[a.id])
        .await
        .unwrap();
    r.documents.delete(vault, first.id).await.unwrap();
    assert!(r
        .documents
        .find_by_id(vault, first.id)
        .await
        .unwrap()
        .is_none());
    r.documents.delete(vault, first.id).await.unwrap();
    assert!(r.tags.find_by_id(vault, a.id).await.unwrap().is_some());

    // Deleting a correspondent keeps the document and clears the link
    let mut linked = new_document("linked");
    linked.correspondent_id = Some(corr.id);
    let linked = r.documents.create(vault, linked).await.unwrap();
    r.correspondents.delete(vault, corr.id).await.unwrap();
    assert_eq!(
        r.documents
            .find_by_id(vault, linked.id)
            .await
            .unwrap()
            .unwrap()
            .correspondent_id,
        None
    );
}

async fn document_filter_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    let tax = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "filter-tax".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let home = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "filter-home".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let acme = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Filter Acme".into(),
            },
        )
        .await
        .unwrap();

    let mut invoice = new_document("Filter invoice");
    invoice.content = Some("Total due: 100 EUR for March".into());
    let invoice = r.documents.create(vault, invoice).await.unwrap();
    let mut letter = new_document("Filter letter");
    letter.content = Some("Dear Sir".into());
    let letter = r.documents.create(vault, letter).await.unwrap();
    r.documents
        .add_tag(vault, invoice.id, tax.id)
        .await
        .unwrap();
    r.documents
        .add_tag(vault, invoice.id, home.id)
        .await
        .unwrap();
    r.documents
        .add_tag(vault, letter.id, home.id)
        .await
        .unwrap();
    r.documents
        .update(
            vault,
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
    let list = |filter: DocumentFilter| async move {
        r.documents.list(vault, Some(filter)).await.unwrap()
    };

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
    let stored = r
        .documents
        .find_by_id(vault, invoice.id)
        .await
        .unwrap()
        .unwrap();
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

    // list_ids applies the same filter and order as list
    for filter in [
        DocumentFilter::default(),
        DocumentFilter {
            tag_ids: Some(vec![home.id]),
            search_query: Some("march".into()),
            ..Default::default()
        },
        DocumentFilter {
            created_after: Some(Utc::now() + Duration::days(1)),
            ..Default::default()
        },
    ] {
        let listed = ids(list(filter.clone()).await);
        assert_eq!(
            r.documents.list_ids(vault, Some(filter)).await.unwrap(),
            listed,
            "list_ids agrees with list"
        );
    }
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
    assert!(r
        .settings
        .list()
        .await
        .unwrap()
        .iter()
        .any(|setting| { setting.key == "test.key" && setting.value == "three" }));
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
    db: &Database,
    vault: folio_core::models::VaultId,
    documents: &dyn folio_core::ports::DocumentRepository,
    search: &dyn SearchService,
) {
    const M: &str = "test:model";
    let near = documents.create(vault, new_document("near")).await.unwrap();
    let mid = documents.create(vault, new_document("mid")).await.unwrap();
    let far = documents.create(vault, new_document("far")).await.unwrap();

    // Several chunks per document: the best chunk decides the document's score
    search
        .store_chunks(
            vault,
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
        .store_chunks(vault, mid.id, M, &[chunk(0, "middle", &[1.0, 1.0, 0.0])])
        .await
        .unwrap();
    search
        .store_chunks(vault, far.id, M, &[chunk(0, "far away", &[0.0, 0.0, 1.0])])
        .await
        .unwrap();

    let results = search.search(vault, M, &[1.0, 0.1, 0.0], 10).await.unwrap();
    let ids: Vec<_> = results.iter().map(|r| r.document_id).collect();
    assert_eq!(ids, [near.id, mid.id, far.id]);
    assert_eq!(results[0].snippet.as_deref(), Some("the hit"));
    assert!(results[0].score > results[1].score && results[1].score > results[2].score);
    assert!(results[0].score > 0.9);

    assert_eq!(
        search
            .search(vault, M, &[1.0, 0.1, 0.0], 2)
            .await
            .unwrap()
            .len(),
        2
    );

    // Vectors of another model are never compared
    assert!(search
        .search(vault, "other:model", &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .is_empty());

    // Switching the active model flags the old vectors stale; switching back restores them
    assert_eq!(search.mark_stale_except("other:model").await.unwrap(), 3);
    assert!(search
        .search(vault, M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(search.mark_stale_except(M).await.unwrap(), 0);
    assert_eq!(
        search
            .search(vault, M, &[1.0, 0.1, 0.0], 10)
            .await
            .unwrap()
            .len(),
        3
    );

    // Storing again replaces the document's chunks
    search
        .store_chunks(vault, far.id, M, &[chunk(0, "now close", &[1.0, 0.1, 0.0])])
        .await
        .unwrap();
    let top = search.search(vault, M, &[1.0, 0.1, 0.0], 1).await.unwrap();
    assert_eq!(top[0].document_id, far.id);

    search.delete_embedding(vault, far.id).await.unwrap();
    let ids: Vec<_> = search
        .search(vault, M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert_eq!(ids, [near.id, mid.id]);

    // Trashed documents are absent from semantic search
    documents.delete(vault, near.id).await.unwrap();
    let ids: Vec<_> = search
        .search(vault, M, &[1.0, 0.1, 0.0], 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    assert_eq!(ids, [mid.id]);
    documents.delete(vault, mid.id).await.unwrap();

    documents.request_purge(vault, near.id).await.unwrap();
    documents.finalize_purge(vault, near.id).await.unwrap();
    documents.request_purge(vault, mid.id).await.unwrap();
    documents.finalize_purge(vault, mid.id).await.unwrap();
    // The storage-free fixtures have already been purged; their jobs must not enter queue tests.
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("DELETE FROM jobs WHERE job_type = 'purge_document' AND json_extract(payload, '$.data.document_id') IN (?, ?)")
                .bind(near.id.to_string()).bind(mid.id.to_string()).execute(pool).await.unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("DELETE FROM jobs WHERE job_type = 'purge_document' AND payload->'data'->>'document_id' IN ($1, $2)")
                .bind(near.id.to_string()).bind(mid.id.to_string()).execute(pool).await.unwrap();
        }
    }
}

async fn jobs_contract(db: &Database, r: &Repositories) {
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
    let second_claim = jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(second_claim.id, second);
    assert!(jobs.dequeue().await.unwrap().is_none());

    // Processing jobs still count as active; completing removes them
    assert_eq!(
        jobs.find_active_jobs_for_document(other)
            .await
            .unwrap()
            .len(),
        1
    );
    jobs.complete(&second_claim).await.unwrap();
    assert!(jobs
        .find_active_jobs_for_document(other)
        .await
        .unwrap()
        .is_empty());

    // Retry puts it back with a delay
    jobs.fail(&claimed, "boom", true).await.unwrap();
    assert!(jobs.dequeue().await.unwrap().is_none());
    assert!(!jobs.has_failed_jobs_for_document(doc).await.unwrap());
    assert_eq!(jobs.pending_count().await.unwrap(), 2);

    // Permanent failure
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE jobs SET scheduled_at = '2000-01-01T00:00:00Z' WHERE id = ?")
                .bind(first.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE jobs SET scheduled_at = '2000-01-01T00:00:00Z' WHERE id = $1")
                .bind(first)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    let fatal_claim = jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(fatal_claim.id, first);
    assert_ne!(fatal_claim.claim_id, claimed.claim_id);
    jobs.fail(&fatal_claim, "fatal", false).await.unwrap();
    assert!(jobs.has_failed_jobs_for_document(doc).await.unwrap());
    assert!(!jobs.has_failed_jobs_for_document(other).await.unwrap());

    // Live claims are not recovered
    let stuck = jobs
        .enqueue(JobType::ProcessDocument { document_id: other })
        .await
        .unwrap();
    let stuck_claim = jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(stuck_claim.id, stuck);
    assert_eq!(jobs.reset_stuck().await.unwrap(), 0);

    // Cleanup only removes old completed jobs
    jobs.complete(&stuck_claim).await.unwrap();
    assert_eq!(jobs.cleanup(1).await.unwrap(), 0);
    assert_eq!(jobs.cleanup(-1).await.unwrap(), 2);
}

async fn processing_recovery_contract(r: &Repositories) {
    let id = Id::new_v4();
    let failed = r
        .jobs
        .enqueue(JobType::ProcessDocument { document_id: id })
        .await
        .unwrap();
    let failed_claim = r.jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(failed_claim.id, failed);
    r.jobs
        .fail(&failed_claim, "unreadable upload", false)
        .await
        .unwrap();
    assert_eq!(
        r.jobs.processing_status(id).await.unwrap(),
        folio_core::ports::ProcessingStatus::Failed
    );
    let unrelated = r
        .jobs
        .enqueue(JobType::GenerateEmbedding { document_id: id })
        .await
        .unwrap();
    let unrelated_claim = r.jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(unrelated_claim.id, unrelated);
    r.jobs.complete(&unrelated_claim).await.unwrap();
    assert_eq!(
        r.jobs.processing_status(id).await.unwrap(),
        folio_core::ports::ProcessingStatus::Failed,
        "successful embedding must not hide a failed processing stage"
    );
    let retry = r
        .jobs
        .enqueue(JobType::ProcessDocument { document_id: id })
        .await
        .unwrap();
    assert_eq!(
        r.jobs.processing_status(id).await.unwrap(),
        folio_core::ports::ProcessingStatus::Pending
    );
    let retry_claim = r.jobs.dequeue().await.unwrap().unwrap();
    assert_eq!(retry_claim.id, retry);
    r.jobs.complete(&retry_claim).await.unwrap();
    assert_eq!(
        r.jobs.processing_status(id).await.unwrap(),
        folio_core::ports::ProcessingStatus::Completed
    );
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
    let vault = r.vaults.instance_default().await.unwrap();
    let uploaded = WorkflowTrigger::DocumentUploaded;
    let wf = r
        .workflows
        .create(vault, workflow_input("Notify", uploaded))
        .await
        .unwrap();
    let fetched = r.workflows.find_by_id(vault, wf.id).await.unwrap().unwrap();
    assert_eq!(wf.vault_id, vault);
    assert_eq!(fetched.vault_id, vault);
    assert_eq!(fetched.name, "Notify");
    assert_eq!(fetched.conditions, wf.conditions);
    assert_eq!(fetched.actions, wf.actions);
    assert!(r
        .workflows
        .find_by_id(vault, Id::new_v4())
        .await
        .unwrap()
        .is_none());

    // Invalid workflows never reach the table
    let mut bad = workflow_input("Bad", uploaded);
    bad.actions = vec![];
    assert!(matches!(
        r.workflows.create(vault, bad).await,
        Err(Error::Validation(_))
    ));

    let mut off = workflow_input("Off", uploaded);
    off.enabled = false;
    let off = r.workflows.create(vault, off).await.unwrap();
    r.workflows
        .create(
            vault,
            workflow_input("Other trigger", WorkflowTrigger::DocumentDeleted),
        )
        .await
        .unwrap();
    let enabled: Vec<_> = r
        .workflows
        .list_enabled(vault, uploaded)
        .await
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    assert_eq!(enabled, vec![wf.id]);
    assert_eq!(r.workflows.list(vault).await.unwrap().len(), 3);

    let mut changed = workflow_input("Renamed", uploaded);
    changed.enabled = false;
    let updated = r.workflows.update(vault, wf.id, changed).await.unwrap();
    assert_eq!(updated.name, "Renamed");
    assert!(!updated.enabled);
    assert!(is_not_found(
        r.workflows
            .update(vault, Id::new_v4(), workflow_input("x", uploaded))
            .await
    ));

    // Runs: round trip, ordering, update
    let doc = Id::new_v4();
    let run = WorkflowRun::new(
        off.vault_id,
        off.id,
        uploaded,
        Some(doc),
        r#"{"a":  1}"#.into(),
    );
    let mut wrong_vault = run.clone();
    wrong_vault.vault_id = VaultId::new();
    assert!(matches!(
        r.workflows.create_run(&wrong_vault).await,
        Err(Error::Validation(_))
    ));
    assert!(r
        .workflows
        .find_run(wrong_vault.id)
        .await
        .unwrap()
        .is_none());
    assert!(r.workflows.create_run(&run).await.unwrap());
    let mut stored = r.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(stored.vault_id, vault);
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
    let once = WorkflowRun::new(off.vault_id, off.id, uploaded, Some(doc), "{}".into());
    assert!(!r.workflows.create_run(&once).await.unwrap());
    let race_doc = Id::new_v4();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let workflows = r.workflows.clone();
        let run = WorkflowRun::new(off.vault_id, off.id, uploaded, Some(race_doc), "{}".into());
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
            off.vault_id,
            off.id,
            WorkflowTrigger::DocumentUpdated,
            Some(doc),
            "{}".into(),
        );
        assert!(r.workflows.create_run(&again).await.unwrap());
    }
    let runs = r.workflows.list_runs(vault, off.id, 100).await.unwrap();
    assert_eq!(runs.len(), 4);
    assert!(runs.iter().all(|run| run.vault_id == vault));
    assert_eq!(
        r.workflows.list_runs(vault, off.id, 2).await.unwrap().len(),
        2
    );
    assert!(runs.windows(2).all(|w| w[0].created_at >= w[1].created_at));

    // Deleting a workflow removes its runs
    r.workflows.delete(vault, off.id).await.unwrap();
    assert!(r.workflows.find_run(run.id).await.unwrap().is_none());
    assert!(is_not_found(r.workflows.delete(vault, off.id).await));
}

async fn ingestion_contract(r: &Repositories) {
    let vault = r.vaults.instance_default().await.unwrap();
    // A document keeps its source and the tags it was created with
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "ingested".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let source = DocumentSource::email(
        "work",
        EmailOrigin {
            account: "work".into(),
            mailbox: "INBOX".into(),
            message_id: "m@test".into(),
            from: "a@b.test".into(),
            to: vec!["me@home.test".into()],
            subject: "Invoice".into(),
            date: None,
            attachment_filename: Some("inv.pdf".into()),
            attachment_content_type: Some("application/pdf".into()),
        },
    );
    let doc = r
        .documents
        .create(
            vault,
            CreateDocument {
                source: source.clone(),
                tag_ids: vec![tag.id, tag.id],
                ..new_document("mail")
            },
        )
        .await
        .unwrap();
    let found = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.source, source);
    assert_eq!(r.documents.get_tags(vault, doc.id).await.unwrap().len(), 1);
    let plain = r
        .documents
        .create(vault, new_document("plain"))
        .await
        .unwrap();
    assert_eq!(plain.source.kind, SourceKind::Upload);

    // Managed sources: unique names, replace, status
    let input = |name: &str| -> IngestionSourceInput {
        IngestionSourceInput {
            name: name.into(),
            enabled: true,
            config: serde_json::from_value::<SourceConfig>(
                serde_json::json!({"type": "folder", "path": "/consume"}),
            )
            .unwrap(),
        }
    };
    let created = r.sources.create(vault, input("inbox")).await.unwrap();
    assert!(is_duplicate(
        r.sources.create(vault, input("inbox")).await.unwrap_err()
    ));
    let mut off = input("inbox");
    off.enabled = false;
    let updated = r.sources.update(vault, created.id, off).await.unwrap();
    assert!(!updated.enabled);
    assert_eq!(r.sources.list(vault).await.unwrap().len(), 1);
    assert!(is_not_found(
        r.sources.update(vault, Id::new_v4(), input("x")).await
    ));

    assert!(r.sources.status(created.id).await.unwrap().is_none());
    let status = SourceStatus {
        last_run_at: Some(Utc::now()),
        last_error: Some("boom".into()),
        last_imported: 3,
        last_skipped: 2,
        last_failed: 1,
    };
    r.sources.save_status(created.id, &status).await.unwrap();
    r.sources.save_status(created.id, &status).await.unwrap();
    let saved = r.sources.status(created.id).await.unwrap().unwrap();
    assert_eq!(
        (saved.last_imported, saved.last_error.as_deref()),
        (3, Some("boom"))
    );
    r.sources.delete(vault, created.id).await.unwrap();
    assert!(r.sources.find(vault, created.id).await.unwrap().is_none());
    assert!(r.sources.status(created.id).await.unwrap().is_none());
    assert!(is_not_found(r.sources.delete(vault, created.id).await));

    // Ledger: the first insert wins, later ones report false
    let entry = LedgerEntry {
        vault_id: vault,
        key: "hash:abc".into(),
        source_id: Id::new_v4(),
        document_id: None,
        outcome: "claimed".into(),
        created_at: Utc::now(),
    };
    assert!(matches!(
        r.ledger.insert(VaultId::new(), &entry).await,
        Err(Error::Validation(_))
    ));
    assert!(r.ledger.insert(vault, &entry).await.unwrap());
    assert!(!r.ledger.insert(vault, &entry).await.unwrap());
    r.ledger
        .complete(vault, "hash:abc", "imported", Some(doc.id))
        .await
        .unwrap();
    let stored = r.ledger.find(vault, "hash:abc").await.unwrap().unwrap();
    assert_eq!(stored.vault_id, vault);
    let wrong_vault = VaultId::new();
    assert!(matches!(
        r.ledger
            .transition_if_matches(wrong_vault, &stored, "retryable", Utc::now())
            .await,
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        r.ledger.delete_if_matches(wrong_vault, &stored).await,
        Err(Error::Validation(_))
    ));
    assert_eq!(
        (stored.outcome.as_str(), stored.document_id),
        ("imported", Some(doc.id))
    );
    assert!(r.ledger.delete_if_matches(vault, &stored).await.unwrap());
    assert!(r.ledger.find(vault, "hash:abc").await.unwrap().is_none());
    assert!(r.ledger.insert(vault, &entry).await.unwrap());
    assert!(!r.ledger.delete_if_matches(vault, &stored).await.unwrap());
    let pending = r.ledger.find(vault, "hash:abc").await.unwrap().unwrap();
    assert_eq!(pending.outcome, "claimed");
    assert!(r.ledger.delete_if_matches(vault, &pending).await.unwrap());
    assert!(r.ledger.find(vault, "hash:abc").await.unwrap().is_none());

    let reservation = LedgerEntry {
        key: "hash:reserved".into(),
        document_id: Some(Id::new_v4()),
        ..entry
    };
    assert!(r.ledger.insert(vault, &reservation).await.unwrap());
    let reservation = r
        .ledger
        .find(vault, &reservation.key)
        .await
        .unwrap()
        .unwrap();
    let mut wrong_owner = reservation.clone();
    wrong_owner.vault_id = VaultId::new();
    assert!(matches!(
        r.documents
            .create_for_ingest(vault, &wrong_owner, new_document("wrong owner"))
            .await,
        Err(Error::Validation(_))
    ));
    let document = r
        .documents
        .create_for_ingest(
            vault,
            &reservation,
            CreateDocument {
                title: "reserved".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let repeated = r
        .documents
        .create_for_ingest(
            vault,
            &reservation,
            CreateDocument {
                title: "retry must preserve original metadata".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    assert!(r
        .documents
        .is_purging_or_missing(Id::new_v4())
        .await
        .unwrap());
    assert_eq!(document.id, repeated.id);
    assert_eq!(repeated.title, "reserved");
    assert!(r
        .ledger
        .transition_if_matches(vault, &reservation, "retryable", reservation.created_at)
        .await
        .unwrap());
    assert!(!r
        .ledger
        .transition_if_matches(vault, &reservation, "imported", reservation.created_at)
        .await
        .unwrap());
    assert!(r
        .documents
        .create_for_ingest(vault, &reservation, CreateDocument::default())
        .await
        .is_err());
    let moved_vault = r.vaults.create("Moved reservation", None).await.unwrap().id;
    r.documents
        .move_to(document.id, vault, moved_vault, None)
        .await
        .unwrap();
    assert!(!r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    r.documents.delete(moved_vault, document.id).await.unwrap();
    assert!(!r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    r.documents.restore(moved_vault, document.id).await.unwrap();
    assert!(!r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    r.documents.delete(moved_vault, document.id).await.unwrap();
    r.documents
        .request_purge(moved_vault, document.id)
        .await
        .unwrap();
    assert!(r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    r.documents
        .finalize_purge(moved_vault, document.id)
        .await
        .unwrap();
    let deleted = r
        .ledger
        .find(vault, &reservation.key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deleted.outcome, "deleted");
    assert!(r
        .documents
        .is_purging_or_missing(document.id)
        .await
        .unwrap());
    assert!(!r
        .ledger
        .transition_if_matches(vault, &reservation, "imported", reservation.created_at)
        .await
        .unwrap());
    assert!(r
        .documents
        .create_for_ingest(vault, &reservation, CreateDocument::default())
        .await
        .is_err());
    assert!(r
        .documents
        .locate_retained(document.id)
        .await
        .unwrap()
        .is_none());
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
        is_owner: false,
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
        tasks.spawn(async move { auth.create_external_user(u).await.map(|()| true) });
    }
    while let Some(done) = tasks.join_next().await {
        done.unwrap().unwrap();
    }
    let mut admins = 0;
    let mut owners = Vec::new();
    for name in ["Owner"]
        .into_iter()
        .map(String::from)
        .chain((0..16).map(|i| format!("ext{i}")))
    {
        if let Some(u) = auth.find_user_by_username(&name).await.unwrap() {
            admins += i32::from(u.is_admin);
            if u.is_owner {
                owners.push(u.id);
            }
        }
    }
    assert_eq!(admins, 1, "exactly one first user is admin");
    assert_eq!(owners.len(), 1, "exactly one first user is owner");
    // A profile sync can never demote the owner.
    auth.update_user_profile(&owners[0], None, None, false)
        .await
        .unwrap();
    assert!(
        auth.find_user_by_id(&owners[0])
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
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
    auth.create_external_user(external(100)).await.unwrap();
    let late = auth
        .find_user_by_external("forward", "ext100")
        .await
        .unwrap()
        .unwrap();
    assert!(!late.is_admin);
    assert!(matches!(
        auth.create_external_user(external(100)).await,
        Err(folio_auth::AuthError::Conflict)
    ));
    assert_eq!(auth.count_users().await.unwrap(), 18);
    let by_name = auth.find_user_by_username("oWnEr").await.unwrap().unwrap();
    assert_eq!(by_name.id, owner.id);
    assert_eq!(by_name.username, owner.username);
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
        owner.username
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
        created_at: 0,
        user_agent: None,
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

    for (id, created_at) in [("stale", 100), ("used", 100), ("fresh", 5_000)] {
        auth.insert_client(OAuthClient {
            client_id: id.into(),
            client_name: id.into(),
            redirect_uris: vec!["app:/cb".into()],
            created_at,
        })
        .await
        .unwrap();
    }
    auth.mark_client_used("used", 200).await.unwrap();
    auth.mark_client_used("client", 200).await.unwrap();
    assert_eq!(auth.purge_unused_clients(1_000).await.unwrap(), 1);
    assert!(auth.find_client("stale").await.unwrap().is_none());
    assert!(auth.find_client("used").await.unwrap().is_some());
    assert!(auth.find_client("fresh").await.unwrap().is_some());

    let code = |hash: &str, expires_at| AuthCode {
        code_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        redirect_uri: "https://a.example/cb".into(),
        code_challenge: "challenge".into(),
        scopes: vec!["documents:read".into(), "documents:write".into()],
        audience: "https://folio.example/mcp".into(),
        expires_at,
        grant_id: "grant".into(),
    };
    auth.insert_code(code("c1", far)).await.unwrap();
    let taken = auth.take_code("c1").await.unwrap().unwrap();
    assert_eq!(taken.scopes, ["documents:read", "documents:write"]);
    assert_eq!(taken.audience, "https://folio.example/mcp");
    assert!(
        auth.take_code("c1").await.unwrap().is_none(),
        "codes are single-use"
    );

    let access = |hash: &str, grant: &str, expires_at| AccessToken {
        token_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        scopes: vec!["documents:read".into()],
        audience: "aud".into(),
        expires_at,
        grant_id: grant.into(),
    };
    let refresh = |hash: &str, grant: &str, expires_at| RefreshToken {
        token_hash: hash.into(),
        client_id: "client".into(),
        user_id: owner.id.clone(),
        scopes: vec!["documents:read".into()],
        audience: "aud".into(),
        expires_at,
        grant_id: grant.into(),
    };
    auth.insert_access_token(access("a1", "g1", far))
        .await
        .unwrap();
    let found = auth.find_access_token("a1").await.unwrap().unwrap();
    assert_eq!(found.scopes, ["documents:read"]);
    assert_eq!(found.grant_id, "g1");
    auth.insert_refresh_token(refresh("r1", "g1", far))
        .await
        .unwrap();
    assert!(auth.used_refresh_grant("r1").await.unwrap().is_none());
    assert_eq!(
        auth.take_refresh_token("r1")
            .await
            .unwrap()
            .unwrap()
            .grant_id,
        "g1"
    );
    assert!(
        auth.take_refresh_token("r1").await.unwrap().is_none(),
        "rotation consumes"
    );
    assert_eq!(
        auth.used_refresh_grant("r1").await.unwrap().as_deref(),
        Some("g1"),
        "a rotated token is remembered so reuse can be detected"
    );
    assert!(auth.used_refresh_grant("unknown").await.unwrap().is_none());

    // Revoking any token of a grant takes the whole family, but only for its client.
    auth.insert_refresh_token(refresh("r2", "g1", far))
        .await
        .unwrap();
    auth.revoke_token("r2", "someone-else").await.unwrap();
    assert!(auth.find_access_token("a1").await.unwrap().is_some());
    auth.revoke_token("r2", "client").await.unwrap();
    assert!(auth.find_access_token("a1").await.unwrap().is_none());
    assert!(auth.take_refresh_token("r2").await.unwrap().is_none());
    auth.revoke_token("unknown", "client").await.unwrap();

    auth.insert_access_token(access("a2", "g2", far))
        .await
        .unwrap();
    auth.insert_access_token(access("a3", "g3", far))
        .await
        .unwrap();
    auth.revoke_grant("g2").await.unwrap();
    assert!(auth.find_access_token("a2").await.unwrap().is_none());
    assert!(auth.find_access_token("a3").await.unwrap().is_some());

    let login = |state: &str, exp: i64| folio_auth::OidcLogin {
        state_hash: state.into(),
        browser_hash: "browser".into(),
        nonce: "nonce".into(),
        pkce_verifier: "verifier".into(),
        next: "/documents".into(),
        expires_at: exp,
    };
    auth.insert_oidc_login(login("s1", far)).await.unwrap();
    let taken = auth.take_oidc_login("s1").await.unwrap().unwrap();
    assert_eq!(taken.browser_hash, "browser");
    assert_eq!(taken.nonce, "nonce");
    assert_eq!(taken.pkce_verifier, "verifier");
    assert_eq!(taken.next, "/documents");
    assert_eq!(taken.expires_at, far);
    assert!(
        auth.take_oidc_login("s1").await.unwrap().is_none(),
        "a callback can be replayed only once"
    );
    assert!(auth.take_oidc_login("unknown").await.unwrap().is_none());
    auth.insert_oidc_login(login("stale", 5)).await.unwrap();

    auth.create_session(Session {
        id_hash: "with-token".into(),
        user_id: owner.id.clone(),
        expires_at: far,
        created_at: 0,
        user_agent: None,
    })
    .await
    .unwrap();
    assert!(auth.session_id_token("with-token").await.unwrap().is_none());
    auth.set_session_id_token("with-token", "jwt")
        .await
        .unwrap();
    assert_eq!(
        auth.session_id_token("with-token")
            .await
            .unwrap()
            .as_deref(),
        Some("jwt")
    );
    assert!(auth.session_id_token("unknown").await.unwrap().is_none());

    auth.insert_access_token(access("old", "g9", 5))
        .await
        .unwrap();
    auth.insert_code(code("oldcode", 5)).await.unwrap();
    auth.purge_expired(10).await.unwrap();
    assert!(auth.find_access_token("old").await.unwrap().is_none());
    assert!(auth.take_code("oldcode").await.unwrap().is_none());
    assert!(auth.take_oidc_login("stale").await.unwrap().is_none());
}

async fn mfa_contract(r: &Repositories) {
    use folio_auth::{
        AccessToken, AuthError, OAuthClient, PasskeyRecord, RefreshToken, Session, User,
    };
    let auth = &r.auth;
    let make = |name: &str| User {
        id: Id::new_v4().to_string(),
        username: name.to_string(),
        password_hash: "hash".into(),
        created_at: 1,
        email: None,
        display_name: None,
        is_admin: false,
        is_owner: false,
        auth_source: "local".into(),
        external_id: None,
    };
    let (ada, bob) = (make("mfa-ada"), make("mfa-bob"));
    auth.create_user(ada.clone()).await.unwrap();
    auth.create_user(bob.clone()).await.unwrap();
    let hashes = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();

    // TOTP enrollment: pending until confirmed, then protected from overwrite.
    assert!(auth.get_totp(&ada.id).await.unwrap().is_none());
    assert!(auth.save_pending_totp(&ada.id, "seed1", 10).await.unwrap());
    assert!(auth.save_pending_totp(&ada.id, "seed2", 11).await.unwrap());
    let pending = auth.get_totp(&ada.id).await.unwrap().unwrap();
    assert_eq!(
        (pending.secret_enc.as_str(), pending.confirmed),
        ("seed2", false)
    );
    assert!(
        !auth.advance_totp_step(&ada.id, 5).await.unwrap(),
        "an unconfirmed enrollment accepts no logins"
    );
    assert!(!auth.confirm_totp(&bob.id, "seed1", 7, &[]).await.unwrap());
    assert!(
        !auth.confirm_totp(&ada.id, "seed1", 7, &[]).await.unwrap(),
        "a replaced seed cannot be confirmed by a code for the old seed"
    );
    assert!(!auth.get_totp(&ada.id).await.unwrap().unwrap().confirmed);

    assert!(auth
        .confirm_totp(&ada.id, "seed2", 7, &hashes(&["a", "b", "c"]))
        .await
        .unwrap());
    assert!(
        !auth
            .confirm_totp(&ada.id, "seed2", 8, &hashes(&["x"]))
            .await
            .unwrap(),
        "confirming twice does nothing"
    );
    assert!(
        !auth.delete_pending_totp(&ada.id).await.unwrap(),
        "cancellation cannot remove a factor confirmed concurrently"
    );
    let on = auth.get_totp(&ada.id).await.unwrap().unwrap();
    assert!((on.confirmed, on.last_step) == (true, 7));
    assert!(
        !auth.save_pending_totp(&ada.id, "other", 12).await.unwrap(),
        "a confirmed seed is not replaced"
    );
    assert_eq!(
        auth.get_totp(&ada.id).await.unwrap().unwrap().secret_enc,
        "seed2"
    );

    // Replay protection: a step is accepted once, older ones never.
    assert!(!auth.advance_totp_step(&ada.id, 7).await.unwrap());
    assert!(!auth.advance_totp_step(&ada.id, 6).await.unwrap());
    assert!(auth.advance_totp_step(&ada.id, 8).await.unwrap());
    assert!(!auth.advance_totp_step(&ada.id, 8).await.unwrap());

    // Recovery codes are single use and scoped to the user.
    assert_eq!(auth.count_recovery_codes(&ada.id).await.unwrap(), 3);
    assert!(!auth.use_recovery_code(&bob.id, "a").await.unwrap());
    assert!(auth.use_recovery_code(&ada.id, "a").await.unwrap());
    assert!(!auth.use_recovery_code(&ada.id, "a").await.unwrap());
    assert_eq!(auth.count_recovery_codes(&ada.id).await.unwrap(), 2);
    auth.replace_recovery_codes(&ada.id, &hashes(&["n1", "n2"]))
        .await
        .unwrap();
    assert!(!auth.use_recovery_code(&ada.id, "b").await.unwrap());
    assert!(auth.use_recovery_code(&ada.id, "n1").await.unwrap());
    auth.delete_totp(&ada.id).await.unwrap();
    assert!(auth.get_totp(&ada.id).await.unwrap().is_none());
    assert_eq!(auth.count_recovery_codes(&ada.id).await.unwrap(), 0);

    // Passkeys.
    let key = |id: &str, cred: &str, user: &User, at: i64| PasskeyRecord {
        id: id.into(),
        user_id: user.id.clone(),
        name: format!("key {id}"),
        credential_id: cred.into(),
        data: "{}".into(),
        created_at: at,
        last_used_at: None,
    };
    auth.insert_passkey(key("p1", "c1", &ada, 1)).await.unwrap();
    auth.insert_passkey(key("p2", "c2", &ada, 2)).await.unwrap();
    assert!(matches!(
        auth.insert_passkey(key("p3", "c1", &bob, 3)).await,
        Err(AuthError::Conflict)
    ));
    let listed = auth.list_passkeys(&ada.id).await.unwrap();
    assert_eq!(
        listed.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["p1", "p2"]
    );
    assert!(auth.list_passkeys(&bob.id).await.unwrap().is_empty());
    auth.touch_passkey("p1", "{\"counter\":2}", 99)
        .await
        .unwrap();
    let touched = &auth.list_passkeys(&ada.id).await.unwrap()[0];
    assert_eq!(
        (touched.data.as_str(), touched.last_used_at),
        ("{\"counter\":2}", Some(99))
    );
    assert!(!auth.rename_passkey(&bob.id, "p1", "stolen").await.unwrap());
    assert!(auth.rename_passkey(&ada.id, "p1", "Laptop").await.unwrap());
    assert_eq!(auth.list_passkeys(&ada.id).await.unwrap()[0].name, "Laptop");
    assert!(!auth.delete_passkey(&bob.id, "p1").await.unwrap());
    assert!(auth.delete_passkey(&ada.id, "p1").await.unwrap());
    assert_eq!(auth.list_passkeys(&ada.id).await.unwrap().len(), 1);

    // Sessions and password.
    let far = i64::MAX / 2;
    for (id, user, created, expires) in [
        ("s-old", &ada, 5, far),
        ("s-new", &ada, 9, far),
        ("s-dead", &ada, 1, 3),
        ("s-bob", &bob, 2, far),
    ] {
        auth.create_session(Session {
            id_hash: id.into(),
            user_id: user.id.clone(),
            expires_at: expires,
            created_at: created,
            user_agent: Some("Firefox".into()),
        })
        .await
        .unwrap();
    }
    let live = auth.list_sessions(&ada.id, 100).await.unwrap();
    assert_eq!(
        live.iter().map(|s| s.id_hash.as_str()).collect::<Vec<_>>(),
        ["s-new", "s-old"]
    );
    assert_eq!(live[0].user_agent.as_deref(), Some("Firefox"));
    auth.delete_user_sessions(&ada.id, Some("s-new"))
        .await
        .unwrap();
    assert!(auth.find_session("s-old").await.unwrap().is_none());
    assert!(auth.find_session("s-new").await.unwrap().is_some());
    assert!(auth.find_session("s-bob").await.unwrap().is_some());
    auth.delete_user_sessions(&ada.id, None).await.unwrap();
    assert!(auth.find_session("s-new").await.unwrap().is_none());
    auth.set_password_hash(&ada.id, "new-hash").await.unwrap();
    assert_eq!(
        auth.find_user_by_id(&ada.id)
            .await
            .unwrap()
            .unwrap()
            .password_hash,
        "new-hash"
    );

    // Connected clients.
    for id in ["mfa-c1", "mfa-c2"] {
        auth.insert_client(OAuthClient {
            client_id: id.into(),
            client_name: format!("Client {id}"),
            redirect_uris: vec!["app:/cb".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    }
    let access = |hash: &str, client: &str, user: &User, exp: i64| AccessToken {
        token_hash: hash.into(),
        client_id: client.into(),
        user_id: user.id.clone(),
        scopes: vec![],
        audience: "a".into(),
        expires_at: exp,
        grant_id: "mfa-g".into(),
    };
    auth.insert_access_token(access("mt1", "mfa-c1", &ada, 500))
        .await
        .unwrap();
    auth.insert_access_token(access("mt2", "mfa-c2", &ada, 50))
        .await
        .unwrap();
    auth.insert_access_token(access("mt3", "mfa-c1", &bob, 500))
        .await
        .unwrap();
    auth.insert_refresh_token(RefreshToken {
        token_hash: "mr1".into(),
        client_id: "mfa-c1".into(),
        user_id: ada.id.clone(),
        scopes: vec![],
        audience: "a".into(),
        expires_at: 900,
        grant_id: "mfa-g".into(),
    })
    .await
    .unwrap();
    let clients = auth.list_connected_clients(&ada.id, 100).await.unwrap();
    assert_eq!(clients.len(), 1, "expired tokens do not count");
    assert_eq!(
        (clients[0].client_id.as_str(), clients[0].expires_at),
        ("mfa-c1", 900)
    );
    auth.revoke_client_tokens(&ada.id, "mfa-c1").await.unwrap();
    assert!(auth
        .list_connected_clients(&ada.id, 100)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        auth.list_connected_clients(&bob.id, 100)
            .await
            .unwrap()
            .len(),
        1,
        "other users keep their tokens"
    );
}

/// Records the id of every event it is handed
#[derive(Default)]
struct Recorder(Mutex<Vec<i64>>);

impl Recorder {
    fn ids(&self) -> Vec<i64> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventConsumer for Recorder {
    async fn consume(&self, event: &OutboxEvent) -> folio_core::Result<()> {
        self.0.lock().unwrap().push(event.id);
        Ok(())
    }
}

fn lease() -> Duration {
    Duration::seconds(30)
}

/// Take every event that is due and mark it dispatched
async fn flush_outbox(r: &Repositories) -> Vec<OutboxEvent> {
    let mut all = Vec::new();
    loop {
        let batch = r.outbox.claim(100, lease(), 10).await.unwrap();
        if batch.is_empty() {
            return all;
        }
        for event in batch {
            r.outbox.mark_dispatched(event.id).await.unwrap();
            all.push(event);
        }
    }
}

fn dispatcher(r: &Repositories, consumer: Arc<Recorder>, batch_size: u32) -> OutboxDispatcher {
    OutboxDispatcher::new(
        r.outbox.clone(),
        vec![consumer],
        DispatcherConfig {
            batch_size,
            concurrency: 2,
            ..Default::default()
        },
    )
}

/// Run raw DDL in the dialect of the connected backend
async fn exec(db: &Database, sqlite: &[&str], postgres: &[&str]) {
    match db {
        Database::Sqlite(pool) => {
            for sql in sqlite {
                sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }
        Database::Postgres(pool) => {
            for sql in postgres {
                sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }
    }
}

async fn event_context_contract(db: &Database) {
    let r = db.repositories().await;
    let vault = r.vaults.create("Captured origin", None).await.unwrap().id;
    let outside = r
        .vaults
        .create("Captured destination", None)
        .await
        .unwrap()
        .id;
    let sender = r
        .correspondents
        .create(
            vault,
            CreateCorrespondent {
                name: "Original sender".into(),
            },
        )
        .await
        .unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Original type".into(),
            },
        )
        .await
        .unwrap();
    let alpha = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "alpha".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let beta = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "beta".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    flush_outbox(&r).await;
    let source = DocumentSource::folder(
        "captured folder",
        folio_core::models::FolderOrigin {
            relative_path: "records/original.pdf".into(),
            original_filename: "original.pdf".into(),
        },
    );
    let doc = r
        .documents
        .create_for_upload(
            vault,
            CreateDocument {
                title: "Original title".into(),
                original_filename: Some("original.pdf".into()),
                mime_type: Some("application/pdf".into()),
                content: Some("large OCR text".repeat(100_000)),
                correspondent_id: Some(sender.id),
                document_type_id: Some(kind.id),
                tag_ids: vec![beta.id, alpha.id],
                source: source.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(flush_outbox(&r).await.is_empty());
    r.documents
        .finalize_upload(vault, doc.id, "immutable/original")
        .await
        .unwrap();
    let created = flush_outbox(&r).await.pop().unwrap();
    let original = created.context.as_ref().unwrap();
    assert_eq!(original.document_id, doc.id);
    assert_eq!(original.vault_id, vault);
    assert_eq!(original.title, "Original title");
    assert_eq!(original.original_filename.as_deref(), Some("original.pdf"));
    assert_eq!(original.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(original.file_path.as_deref(), Some("immutable/original"));
    assert_eq!(original.source, source);
    assert_eq!(original.correspondent_id, Some(sender.id));
    assert_eq!(
        original.correspondent_name.as_deref(),
        Some("Original sender")
    );
    assert_eq!(original.document_type_id, Some(kind.id));
    assert_eq!(
        original.document_type_name.as_deref(),
        Some("Original type")
    );
    let mut expected_tags = vec![
        folio_core::models::NamedRef {
            id: alpha.id,
            name: "alpha".into(),
        },
        folio_core::models::NamedRef {
            id: beta.id,
            name: "beta".into(),
        },
    ];
    expected_tags.sort_by_key(|tag| tag.id);
    assert_eq!(original.tags, expected_tags);
    let payload = folio_core::events::encode_outbox_payload_with_context(
        created.event.as_ref().unwrap(),
        created.origin,
        Some(original),
    );
    assert!(
        payload.len() < 4096,
        "OCR text must not enter the event payload"
    );
    assert!(!payload.contains("large OCR text"));
    r.correspondents
        .update(
            vault,
            sender.id,
            UpdateCorrespondent {
                name: Some("Later sender".into()),
            },
        )
        .await
        .unwrap();
    r.document_types
        .update(
            vault,
            kind.id,
            UpdateDocumentType {
                name: Some("Later type".into()),
            },
        )
        .await
        .unwrap();
    r.tags
        .update(
            vault,
            alpha.id,
            UpdateTag {
                name: Some("later alpha".into()),
                color: None,
            },
        )
        .await
        .unwrap();
    exec(db,
        &["CREATE TRIGGER refuse_capture BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &["CREATE FUNCTION refuse_capture() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
          "CREATE TRIGGER refuse_capture BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_capture()"],
    ).await;
    assert!(r
        .documents
        .update(
            vault,
            doc.id,
            UpdateDocument {
                title: Some("must roll back".into()),
                ..Default::default()
            }
        )
        .await
        .is_err());
    exec(
        db,
        &["DROP TRIGGER refuse_capture"],
        &[
            "DROP TRIGGER refuse_capture ON outbox_events",
            "DROP FUNCTION refuse_capture()",
        ],
    )
    .await;
    assert_eq!(
        r.documents
            .find_by_id(vault, doc.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "Original title"
    );
    assert!(flush_outbox(&r).await.is_empty());
    r.workflow_documents
        .update(
            vault,
            doc.id,
            UpdateDocument {
                title: Some("Later title".into()),
                tag_ids: Some(vec![alpha.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let updated = flush_outbox(&r).await.pop().unwrap();
    assert_eq!(updated.origin, EventOrigin::Workflow);
    let later = updated.context.as_ref().unwrap();
    assert_eq!(later.title, "Later title");
    assert_eq!(later.correspondent_name.as_deref(), Some("Later sender"));
    assert_eq!(later.document_type_name.as_deref(), Some("Later type"));
    assert_eq!(later.tags[0].name, "later alpha");
    let replayed = r
        .outbox
        .replay(created.id - 1, created.created_at, 10)
        .await
        .unwrap();
    assert_eq!(replayed[0].context, created.context);
    assert_eq!(replayed[0].created_at, created.created_at);
    assert!(updated.created_at >= created.created_at);
    let progress = DocumentEvent::Processed {
        document_id: doc.id,
    };
    assert!(r
        .outbox
        .append_document(outside, &progress, EventOrigin::User)
        .await
        .is_err());
    assert!(r
        .outbox
        .append_document(
            vault,
            &DocumentEvent::Processed {
                document_id: Id::new_v4()
            },
            EventOrigin::User
        )
        .await
        .is_err());
    assert!(flush_outbox(&r).await.is_empty());
    r.outbox
        .append_document(vault, &progress, EventOrigin::User)
        .await
        .unwrap();
    assert_eq!(
        flush_outbox(&r).await[0].context.as_ref().unwrap().title,
        "Later title"
    );
    r.documents
        .move_to(doc.id, vault, outside, None)
        .await
        .unwrap();
    flush_outbox(&r).await;
    assert!(r
        .outbox
        .append_document(vault, &progress, EventOrigin::User)
        .await
        .is_err());
    assert!(flush_outbox(&r).await.is_empty());
    r.outbox
        .append_document(outside, &progress, EventOrigin::User)
        .await
        .unwrap();
    assert_eq!(
        flush_outbox(&r).await[0].context.as_ref().unwrap().vault_id,
        outside
    );
    r.documents.delete(outside, doc.id).await.unwrap();
    flush_outbox(&r).await;
    assert!(r
        .outbox
        .append_document(outside, &progress, EventOrigin::User)
        .await
        .is_err());
    assert!(flush_outbox(&r).await.is_empty());
    r.documents.request_purge(outside, doc.id).await.unwrap();
    r.documents.finalize_purge(outside, doc.id).await.unwrap();
    let deleted = flush_outbox(&r).await.pop().unwrap();
    assert_eq!(deleted.context.as_ref().unwrap().vault_id, outside);
    assert_eq!(deleted.context.as_ref().unwrap().title, "Later title");
    let DocumentEvent::Deleted {
        snapshot: Some(ref fallback),
        ..
    } = deleted.event.as_ref().unwrap()
    else {
        panic!("deletion snapshot")
    };
    assert!(fallback.document.content.is_none());
    assert!(
        folio_core::events::encode_outbox_payload_with_context(
            deleted.event.as_ref().unwrap(),
            deleted.origin,
            deleted.context.as_deref()
        )
        .len()
            < 8192
    );
}

async fn upload_finalization_contract(db: &Database) {
    let r = db.repositories().await;
    let vault = r
        .vaults
        .create("Upload finalization", None)
        .await
        .unwrap()
        .id;
    flush_outbox(&r).await;
    let pending = r
        .documents
        .create_for_upload(vault, new_document("pending upload"))
        .await
        .unwrap();
    assert!(flush_outbox(&r).await.is_empty());
    let outside = r.vaults.instance_default().await.unwrap();
    assert!(r
        .documents
        .finalize_upload(outside, pending.id, "wrong")
        .await
        .is_err());
    assert!(flush_outbox(&r).await.is_empty());
    exec(db,
        &["CREATE TRIGGER refuse_upload_event BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &["CREATE FUNCTION refuse_upload_event() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
          "CREATE TRIGGER refuse_upload_event BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_upload_event()"],
    ).await;
    assert!(r
        .documents
        .finalize_upload(vault, pending.id, "original")
        .await
        .is_err());
    assert!(r
        .documents
        .find_by_id(vault, pending.id)
        .await
        .unwrap()
        .unwrap()
        .file_path
        .is_none());
    exec(
        db,
        &["DROP TRIGGER refuse_upload_event"],
        &[
            "DROP TRIGGER refuse_upload_event ON outbox_events",
            "DROP FUNCTION refuse_upload_event()",
        ],
    )
    .await;
    let (first, concurrent) = tokio::join!(
        r.documents.finalize_upload(vault, pending.id, "original"),
        r.documents.finalize_upload(vault, pending.id, "original"),
    );
    first.unwrap();
    concurrent.unwrap();
    let created = flush_outbox(&r).await;
    assert_eq!(created.len(), 1);
    assert!(
        matches!(created[0].event, Some(DocumentEvent::Created { document_id }) if document_id == pending.id)
    );
    assert_eq!(
        r.documents
            .find_by_id(vault, pending.id)
            .await
            .unwrap()
            .unwrap()
            .file_path
            .as_deref(),
        Some("original")
    );
    r.outbox
        .prune(Utc::now() + Duration::days(1))
        .await
        .unwrap();
    r.documents
        .finalize_upload(vault, pending.id, "original")
        .await
        .unwrap();
    assert!(
        flush_outbox(&r).await.is_empty(),
        "finalization remains idempotent after outbox pruning"
    );
    let direct = r
        .documents
        .create(vault, new_document("direct creation"))
        .await
        .unwrap();
    let events = flush_outbox(&r).await;
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events[0].event, Some(DocumentEvent::Created { document_id }) if document_id == direct.id)
    );
    r.documents
        .finalize_upload(vault, direct.id, "legacy-original")
        .await
        .unwrap();
    assert!(
        flush_outbox(&r).await.is_empty(),
        "already-announced records never emit another Created"
    );
}

async fn ocr_completion_contract(db: &Database) {
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    flush_outbox(&r).await;
    let doc = r
        .documents
        .create(
            vault,
            CreateDocument {
                content: None,
                ..new_document("OCR transition")
            },
        )
        .await
        .unwrap();
    flush_outbox(&r).await;

    let (first, duplicate) = tokio::join!(
        r.documents.complete_ocr(vault, doc.id, ""),
        r.documents.complete_ocr(vault, doc.id, ""),
    );
    first.unwrap();
    duplicate.unwrap();
    let completed = flush_outbox(&r).await;
    assert_eq!(completed.len(), 1);
    assert!(
        matches!(completed[0].event, Some(DocumentEvent::OcrComplete { document_id }) if document_id == doc.id)
    );
    assert_eq!(completed[0].envelope.as_ref().unwrap().vault_id(), vault);
    let empty = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(empty.content.as_deref(), Some(""));
    assert_eq!(empty.ocr_status, folio_core::models::ProcessingStatus::Done);

    r.documents.complete_ocr(vault, doc.id, "").await.unwrap();
    assert!(flush_outbox(&r).await.is_empty());
    assert_eq!(
        r.documents
            .find_by_id(vault, doc.id)
            .await
            .unwrap()
            .unwrap()
            .updated_at,
        empty.updated_at
    );

    exec(db,
        &["CREATE TRIGGER refuse_ocr_outbox BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &["CREATE FUNCTION refuse_ocr_outbox() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
          "CREATE TRIGGER refuse_ocr_outbox BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_ocr_outbox()"],
    ).await;
    assert!(r
        .documents
        .complete_ocr(vault, doc.id, "must roll back")
        .await
        .is_err());
    exec(
        db,
        &["DROP TRIGGER refuse_ocr_outbox"],
        &[
            "DROP TRIGGER refuse_ocr_outbox ON outbox_events",
            "DROP FUNCTION refuse_ocr_outbox()",
        ],
    )
    .await;
    let retained = r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.content.as_deref(), Some(""));
    assert_eq!(retained.updated_at, empty.updated_at);
    assert!(flush_outbox(&r).await.is_empty());

    r.documents
        .complete_ocr(vault, doc.id, "recognized text")
        .await
        .unwrap();
    assert_eq!(flush_outbox(&r).await.len(), 1);
    assert!(is_not_found(
        r.documents
            .complete_ocr(VaultId::new(), doc.id, "wrong vault")
            .await
    ));
    r.documents.delete(vault, doc.id).await.unwrap();
    flush_outbox(&r).await;
    assert!(is_not_found(
        r.documents.complete_ocr(vault, doc.id, "trashed").await
    ));
    assert!(flush_outbox(&r).await.is_empty());
}

async fn outbox_contract(db: &Database) {
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    flush_outbox(&r).await;

    // Lifecycle changes leave their event in the transaction that made them.
    let doc = r
        .documents
        .create(vault, new_document("outbox"))
        .await
        .unwrap();
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "outbox-tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    r.documents.add_tag(vault, doc.id, tag.id).await.unwrap();
    r.documents.add_tag(vault, doc.id, tag.id).await.unwrap();
    r.documents
        .update(
            vault,
            doc.id,
            UpdateDocument {
                title: Some("renamed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    r.documents
        .set_file_path(vault, doc.id, "a/b.pdf")
        .await
        .unwrap();
    r.documents.set_page_count(vault, doc.id, 2).await.unwrap();
    r.documents.remove_tag(vault, doc.id, tag.id).await.unwrap();
    r.documents.remove_tag(vault, doc.id, tag.id).await.unwrap();
    r.documents
        .set_tags(vault, doc.id, &[tag.id])
        .await
        .unwrap();
    r.workflow_documents
        .remove_tag(vault, doc.id, tag.id)
        .await
        .unwrap();
    assert!(is_not_found(
        r.documents
            .update(vault, Id::new_v4(), UpdateDocument::default())
            .await
    ));
    assert!(is_not_found(r.documents.delete(vault, Id::new_v4()).await));
    r.documents.delete(vault, doc.id).await.unwrap();

    let events = flush_outbox(&r).await;
    let seen: Vec<_> = events
        .iter()
        .filter_map(|e| e.event.as_ref().map(|event| (event.event_type(), e.origin)))
        .collect();
    assert_eq!(
        seen,
        [
            ("created", EventOrigin::User),
            ("updated", EventOrigin::User), // add_tag; the repeat changed nothing
            ("updated", EventOrigin::User), // update
            ("updated", EventOrigin::User), // remove_tag; the repeat changed nothing
            ("updated", EventOrigin::User), // set_tags
            ("updated", EventOrigin::Workflow),
            ("trashed", EventOrigin::User),
        ]
    );
    assert!(events.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(events
        .iter()
        .all(|e| e.envelope.as_ref().and_then(|event| event.document_id()) == Some(doc.id)));
    assert!(matches!(
        events.last().unwrap().event,
        Some(DocumentEvent::Trashed { .. })
    ));
    r.documents.request_purge(vault, doc.id).await.unwrap();
    r.documents.finalize_purge(vault, doc.id).await.unwrap();
    let deleted = flush_outbox(&r).await;
    assert_eq!(
        deleted
            .iter()
            .map(|event| event.event.as_ref().unwrap().event_type())
            .collect::<Vec<_>>(),
        ["trashed", "deleted"]
    );
    let DocumentEvent::Deleted { snapshot, .. } = deleted[1].event.as_ref().unwrap() else {
        panic!("permanent deletion emits a deleted event");
    };
    assert_eq!(snapshot.as_ref().unwrap().document.title, "renamed");
    assert!(snapshot.as_ref().unwrap().tags.is_empty());

    let atomic = r
        .documents
        .create(vault, new_document("metadata"))
        .await
        .unwrap();
    flush_outbox(&r).await;
    r.documents
        .update(
            vault,
            atomic.id,
            UpdateDocument {
                title: Some("metadata changed".into()),
                tag_ids: Some(vec![tag.id, tag.id]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        flush_outbox(&r)
            .await
            .iter()
            .filter(|event| event.event.is_some())
            .count(),
        1,
        "one metadata save emits one compatibility projection"
    );
    r.documents
        .set_tags(vault, atomic.id, &[tag.id])
        .await
        .unwrap();
    r.documents
        .update(
            vault,
            atomic.id,
            UpdateDocument {
                title: Some("metadata changed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        flush_outbox(&r).await.is_empty(),
        "unchanged metadata emits no event"
    );
    assert!(r
        .documents
        .update(
            vault,
            atomic.id,
            UpdateDocument {
                title: Some("must roll back".into()),
                tag_ids: Some(vec![Id::new_v4()]),
                ..Default::default()
            }
        )
        .await
        .is_err());
    assert_eq!(
        r.documents
            .find_by_id(vault, atomic.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "metadata changed"
    );
    assert_eq!(
        r.documents.get_tags(vault, atomic.id).await.unwrap()[0].id,
        tag.id
    );
    assert!(flush_outbox(&r).await.is_empty());

    // A change whose event cannot be written does not happen either.
    let survivor = r
        .documents
        .create(vault, new_document("survivor"))
        .await
        .unwrap();
    flush_outbox(&r).await;
    exec(
        db,
        &["CREATE TRIGGER refuse_outbox BEFORE INSERT ON outbox_events
           BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &[
            "CREATE FUNCTION refuse_outbox() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
            "CREATE TRIGGER refuse_outbox BEFORE INSERT ON outbox_events
             FOR EACH ROW EXECUTE FUNCTION refuse_outbox()",
        ],
    )
    .await;
    assert!(r
        .documents
        .create(vault, new_document("doomed"))
        .await
        .is_err());
    assert!(r
        .documents
        .update(
            vault,
            survivor.id,
            UpdateDocument {
                title: Some("doomed".into()),
                ..Default::default()
            },
        )
        .await
        .is_err());
    assert!(r.documents.delete(vault, survivor.id).await.is_err());
    exec(
        db,
        &["DROP TRIGGER refuse_outbox"],
        &[
            "DROP TRIGGER refuse_outbox ON outbox_events",
            "DROP FUNCTION refuse_outbox()",
        ],
    )
    .await;
    let titles: Vec<_> = r
        .documents
        .list(vault, None)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect();
    assert!(!titles.contains(&"doomed".to_string()));
    let kept = r
        .documents
        .find_by_id(vault, survivor.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.title, "survivor", "the update was rolled back");
    assert!(
        flush_outbox(&r).await.is_empty(),
        "no event for a rolled back change"
    );

    // Written, never dispatched, process gone: the restart delivers it exactly once.
    let orphan = r
        .documents
        .create(vault, new_document("orphan"))
        .await
        .unwrap();
    drop(r);
    let restarted = db.repositories().await;
    let recorder = Arc::new(Recorder::default());
    let after_restart = dispatcher(&restarted, recorder.clone(), 10);
    assert_eq!(after_restart.dispatch_once().await.unwrap(), 1);
    assert_eq!(after_restart.dispatch_once().await.unwrap(), 0);
    assert_eq!(recorder.ids().len(), 1);
    let replayed = restarted
        .outbox
        .replay(recorder.ids()[0] - 1, orphan.created_at, 10)
        .await
        .unwrap();
    assert_eq!(replayed.len(), 1);
    assert!(
        matches!(replayed[0].event, Some(DocumentEvent::Created { document_id }) if document_id == orphan.id)
    );

    // Claimed but never finished: after the lease it is handed out again, then done for good.
    let r = restarted;
    r.documents
        .create(vault, new_document("interrupted"))
        .await
        .unwrap();
    let first = r.outbox.claim(10, Duration::seconds(-1), 10).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].attempts, 1);
    let recorder = Arc::new(Recorder::default());
    let dispatch = dispatcher(&r, recorder.clone(), 10);
    assert_eq!(dispatch.dispatch_once().await.unwrap(), 1);
    assert_eq!(dispatch.dispatch_once().await.unwrap(), 0);
    assert_eq!(recorder.ids(), [first[0].id]);

    // Several dispatchers at once never deliver an event twice.
    flush_outbox(&r).await;
    for i in 0..40 {
        r.documents
            .create(vault, new_document(&format!("burst-{i}")))
            .await
            .unwrap();
    }
    let recorder = Arc::new(Recorder::default());
    let mut dispatchers = Vec::new();
    for _ in 0..4 {
        let dispatch = dispatcher(&db.repositories().await, recorder.clone(), 5);
        dispatchers.push(tokio::spawn(async move {
            while dispatch.dispatch_once().await.unwrap() > 0 {}
        }));
    }
    for handle in dispatchers {
        handle.await.unwrap();
    }
    let mut delivered = recorder.ids();
    delivered.sort_unstable();
    let total = delivered.len();
    delivered.dedup();
    assert_eq!(total, 40);
    assert_eq!(delivered.len(), 40, "no event was delivered twice");

    // Failures back off, and an event stops being retried once it used its attempts.
    r.documents
        .create(vault, new_document("flaky"))
        .await
        .unwrap();
    let past = || Utc::now() - Duration::seconds(1);
    let first = r.outbox.claim(10, lease(), 3).await.unwrap().remove(0);
    assert!(r.outbox.claim(10, lease(), 3).await.unwrap().is_empty());
    r.outbox
        .release(first.id, "boom", Utc::now() + Duration::hours(1))
        .await
        .unwrap();
    assert!(r.outbox.claim(10, lease(), 3).await.unwrap().is_empty());
    r.outbox.release(first.id, "boom", past()).await.unwrap();
    let second = r.outbox.claim(10, lease(), 3).await.unwrap().remove(0);
    assert_eq!((second.id, second.attempts), (first.id, 2));
    r.outbox.release(first.id, "boom", past()).await.unwrap();
    let third = r.outbox.claim(10, lease(), 3).await.unwrap().remove(0);
    assert_eq!(third.attempts, 3);
    r.outbox.release(first.id, "boom", past()).await.unwrap();
    assert!(r.outbox.claim(10, lease(), 3).await.unwrap().is_empty());
    r.outbox.mark_dispatched(first.id).await.unwrap();

    // Replay includes pending events so consumer failures cannot hide live updates.
    for title in ["replay-1", "replay-2", "replay-3"] {
        r.documents
            .create(vault, new_document(title))
            .await
            .unwrap();
    }
    let ids: Vec<_> = r
        .outbox
        .claim(10, lease(), 10)
        .await
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids.len(), 3);
    r.outbox.mark_dispatched(ids[0]).await.unwrap();
    r.outbox.mark_dispatched(ids[1]).await.unwrap();
    let hour_ago = Utc::now() - Duration::hours(1);
    let replay_ids = |after: i64, since| {
        let outbox = r.outbox.clone();
        async move {
            outbox
                .replay(after, since, 100)
                .await
                .unwrap()
                .into_iter()
                .map(|e| e.id)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        replay_ids(ids[0] - 1, hour_ago).await,
        [ids[0], ids[1], ids[2]]
    );
    assert_eq!(replay_ids(ids[0], hour_ago).await, [ids[1], ids[2]]);
    assert!(replay_ids(0, Utc::now() + Duration::hours(1))
        .await
        .is_empty());
    let limited = r.outbox.replay(ids[0] - 1, hour_ago, 1).await.unwrap();
    assert_eq!(limited.len(), 1);

    assert!(r.outbox.prune(hour_ago).await.unwrap() == 0);
    assert!(
        r.outbox
            .prune(Utc::now() + Duration::hours(1))
            .await
            .unwrap()
            >= 2
    );
    assert_eq!(replay_ids(ids[0] - 1, hour_ago).await, [ids[2]]);
    r.outbox.mark_dispatched(ids[2]).await.unwrap();

    r.documents
        .create(vault, new_document("delayed-first"))
        .await
        .unwrap();
    r.documents
        .create(vault, new_document("delivered-second"))
        .await
        .unwrap();
    let batch = r.outbox.claim(10, lease(), 10).await.unwrap();
    let earlier = batch[0].id;
    let later = batch[1].id;
    r.outbox.mark_dispatched(later).await.unwrap();
    r.outbox.mark_dispatched(earlier).await.unwrap();
    assert_eq!(
        replay_ids(earlier, hour_ago).await,
        [later],
        "a later broadcast is replayed even if its consumers finish first"
    );
    assert_eq!(
        replay_ids(later, hour_ago).await,
        [earlier],
        "reconnect must recover a lower id delivered after the acknowledged event"
    );
}

async fn metadata_workflow_dispatch_contract(db: &Database) {
    use folio_core::services::WorkflowEngine;
    use folio_infra::{ObjectStoreDocumentStore, StorageFileMetadata};

    let r = db.repositories().await;
    let vault = r.vaults.create("Metadata workflow", None).await.unwrap().id;
    let document = r
        .documents
        .create(vault, new_document("before"))
        .await
        .unwrap();
    let workflow = r
        .workflows
        .create(
            vault,
            workflow_input("title event", WorkflowTrigger::DocumentTitleChanged),
        )
        .await
        .unwrap();
    r.documents
        .update(
            vault,
            document.id,
            UpdateDocument {
                title: Some("after".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let event = r
        .outbox
        .replay(0, Utc::now() - Duration::days(1), 10)
        .await
        .unwrap()
        .into_iter()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.event_type() == "document.title_changed"
                    && envelope.document_id() == Some(document.id)
            })
        })
        .unwrap();
    assert!(event.event.is_none());
    let engine = WorkflowEngine::new(
        r.workflows.clone(),
        r.documents.clone(),
        r.correspondents.clone(),
        r.document_types.clone(),
        Arc::new(StorageFileMetadata::new(Arc::new(
            ObjectStoreDocumentStore::in_memory(),
        ))),
        "http://folio.test",
    );
    let runs = engine.handle(&event).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
    assert!(engine.handle(&event).await.unwrap().is_empty());
    let stored = r.workflows.find_run(runs[0].id).await.unwrap().unwrap();
    assert_eq!(stored.event, WorkflowTrigger::DocumentTitleChanged);
    let body: serde_json::Value = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(body["document"]["title"], "after");
    assert_eq!(
        body["source_event"]["event_id"],
        event.envelope.as_ref().unwrap().event_id().to_string()
    );
}

async fn run_atomicity_contract(db: &Database) {
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let workflow = r
        .workflows
        .create(
            vault,
            workflow_input("atomic", WorkflowTrigger::DocumentUpdated),
        )
        .await
        .unwrap();
    let document = Id::new_v4();
    let run_for = |event: i64| {
        WorkflowRun::new(
            workflow.vault_id,
            workflow.id,
            WorkflowTrigger::DocumentUpdated,
            Some(document),
            "{}".into(),
        )
        .with_actions(workflow.actions.clone())
        .with_source_event(event)
    };
    let queued = r.jobs.pending_count().await.unwrap();

    // A run and its job appear together, once per outbox event.
    let run = run_for(42);
    assert!(r.workflows.create_run(&run).await.unwrap());
    assert_eq!(r.jobs.pending_count().await.unwrap(), queued + 1);
    assert!(!r.workflows.create_run(&run_for(42)).await.unwrap());
    assert_eq!(r.jobs.pending_count().await.unwrap(), queued + 1);
    assert!(r.workflows.create_run(&run_for(43)).await.unwrap());
    assert_eq!(r.jobs.pending_count().await.unwrap(), queued + 2);

    let stored = r.workflows.find_run(run.id).await.unwrap().unwrap();
    assert_eq!(stored.actions, workflow.actions);
    assert_eq!(stored.source_event_id, Some(42));

    // When the job cannot be queued the run is not created either.
    exec(
        db,
        &["CREATE TRIGGER refuse_jobs BEFORE INSERT ON jobs
           BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &[
            "CREATE FUNCTION refuse_jobs() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
            "CREATE TRIGGER refuse_jobs BEFORE INSERT ON jobs
             FOR EACH ROW EXECUTE FUNCTION refuse_jobs()",
        ],
    )
    .await;
    let doomed = run_for(44);
    assert!(r.workflows.create_run(&doomed).await.is_err());
    exec(
        db,
        &["DROP TRIGGER refuse_jobs"],
        &[
            "DROP TRIGGER refuse_jobs ON jobs",
            "DROP FUNCTION refuse_jobs()",
        ],
    )
    .await;
    assert!(r.workflows.find_run(doomed.id).await.unwrap().is_none());
    assert_eq!(r.jobs.pending_count().await.unwrap(), queued + 2);
    assert!(r.workflows.create_run(&run_for(44)).await.unwrap());
}

async fn fresh_database_uses_config_and_preserves_saved_overrides(db: &Database) {
    let repos = db.repositories().await;
    let config = folio_infra::config::Config::load_with(
        "/nonexistent/folio-config.toml",
        [
            ("FOLIO_OCR__PROVIDER".to_string(), "none".to_string()),
            (
                "FOLIO_PROCESSING__AUTO_OCR".to_string(),
                "false".to_string(),
            ),
        ],
    )
    .unwrap();
    let pipeline = folio_infra::PipelineContext::from_config(repos.settings.clone(), &config);
    assert!(repos.settings.get("ocr.provider").await.unwrap().is_none());
    assert!(repos
        .settings
        .get("processing.auto_ocr")
        .await
        .unwrap()
        .is_none());
    assert_eq!(pipeline.settings().await.ocr.provider, "none");
    assert!(!pipeline.settings().await.processing.auto_ocr);
    repos
        .settings
        .set("ocr.provider", "tesseract")
        .await
        .unwrap();
    repos
        .settings
        .set("processing.auto_ocr", "true")
        .await
        .unwrap();
    db.migrate().await.unwrap();
    pipeline.invalidate().await;
    assert_eq!(
        repos
            .settings
            .get("ocr.provider")
            .await
            .unwrap()
            .unwrap()
            .value,
        "tesseract"
    );
    assert!(pipeline.settings().await.processing.auto_ocr);
}

async fn pagination_contract(db: &Database, r: &Repositories) {
    use folio_core::models::{DocumentPageQuery, DocumentSort};
    let vault = r.vaults.create("Pagination", None).await.unwrap().id;
    let first = r
        .documents
        .create(vault, new_document("Same"))
        .await
        .unwrap();
    let second = r
        .documents
        .create(vault, new_document("Same"))
        .await
        .unwrap();
    let third = r
        .documents
        .create(vault, new_document("Zebra"))
        .await
        .unwrap();
    let at = "2026-01-01T00:00:00+00:00";
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE documents SET created_at = ? WHERE vault_id = ?")
                .bind(at)
                .bind(vault.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE documents SET created_at = $1 WHERE vault_id = $2")
                .bind(at.parse::<chrono::DateTime<Utc>>().unwrap())
                .bind(vault.0)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    let query = DocumentPageQuery {
        limit: 2,
        sort: DocumentSort::Title,
        ..Default::default()
    };
    let page = r.documents.page(vault, query.clone()).await.unwrap();
    let mut expected = vec![first.id, second.id];
    expected.sort();
    assert_eq!(
        page.documents.iter().map(|d| d.id).collect::<Vec<_>>(),
        expected
    );
    let cursor = page.next_cursor.unwrap();
    let next = DocumentPageQuery {
        cursor: Some(cursor.clone()),
        ..query.clone()
    };
    let page = r.documents.page(vault, next.clone()).await.unwrap();
    assert_eq!(
        page.documents.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![third.id]
    );
    assert!(page.next_cursor.is_none());
    let other = r.vaults.instance_default().await.unwrap();
    assert!(matches!(
        r.documents.page(other, next.clone()).await,
        Err(Error::Validation(_))
    ));
    let changed_sort = DocumentPageQuery {
        sort: DocumentSort::Newest,
        ..next.clone()
    };
    assert!(matches!(
        r.documents.page(vault, changed_sort).await,
        Err(Error::Validation(_))
    ));
    let changed_filter = DocumentPageQuery {
        filter: DocumentFilter {
            search_query: Some("Same".into()),
            ..Default::default()
        },
        ..next
    };
    assert!(matches!(
        r.documents.page(vault, changed_filter).await,
        Err(Error::Validation(_))
    ));
    let filtered = DocumentPageQuery {
        filter: DocumentFilter {
            search_query: Some("Zebra".into()),
            ..Default::default()
        },
        ..query.clone()
    };
    let page = r.documents.page(vault, filtered).await.unwrap();
    assert_eq!(page.documents.len(), 1);
    assert_eq!(page.documents[0].id, third.id);
    assert!(page.next_cursor.is_none());
    let chronological = DocumentPageQuery {
        limit: 2,
        ..Default::default()
    };
    let page = r
        .documents
        .page(vault, chronological.clone())
        .await
        .unwrap();
    let mut descending = [first.id, second.id, third.id];
    descending.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        page.documents.iter().map(|d| d.id).collect::<Vec<_>>(),
        descending[..2]
    );
    let page = r
        .documents
        .page(
            vault,
            DocumentPageQuery {
                cursor: page.next_cursor,
                ..chronological
            },
        )
        .await
        .unwrap();
    assert_eq!(page.documents[0].id, descending[2]);
    assert!(page.next_cursor.is_none());
    assert!(matches!(
        r.documents
            .page(
                vault,
                DocumentPageQuery {
                    limit: 0,
                    ..query.clone()
                }
            )
            .await,
        Err(Error::Validation(_))
    ));
    assert!(matches!(
        r.documents
            .page(
                vault,
                DocumentPageQuery {
                    limit: 101,
                    ..query
                }
            )
            .await,
        Err(Error::Validation(_))
    ));
    let oldest = DocumentPageQuery {
        limit: 2,
        sort: DocumentSort::Oldest,
        ..Default::default()
    };
    let page = r.documents.page(vault, oldest.clone()).await.unwrap();
    descending.reverse();
    assert_eq!(
        page.documents.iter().map(|d| d.id).collect::<Vec<_>>(),
        descending[..2]
    );
    let page = r
        .documents
        .page(
            vault,
            DocumentPageQuery {
                cursor: page.next_cursor,
                ..oldest
            },
        )
        .await
        .unwrap();
    assert_eq!(page.documents[0].id, descending[2]);
    assert!(page.next_cursor.is_none());
    assert_eq!(r.documents.list(vault, None).await.unwrap().len(), 3);
}

async fn batch_tags_contract(r: &Repositories) {
    let vault = r.vaults.create("Tag batch", None).await.unwrap().id;
    let outside = r.vaults.instance_default().await.unwrap();
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "Batch tag".into(),
                color: Some("#123456".into()),
            },
        )
        .await
        .unwrap();
    let tagged = r
        .documents
        .create(vault, new_document("Tagged"))
        .await
        .unwrap();
    let untagged = r
        .documents
        .create(vault, new_document("Untagged"))
        .await
        .unwrap();
    let foreign = r
        .documents
        .create(outside, new_document("Foreign"))
        .await
        .unwrap();
    r.documents.add_tag(vault, tagged.id, tag.id).await.unwrap();
    let foreign_tag = r
        .tags
        .create(
            outside,
            CreateTag {
                name: "Foreign batch tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    r.documents
        .add_tag(outside, foreign.id, foreign_tag.id)
        .await
        .unwrap();
    let result = r
        .documents
        .tags_for_documents(
            vault,
            &[tagged.id, tagged.id, untagged.id, foreign.id, Id::new_v4()],
        )
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[&tagged.id].len(), 1);
    assert_eq!(result[&tagged.id][0].id, tag.id);
    assert_eq!(result[&tagged.id][0].color, tag.color);
    assert!(r
        .documents
        .tags_for_documents(vault, &[])
        .await
        .unwrap()
        .is_empty());
    assert!(r
        .documents
        .tags_for_documents(outside, &[tagged.id])
        .await
        .unwrap()
        .is_empty());
}

async fn trash_contract(db: &Database, r: &Repositories) {
    let vault = r.vaults.create("Trash contract", None).await.unwrap().id;
    let other = r
        .vaults
        .create("Other trash contract", None)
        .await
        .unwrap()
        .id;
    let tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "retained".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let document = r
        .documents
        .create(
            vault,
            CreateDocument {
                tag_ids: vec![tag.id],
                ..new_document("Trash contract invoice")
            },
        )
        .await
        .unwrap();
    r.documents
        .set_file_path(
            vault,
            document.id,
            &folio_core::ports::keys::original(document.id),
        )
        .await
        .unwrap();
    r.search
        .store_chunks(
            vault,
            document.id,
            "trash-model",
            &[EmbeddedChunk {
                index: 0,
                text: "preserved text".into(),
                embedding: vec![1.0, 0.0],
            }],
        )
        .await
        .unwrap();
    assert!(matches!(
        r.documents.request_purge(vault, document.id).await,
        Err(Error::Conflict(_))
    ));

    let persisted = r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap();
    r.documents.delete(vault, document.id).await.unwrap();
    r.documents.delete(vault, document.id).await.unwrap();

    assert!(r.documents.locate(document.id).await.unwrap().is_none());
    assert_eq!(
        r.documents.locate_retained(document.id).await.unwrap(),
        Some(vault)
    );
    assert!(r
        .documents
        .find_by_id(vault, document.id)
        .await
        .unwrap()
        .is_none());
    assert!(r.documents.list(vault, None).await.unwrap().is_empty());
    assert!(!r
        .documents
        .list_all()
        .await
        .unwrap()
        .iter()
        .any(|doc| doc.id == document.id));
    assert!(!r
        .documents
        .list_pending(folio_core::models::ProcessingStage::Embed)
        .await
        .unwrap()
        .iter()
        .any(|doc| doc.id == document.id));
    assert!(!r
        .documents
        .reset_embedding_status()
        .await
        .unwrap()
        .contains(&document.id));
    assert!(r.documents.list_ids(vault, None).await.unwrap().is_empty());
    assert!(r
        .documents
        .page(vault, Default::default())
        .await
        .unwrap()
        .documents
        .is_empty());
    assert!(r
        .documents
        .get_tags(vault, document.id)
        .await
        .unwrap()
        .is_empty());
    assert!(r
        .documents
        .tags_for_documents(vault, &[document.id])
        .await
        .unwrap()
        .is_empty());
    assert!(r
        .search
        .search(vault, "trash-model", &[1.0, 0.0], 10)
        .await
        .unwrap()
        .is_empty());
    assert!(is_not_found(
        r.documents
            .update(
                vault,
                document.id,
                UpdateDocument {
                    title: Some("changed".into()),
                    ..Default::default()
                }
            )
            .await
    ));
    assert!(is_not_found(
        r.documents.add_tag(vault, document.id, tag.id).await
    ));
    assert!(r
        .documents
        .find_trashed_by_id(other, document.id)
        .await
        .unwrap()
        .is_none());
    assert!(r.documents.list_trash(other).await.unwrap().is_empty());
    assert!(is_not_found(r.documents.restore(other, document.id).await));
    assert!(is_not_found(
        r.documents.request_purge(other, document.id).await
    ));
    assert_eq!(
        r.documents.locate_trashed(document.id).await.unwrap(),
        Some(vault)
    );
    assert!(r.documents.list_trash(vault).await.unwrap()[0]
        .deleted_at
        .is_some());
    assert!(matches!(
        r.vaults.delete(vault).await,
        Err(Error::Conflict(_))
    ));

    let restored = r.documents.restore(vault, document.id).await.unwrap();
    assert_eq!(restored.id, document.id);
    assert_eq!(restored.content, document.content);
    assert_eq!(restored.created_at, persisted.created_at);
    assert!(restored.deleted_at.is_none());
    assert_eq!(
        r.documents.locate_retained(document.id).await.unwrap(),
        Some(vault)
    );
    assert_eq!(
        r.documents.get_tags(vault, document.id).await.unwrap()[0].id,
        tag.id
    );
    assert_eq!(
        r.search
            .search(vault, "trash-model", &[1.0, 0.0], 10)
            .await
            .unwrap()[0]
            .document_id,
        document.id
    );

    r.documents.delete(vault, document.id).await.unwrap();
    exec(db,
        &["CREATE TRIGGER refuse_trash_jobs BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT, 'refused'); END"],
        &["CREATE FUNCTION refuse_trash_jobs() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql",
          "CREATE TRIGGER refuse_trash_jobs BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION refuse_trash_jobs()"]
    ).await;
    assert!(r.documents.request_purge(vault, document.id).await.is_err());
    assert!(r
        .documents
        .find_trashed_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap()
        .purge_requested_at
        .is_none());
    exec(
        db,
        &["DROP TRIGGER refuse_trash_jobs"],
        &[
            "DROP TRIGGER refuse_trash_jobs ON jobs",
            "DROP FUNCTION refuse_trash_jobs()",
        ],
    )
    .await;
    r.documents.request_purge(vault, document.id).await.unwrap();
    r.documents.request_purge(vault, document.id).await.unwrap();
    assert!(r
        .documents
        .find_trashed_by_id(vault, document.id)
        .await
        .unwrap()
        .unwrap()
        .purge_requested_at
        .is_some());
    assert!(matches!(
        r.documents.restore(vault, document.id).await,
        Err(Error::Conflict(_))
    ));
    let jobs = r
        .jobs
        .find_active_jobs_for_document(document.id)
        .await
        .unwrap();
    assert_eq!(
        jobs.iter()
            .filter(|job| matches!(job.job_type, JobType::PurgeDocument { .. }))
            .count(),
        1
    );
    r.documents
        .finalize_purge(vault, document.id)
        .await
        .unwrap();
    r.documents
        .finalize_purge(vault, document.id)
        .await
        .unwrap();
    assert!(r
        .documents
        .locate_trashed(document.id)
        .await
        .unwrap()
        .is_none());
    assert!(r.documents.list_trash(vault).await.unwrap().is_empty());
    assert!(r
        .documents
        .locate_retained(document.id)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .search
        .search(vault, "trash-model", &[1.0, 0.0], 10)
        .await
        .unwrap()
        .is_empty());
}

async fn run_contract(db: &Database) {
    let (first, second) = tokio::join!(db.migrate(), db.migrate());
    first.unwrap();
    second.unwrap();
    fresh_database_uses_config_and_preserves_saved_overrides(db).await;
    let repos = db.repositories().await;

    tags_contract(&repos).await;
    correspondents_contract(&repos).await;
    document_types_contract(&repos).await;
    documents_contract(&repos).await;
    document_filter_contract(&repos).await;
    pagination_contract(db, &repos).await;
    batch_tags_contract(&repos).await;
    settings_contract(&repos).await;
    search_contract(
        db,
        repos.vaults.instance_default().await.unwrap(),
        repos.documents.as_ref(),
        repos.search.as_ref(),
    )
    .await;
    // Before the workflow contracts, which leave queued run jobs behind
    jobs_contract(db, &repos).await;
    processing_recovery_contract(&repos).await;
    concurrent_dequeue_contract(&repos).await;
    workflows_contract(&repos).await;
    ingestion_contract(&repos).await;
    event_context_contract(db).await;
    upload_finalization_contract(db).await;
    outbox_contract(db).await;
    ocr_completion_contract(db).await;
    run_atomicity_contract(db).await;
    auth_contract(&repos).await;
    mfa_contract(&repos).await;
    trash_contract(db, &repos).await;
    metadata_workflow_dispatch_contract(db).await;
}

#[tokio::test]
async fn sqlite_satisfies_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("folio.db").display());
    let db = Database::connect(&url).await.unwrap();
    run_contract(&db).await;
}

#[tokio::test]
async fn deleting_a_document_retains_its_record_until_explicit_purge() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let doc = r
        .documents
        .create(vault, new_document("retained"))
        .await
        .unwrap();

    r.documents.delete(vault, doc.id).await.unwrap();

    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE id = ?")
        .bind(doc.id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "trash must preserve the record and its metadata");
    assert!(r
        .documents
        .find_by_id(vault, doc.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn interrupted_fresh_initialization_removes_seeded_overrides() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    sqlx::migrate!("../../migrations/sqlite")
        .run(pool)
        .await
        .unwrap();
    db.migrate().await.unwrap();
    fresh_database_uses_config_and_preserves_saved_overrides(&db).await;
}

#[tokio::test]
async fn existing_database_preserves_seeded_overrides() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let Database::Sqlite(pool) = &db else {
        unreachable!()
    };
    let mut migrator = sqlx::migrate!("../../migrations/sqlite");
    migrator
        .migrations
        .to_mut()
        .retain(|migration| migration.version != 0);
    migrator.run(pool).await.unwrap();
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    assert_eq!(
        repos
            .settings
            .get("ocr.provider")
            .await
            .unwrap()
            .unwrap()
            .value,
        "tesseract"
    );
}

#[tokio::test]
async fn memory_database_survives_initialization() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let (first, second) = tokio::join!(db.migrate(), db.migrate());
    first.unwrap();
    second.unwrap();
    let repos = db.repositories().await;
    repos.settings.set("test.key", "value").await.unwrap();
    db.migrate().await.unwrap();
    assert_eq!(
        repos.settings.get("test.key").await.unwrap().unwrap().value,
        "value"
    );
}

async fn postgres_initialization_recovery(pool: &sqlx::PgPool, host: &str, port: u16) {
    for existing in [false, true] {
        let name = if existing {
            "existing_install"
        } else {
            "interrupted_install"
        };
        sqlx::query(if existing {
            "CREATE DATABASE existing_install"
        } else {
            "CREATE DATABASE interrupted_install"
        })
        .execute(pool)
        .await
        .unwrap();
        let fixture = Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/{name}"
        ))
        .await
        .unwrap();
        let Database::Postgres(fixture_pool) = &fixture else {
            unreachable!()
        };
        let mut migrator = sqlx::migrate!("../../migrations/postgres");
        if existing {
            migrator
                .migrations
                .to_mut()
                .retain(|migration| migration.version != 0);
        }
        migrator.run(fixture_pool).await.unwrap();
        fixture.migrate().await.unwrap();
        if existing {
            assert_eq!(
                fixture
                    .repositories()
                    .await
                    .settings
                    .get("ocr.provider")
                    .await
                    .unwrap()
                    .unwrap()
                    .value,
                "tesseract"
            );
        } else {
            fresh_database_uses_config_and_preserves_saved_overrides(&fixture).await;
        }
        fixture_pool.close().await;
    }
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
    let Database::Postgres(pool) = &db else {
        unreachable!()
    };
    postgres_initialization_recovery(pool, &host.to_string(), port).await;
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
    search_contract(
        &db,
        repos.vaults.instance_default().await.unwrap(),
        repos.documents.as_ref(),
        &fallback,
    )
    .await;
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
    let Database::Postgres(pool) = &db else {
        unreachable!()
    };
    postgres_initialization_recovery(pool, &host.to_string(), port).await;
    run_contract(&db).await;

    let Database::Postgres(pool) = &db else {
        panic!("expected a PostgreSQL database");
    };
    assert!(!PostgresSearchService::new(pool.clone())
        .await
        .uses_pgvector());
}

#[tokio::test]
async fn postgres_scoped_capture_after_lock_wait_sees_one_complete_metadata_version() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL capture lock wait", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let Database::Postgres(pool) = &db else {
        unreachable!()
    };
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let old_tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "before".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let new_tag = r
        .tags
        .create(
            vault,
            CreateTag {
                name: "after".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let doc = r
        .documents
        .create(
            vault,
            CreateDocument {
                tag_ids: vec![old_tag.id],
                ..new_document("before")
            },
        )
        .await
        .unwrap();
    let mut writer = pool.begin().await.unwrap();
    let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    sqlx::query("UPDATE documents SET title = 'after' WHERE id = $1")
        .bind(doc.id)
        .execute(&mut *writer)
        .await
        .unwrap();
    sqlx::query("DELETE FROM document_tags WHERE document_id = $1")
        .bind(doc.id)
        .execute(&mut *writer)
        .await
        .unwrap();
    sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES ($1, $2)")
        .bind(doc.id)
        .bind(new_tag.id)
        .execute(&mut *writer)
        .await
        .unwrap();
    let outbox = r.outbox.clone();
    let append = tokio::spawn(async move {
        outbox
            .append_document(
                vault,
                &DocumentEvent::Processed {
                    document_id: doc.id,
                },
                EventOrigin::User,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                SELECT 1 FROM pg_stat_activity WHERE datname = current_database()
                AND wait_event_type = 'Lock' AND $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(writer_pid)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scoped append must wait for the writer's document lock");
    writer.commit().await.unwrap();

    let id = tokio::time::timeout(std::time::Duration::from_secs(5), append)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let events = r
        .outbox
        .replay(id - 1, Utc::now() - Duration::days(1), 10)
        .await
        .unwrap();
    let snapshot = events[0].context.as_ref().unwrap();
    assert_eq!(snapshot.title, "after");
    assert_eq!(
        snapshot.tags,
        vec![folio_core::models::NamedRef {
            id: new_tag.id,
            name: "after".into()
        }]
    );
}
