mod common;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, TimeDelta, Utc};
use folio_core::error::Error;
use folio_core::models::{
    CreateImportRun, FolioState, ImportAction, ImportApplied, ImportItemOutcome, ImportItemStatus,
    ImportRun, ImportRunKind, ImportRunStatus, ImportTrigger, LibrarySample, NewImportItem,
    NewImportRun, PaperlessLedgerEntry, PaperlessSyncState, RunTransition, TaxonomyKind,
    TaxonomyMapping, VaultId,
};
use folio_infra::database::Repositories;
use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
use uuid::Uuid;

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn new_run(vault: VaultId) -> NewImportRun {
    NewImportRun {
        vault_id: vault,
        kind: ImportRunKind::PaperlessExport,
        trigger: ImportTrigger::Manual,
        source_id: None,
        source_path: Some("/imports/export".into()),
        dry_run: false,
        params: serde_json::json!({"default_currency": "EUR"}),
        created_by: None,
    }
}

fn item(paperless_id: i64, action: ImportAction) -> NewImportItem {
    NewImportItem {
        paperless_id,
        action,
        document_id: Some(Uuid::new_v4()),
        title: Some(format!("Document {paperless_id}")),
        status: ImportItemStatus::Pending,
        reason: None,
        payload: Some(serde_json::json!({"id": paperless_id})),
    }
}

async fn created(r: &Repositories, run: NewImportRun) -> ImportRun {
    match r.import_runs.create(run, Utc::now()).await.unwrap() {
        CreateImportRun::Created(run) => *run,
        other => panic!("expected a new run, got {other:?}"),
    }
}

async fn runs_contract(r: &Repositories, vault: VaultId, other_vault: VaultId) {
    let first = created(r, new_run(vault)).await;
    assert_eq!(first.status, ImportRunStatus::Queued);
    assert_eq!(first.params, serde_json::json!({"default_currency": "EUR"}));
    assert_eq!(
        r.import_runs.get(vault, first.id).await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        r.import_runs.get(other_vault, first.id).await.unwrap(),
        None
    );

    assert_eq!(
        r.import_runs
            .create(new_run(vault), Utc::now())
            .await
            .unwrap(),
        CreateImportRun::Conflict {
            active_run: first.id
        },
        "a second active run in a vault is a conflict"
    );
    let elsewhere = created(r, new_run(other_vault)).await;
    assert_ne!(elsewhere.id, first.id);

    let now = at("2026-11-07T10:00:00Z");
    assert_eq!(
        r.import_runs
            .transition(
                first.id,
                &[ImportRunStatus::Running],
                ImportRunStatus::Completed,
                None,
                now
            )
            .await
            .unwrap(),
        RunTransition::Skipped
    );
    assert_eq!(
        r.import_runs
            .transition(
                first.id,
                &[ImportRunStatus::Queued],
                ImportRunStatus::Running,
                None,
                now
            )
            .await
            .unwrap(),
        RunTransition::Applied
    );
    r.import_runs
        .set_plan_complete(first.id, true)
        .await
        .unwrap();
    r.import_runs
        .set_cursor_after(first.id, Some(at("2026-11-01T08:30:00.123456Z")))
        .await
        .unwrap();
    r.import_runs
        .set_confirm_trash(first.id, true)
        .await
        .unwrap();
    r.import_runs
        .set_summary(first.id, serde_json::json!({"created": 2}))
        .await
        .unwrap();
    assert_eq!(
        r.import_runs
            .increment_driver_restarts(first.id)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        r.import_runs
            .increment_driver_restarts(first.id)
            .await
            .unwrap(),
        Some(2)
    );
    assert!(r.import_runs.request_cancel(first.id).await.unwrap());
    let running = r.import_runs.get(vault, first.id).await.unwrap().unwrap();
    assert_eq!(running.status, ImportRunStatus::Running);
    assert_eq!(running.started_at, Some(now));
    assert!(running.plan_complete && running.confirm_trash && running.cancel_requested);
    assert_eq!(
        running.cursor_after,
        Some(at("2026-11-01T08:30:00.123456Z"))
    );
    assert_eq!(running.summary, Some(serde_json::json!({"created": 2})));
    assert_eq!(running.driver_restarts, 2);

    let later = now + TimeDelta::minutes(5);
    assert_eq!(
        r.import_runs
            .transition(
                first.id,
                &[ImportRunStatus::Running],
                ImportRunStatus::Failed,
                Some("library_mismatch".into()),
                later
            )
            .await
            .unwrap(),
        RunTransition::Applied
    );
    assert!(!r.import_runs.request_cancel(first.id).await.unwrap());
    assert_eq!(
        r.import_runs
            .increment_driver_restarts(first.id)
            .await
            .unwrap(),
        None,
        "only an active run's driver restarts"
    );
    let failed = r.import_runs.get(vault, first.id).await.unwrap().unwrap();
    assert_eq!(failed.finished_at, Some(later));
    assert_eq!(failed.error.as_deref(), Some("library_mismatch"));

    let second = created(r, new_run(vault)).await;
    let listed: Vec<_> = r
        .import_runs
        .list(vault, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|run| run.id)
        .collect();
    assert_eq!(listed, vec![second.id, first.id], "newest first");

    assert_eq!(
        r.import_runs
            .transition(
                first.id,
                &[ImportRunStatus::Failed],
                ImportRunStatus::Queued,
                None,
                later
            )
            .await
            .unwrap(),
        RunTransition::Conflict {
            active_run: second.id
        },
        "a retry can't re-enter queued while another run is active"
    );
    assert_eq!(
        r.import_runs
            .get(vault, first.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportRunStatus::Failed
    );
}

async fn items_contract(r: &Repositories, vault: VaultId) {
    let run = created(r, new_run(vault)).await;
    let now = at("2026-11-07T10:00:00Z");
    let items: Vec<_> = (1..=20)
        .map(|id| {
            item(
                id,
                if id % 4 == 0 {
                    ImportAction::Update
                } else {
                    ImportAction::Create
                },
            )
        })
        .collect();
    assert_eq!(
        r.import_items
            .insert_items_ignore_existing(run.id, &items, now)
            .await
            .unwrap(),
        20
    );
    let again = vec![
        item(20, ImportAction::Create),
        item(21, ImportAction::Trash),
    ];
    assert_eq!(
        r.import_items
            .insert_items_ignore_existing(run.id, &again, now)
            .await
            .unwrap(),
        1,
        "existing items are left alone"
    );
    let stored = r
        .import_items
        .list_items(run.id, None, Some(19), 10)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].paperless_id, 20);
    assert_eq!(stored[0].action, ImportAction::Update);
    assert_eq!(stored[0].payload, Some(serde_json::json!({"id": 20})));
    assert_eq!(stored[1].action, ImportAction::Trash);

    // Concurrent claimers never share an item.
    let claimers = (0..6).map(|_| {
        let items = r.import_items.clone();
        let run = run.id;
        tokio::spawn(async move {
            let claim = Uuid::new_v4();
            let mut won = Vec::new();
            while let Some(item) = items.claim_next(run, claim, now).await.unwrap() {
                assert_eq!(item.claim, Some(claim));
                assert_eq!(item.status, ImportItemStatus::InProgress);
                assert_eq!(item.attempts, 1);
                won.push(item.paperless_id);
            }
            won
        })
    });
    let mut claimed = Vec::new();
    for claimer in claimers {
        claimed.extend(claimer.await.unwrap());
    }
    claimed.sort_unstable();
    assert_eq!(
        claimed,
        (1..=21).collect::<Vec<_>>(),
        "each item claimed once"
    );

    // A claim older than ten minutes can be taken over, a fresh one can't.
    assert_eq!(
        r.import_items
            .claim_next(run.id, Uuid::new_v4(), now + TimeDelta::minutes(9))
            .await
            .unwrap(),
        None
    );
    let stale_claim = r
        .import_items
        .list_items(run.id, None, Some(0), 1)
        .await
        .unwrap()[0]
        .claim
        .unwrap();
    let taker = Uuid::new_v4();
    let retaken = r
        .import_items
        .claim_next(run.id, taker, now + TimeDelta::minutes(11))
        .await
        .unwrap()
        .expect("a stale item is reclaimed");
    assert_eq!(retaken.paperless_id, 1);
    assert_eq!(retaken.attempts, 2);
    assert_eq!(retaken.claim, Some(taker));
    assert_eq!(retaken.claimed_at, Some(now + TimeDelta::minutes(11)));

    let done = ImportItemOutcome {
        status: ImportItemStatus::Done,
        reason: None,
        warnings: vec!["dropped field".into()],
    };
    assert!(
        !r.import_items
            .finish_item(run.id, 1, stale_claim, done.clone(), now)
            .await
            .unwrap(),
        "a lost claim can't finish the item"
    );
    assert!(r
        .import_items
        .finish_item(run.id, 1, taker, done.clone(), now)
        .await
        .unwrap());
    assert!(
        !r.import_items
            .finish_item(run.id, 1, taker, done, now)
            .await
            .unwrap(),
        "a finished item stays finished"
    );

    for id in 2..=21 {
        let holder = r
            .import_items
            .list_items(run.id, None, Some(id - 1), 1)
            .await
            .unwrap()[0]
            .claim
            .unwrap();
        let (status, reason) = match id % 3 {
            0 => (
                ImportItemStatus::Failed,
                Some("checksum_mismatch".to_string()),
            ),
            1 => (ImportItemStatus::Skipped, Some("unchanged".to_string())),
            _ => (ImportItemStatus::Done, None),
        };
        assert!(r
            .import_items
            .finish_item(
                run.id,
                id,
                holder,
                ImportItemOutcome {
                    status,
                    reason,
                    warnings: Vec::new()
                },
                now
            )
            .await
            .unwrap());
    }

    let all = r
        .import_items
        .list_items(run.id, None, None, 100)
        .await
        .unwrap();
    let mut expected: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for item in &all {
        *expected
            .entry((item.action.as_str(), item.status.as_str()))
            .or_default() += 1;
    }
    let counts: BTreeMap<_, _> = r
        .import_items
        .counts(run.id)
        .await
        .unwrap()
        .into_iter()
        .map(|count| ((count.action.as_str(), count.status.as_str()), count.count))
        .collect();
    assert_eq!(counts, expected, "counts equal the items grouped by status");
    assert_eq!(all[0].warnings, vec!["dropped field".to_string()]);

    let failed = r
        .import_items
        .list_items(run.id, Some(ImportItemStatus::Failed), None, 100)
        .await
        .unwrap();
    assert_eq!(
        failed
            .iter()
            .map(|item| item.paperless_id)
            .collect::<Vec<_>>(),
        vec![3, 6, 9, 12, 15, 18, 21]
    );
    assert_eq!(failed[0].reason.as_deref(), Some("checksum_mismatch"));

    assert_eq!(
        r.import_items
            .reset_failed_items(run.id, now)
            .await
            .unwrap(),
        7
    );
    let reset = r
        .import_items
        .list_items(run.id, Some(ImportItemStatus::Pending), None, 100)
        .await
        .unwrap();
    assert_eq!(reset.len(), 7);
    assert!(reset
        .iter()
        .all(|item| item.attempts == 0 && item.claim.is_none() && item.reason.is_none()));
    assert_eq!(
        r.import_items
            .claim_next(run.id, Uuid::new_v4(), now)
            .await
            .unwrap()
            .map(|item| item.paperless_id),
        Some(3)
    );
}

async fn ledger_contract(r: &Repositories, vault: VaultId) {
    let entry = PaperlessLedgerEntry {
        paperless_modified: at("2026-10-01T12:00:00.5Z"),
        ..ledger_entry(vault, 42)
    };
    assert_eq!(
        r.paperless_ledger.get_document(vault, 42).await.unwrap(),
        None
    );
    r.paperless_ledger.upsert_document(&entry).await.unwrap();
    let changed = PaperlessLedgerEntry {
        fingerprint: "f2".into(),
        needs_reapply: true,
        folio_state: FolioState::TrashedByUser,
        last_run_id: Some(Uuid::new_v4()),
        ..entry.clone()
    };
    r.paperless_ledger.upsert_document(&changed).await.unwrap();
    assert_eq!(
        r.paperless_ledger.get_document(vault, 42).await.unwrap(),
        Some(changed.clone())
    );
    let next = PaperlessLedgerEntry {
        paperless_id: 43,
        document_id: Uuid::new_v4(),
        ..entry
    };
    r.paperless_ledger.upsert_document(&next).await.unwrap();
    assert_eq!(
        r.paperless_ledger
            .list_documents(vault, Some(42), 10)
            .await
            .unwrap(),
        vec![next]
    );

    let tag = TaxonomyMapping {
        vault_id: vault,
        kind: TaxonomyKind::Tag,
        paperless_id: 7,
        entity_id: Uuid::new_v4(),
        name: "Invoices".into(),
    };
    r.paperless_ledger.upsert_taxonomy(&tag).await.unwrap();
    let renamed = TaxonomyMapping {
        name: "Bills".into(),
        ..tag.clone()
    };
    r.paperless_ledger.upsert_taxonomy(&renamed).await.unwrap();
    let correspondent = TaxonomyMapping {
        kind: TaxonomyKind::Correspondent,
        ..tag.clone()
    };
    r.paperless_ledger
        .upsert_taxonomy(&correspondent)
        .await
        .unwrap();
    assert_eq!(
        r.paperless_ledger
            .get_taxonomy(vault, TaxonomyKind::Tag, 7)
            .await
            .unwrap(),
        Some(renamed.clone())
    );
    assert_eq!(
        r.paperless_ledger
            .taxonomy_map(vault, TaxonomyKind::Tag)
            .await
            .unwrap(),
        vec![renamed]
    );

    assert_eq!(r.paperless_ledger.sync_state(vault).await.unwrap(), None);
    let mut state = PaperlessSyncState::new(vault, at("2026-11-07T10:00:00Z"));
    r.paperless_ledger
        .put_sync_state(&state, None)
        .await
        .unwrap();
    assert!(
        matches!(
            r.paperless_ledger.put_sync_state(&state, None).await,
            Err(Error::Conflict(_))
        ),
        "a second first write loses"
    );
    assert_eq!(
        r.paperless_ledger.sync_state(vault).await.unwrap(),
        Some(state.clone())
    );
    state.source_id = Some(Uuid::new_v4());
    state.cursor_modified = Some(at("2026-11-07T09:59:00.250Z"));
    state.library_sample = Some(vec![
        LibrarySample {
            paperless_id: 1,
            added: at("2020-01-01T00:00:00Z"),
        },
        LibrarySample {
            paperless_id: 2,
            added: at("2020-01-02T03:04:05Z"),
        },
    ]);
    state.retry_ids = BTreeMap::from([(5, 1), (9, 4)]);
    state.parked_ids = BTreeSet::from([3]);
    state.once_done = true;
    state.last_poll_at = Some(at("2026-11-07T10:00:00Z"));
    state.last_full_reconcile_at = Some(at("2026-11-06T10:00:00Z"));
    let read_at = state.updated_at;
    state.updated_at = at("2026-11-07T10:01:00Z");
    r.paperless_ledger
        .put_sync_state(&state, Some(read_at))
        .await
        .unwrap();
    let mut stale = state.clone();
    stale.cursor_modified = None;
    stale.updated_at = at("2026-11-07T10:02:00Z");
    assert!(
        matches!(
            r.paperless_ledger
                .put_sync_state(&stale, Some(read_at))
                .await,
            Err(Error::Conflict(_))
        ),
        "a writer that read an older state loses"
    );
    assert_eq!(
        r.paperless_ledger.sync_state(vault).await.unwrap(),
        Some(state)
    );
}

async fn contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let other_vault = common::legacy_vault(&db).await;
    let items_vault = common::legacy_vault(&db).await;
    runs_contract(&r, vault, other_vault).await;
    items_contract(&r, items_vault).await;
    ledger_contract(&r, vault).await;
}

fn ledger_entry(vault: VaultId, paperless_id: i64) -> PaperlessLedgerEntry {
    PaperlessLedgerEntry {
        vault_id: vault,
        paperless_id,
        document_id: Uuid::new_v4(),
        paperless_modified: at("2026-10-01T12:00:00Z"),
        checksum: "abc".into(),
        fingerprint: "f1".into(),
        needs_reapply: false,
        trashed_in_paperless: false,
        folio_state: FolioState::Live,
        last_synced_at: at("2026-11-07T10:00:00Z"),
        last_run_id: None,
    }
}

/// The driver's final transition and the cursor copy commit or roll back
/// together
async fn finish_in_transaction(db: &Database, run: &ImportRun, commit: bool) {
    let now = at("2026-11-07T11:00:00Z");
    let mut state = PaperlessSyncState::new(run.vault_id, now);
    state.cursor_modified = Some(at("2026-11-07T09:00:00Z"));
    let from = [ImportRunStatus::Running];
    match db {
        Database::Sqlite(pool) => {
            use folio_infra::database::sqlite::{put_paperless_sync_state, transition_import_run};
            let mut tx = pool.begin().await.unwrap();
            assert_eq!(
                transition_import_run(
                    &mut tx,
                    run.id,
                    &from,
                    ImportRunStatus::Completed,
                    None,
                    now
                )
                .await
                .unwrap(),
                RunTransition::Applied
            );
            put_paperless_sync_state(&mut tx, &state, None)
                .await
                .unwrap();
            if commit {
                tx.commit().await.unwrap();
            }
        }
        Database::Postgres(pool) => {
            use folio_infra::database::postgres::{
                put_paperless_sync_state, transition_import_run,
            };
            let mut tx = pool.begin().await.unwrap();
            assert_eq!(
                transition_import_run(
                    &mut tx,
                    run.id,
                    &from,
                    ImportRunStatus::Completed,
                    None,
                    now
                )
                .await
                .unwrap(),
                RunTransition::Applied
            );
            put_paperless_sync_state(&mut tx, &state, None)
                .await
                .unwrap();
            if commit {
                tx.commit().await.unwrap();
            }
        }
    }
}

async fn record(db: &Database, applied: &ImportApplied, commit: bool) -> Result<(), Error> {
    let now = at("2026-11-07T10:00:00Z");
    match db {
        Database::Sqlite(pool) => {
            let mut tx = pool.begin().await.unwrap();
            let recorded =
                folio_infra::database::sqlite::record_import_applied(&mut tx, applied, now).await;
            if recorded.is_ok() && commit {
                tx.commit().await.unwrap();
            }
            recorded
        }
        Database::Postgres(pool) => {
            let mut tx = pool.begin().await.unwrap();
            let recorded =
                folio_infra::database::postgres::record_import_applied(&mut tx, applied, now).await;
            if recorded.is_ok() && commit {
                tx.commit().await.unwrap();
            }
            recorded
        }
    }
}

/// The ledger row and the item's outcome commit or roll back with the
/// caller's transaction, and only while the claim holds
async fn transaction_contract(db: Database) {
    db.migrate().await.unwrap();
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let run = created(&r, new_run(vault)).await;
    let now = at("2026-11-07T10:00:00Z");
    r.import_items
        .insert_items_ignore_existing(run.id, &[item(1, ImportAction::Create)], now)
        .await
        .unwrap();
    let claim = Uuid::new_v4();
    r.import_items
        .claim_next(run.id, claim, now)
        .await
        .unwrap()
        .unwrap();
    let applied = |claim| ImportApplied {
        ledger: ledger_entry(vault, 1),
        run_id: run.id,
        claim,
        outcome: ImportItemOutcome {
            status: ImportItemStatus::Done,
            reason: None,
            warnings: Vec::new(),
        },
    };

    assert!(matches!(
        record(&db, &applied(Uuid::new_v4()), true).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        r.paperless_ledger.get_document(vault, 1).await.unwrap(),
        None
    );

    record(&db, &applied(claim), false).await.unwrap();
    assert_eq!(
        r.paperless_ledger.get_document(vault, 1).await.unwrap(),
        None
    );

    record(&db, &applied(claim), true).await.unwrap();
    assert!(r
        .paperless_ledger
        .get_document(vault, 1)
        .await
        .unwrap()
        .is_some());
    let finishing = created(&r, new_run(common::legacy_vault(&db).await)).await;
    let start = at("2026-11-07T10:30:00Z");
    r.import_runs
        .transition(
            finishing.id,
            &[ImportRunStatus::Queued],
            ImportRunStatus::Running,
            None,
            start,
        )
        .await
        .unwrap();
    finish_in_transaction(&db, &finishing, false).await;
    assert_eq!(
        r.import_runs
            .get(finishing.vault_id, finishing.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportRunStatus::Running
    );
    assert_eq!(
        r.paperless_ledger
            .sync_state(finishing.vault_id)
            .await
            .unwrap(),
        None
    );
    finish_in_transaction(&db, &finishing, true).await;
    assert_eq!(
        r.import_runs
            .get(finishing.vault_id, finishing.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportRunStatus::Completed
    );
    assert_eq!(
        r.paperless_ledger
            .sync_state(finishing.vault_id)
            .await
            .unwrap()
            .unwrap()
            .cursor_modified,
        Some(at("2026-11-07T09:00:00Z"))
    );

    let items = r
        .import_items
        .list_items(run.id, None, None, 10)
        .await
        .unwrap();
    assert_eq!(items[0].status, ImportItemStatus::Done);
}

async fn sqlite(dir: &tempfile::TempDir) -> Database {
    Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("runs.db").display()
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn sqlite_import_runs_contract() {
    let dir = tempfile::tempdir().unwrap();
    contract(sqlite(&dir).await).await;
}

#[tokio::test]
async fn sqlite_import_runs_transaction_contract() {
    let dir = tempfile::tempdir().unwrap();
    transaction_contract(sqlite(&dir).await).await;
}

async fn postgres() -> Option<(
    testcontainers_modules::testcontainers::ContainerAsync<Postgres>,
    Database,
)> {
    let container = common::container_or_skip(
        "import runs",
        Postgres::default().with_tag("17-alpine").start().await,
    )?;
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    Some((container, db))
}

#[tokio::test]
async fn postgres_import_runs_contract() {
    let Some((_container, db)) = postgres().await else {
        return;
    };
    contract(db).await;
}

#[tokio::test]
async fn postgres_import_runs_transaction_contract() {
    let Some((_container, db)) = postgres().await else {
        return;
    };
    transaction_contract(db).await;
}
