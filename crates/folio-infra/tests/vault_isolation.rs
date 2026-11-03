mod common;

use folio_auth::User;
use folio_core::error::Error;
use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateDocumentType, CreateTag, DocumentFilter, Id,
    Principal, ProcessingStage, ProcessingStatus, UpdateCorrespondent, UpdateDocument,
    UpdateDocumentType, UpdateTag, UserId, VaultId, VaultRole,
};
use folio_core::ports::EmbeddedChunk;
use folio_core::services::VaultService;
use folio_infra::{Database, Repositories};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

const VAULT_MIGRATION: i64 = 20261102010000;
const DOCUMENT: &str = "00000000-0000-4000-8000-000000000001";
const TAG: &str = "00000000-0000-4000-8000-000000000002";
const CORRESPONDENT: &str = "00000000-0000-4000-8000-000000000003";
const DOCUMENT_TYPE: &str = "00000000-0000-4000-8000-000000000004";
static SQLITE_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/postgres");

fn document(title: &str) -> CreateDocument {
    CreateDocument {
        title: title.into(),
        content: Some("vault isolation".into()),
        ..Default::default()
    }
}

fn not_found<T: std::fmt::Debug>(result: Result<T, Error>) {
    assert!(matches!(result, Err(Error::NotFound { .. })), "{result:?}");
}

async fn user(r: &Repositories, name: &str, owner: bool) -> UserId {
    r.auth
        .create_user(User {
            id: name.into(),
            username: name.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: owner,
            is_owner: owner,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    UserId::from(name)
}

async fn scope_contract(r: &Repositories) {
    let a = r.vaults.create("Scope A", None).await.unwrap().id;
    let b = r.vaults.create("Scope B", None).await.unwrap().id;
    let tag_a = r
        .tags
        .create(
            a,
            CreateTag {
                name: "Tax".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let tag_b = r
        .tags
        .create(
            b,
            CreateTag {
                name: "Tax".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let correspondent_a = r
        .correspondents
        .create(
            a,
            CreateCorrespondent {
                name: "Bank".into(),
            },
        )
        .await
        .unwrap();
    let correspondent_b = r
        .correspondents
        .create(
            b,
            CreateCorrespondent {
                name: "Bank".into(),
            },
        )
        .await
        .unwrap();
    let type_a = r
        .document_types
        .create(
            a,
            CreateDocumentType {
                name: "Statement".into(),
            },
        )
        .await
        .unwrap();
    let type_b = r
        .document_types
        .create(
            b,
            CreateDocumentType {
                name: "Statement".into(),
            },
        )
        .await
        .unwrap();
    let doc_a = r
        .documents
        .create(
            a,
            CreateDocument {
                tag_ids: vec![tag_a.id],
                correspondent_id: Some(correspondent_a.id),
                document_type_id: Some(type_a.id),
                ..document("Identical title")
            },
        )
        .await
        .unwrap();
    let doc_b = r
        .documents
        .create(
            b,
            CreateDocument {
                tag_ids: vec![tag_b.id],
                correspondent_id: Some(correspondent_b.id),
                document_type_id: Some(type_b.id),
                ..document("Identical title")
            },
        )
        .await
        .unwrap();

    assert_ne!(tag_a.id, tag_b.id);
    assert_eq!(
        r.tags
            .list(a)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        [tag_a.id]
    );
    assert_eq!(
        r.correspondents
            .list(a)
            .await
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [correspondent_a.id]
    );
    assert_eq!(
        r.document_types
            .list(a)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        [type_a.id]
    );
    assert_eq!(r.documents.list_ids(a, None).await.unwrap(), [doc_a.id]);
    assert_eq!(r.documents.locate(doc_b.id).await.unwrap(), Some(b));
    assert!(r.documents.find_by_id(a, doc_b.id).await.unwrap().is_none());
    assert!(r.tags.find_by_id(a, tag_b.id).await.unwrap().is_none());
    assert!(r
        .correspondents
        .find_by_id(a, correspondent_b.id)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .document_types
        .find_by_id(a, type_b.id)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .documents
        .list(
            a,
            Some(DocumentFilter {
                tag_ids: Some(vec![tag_b.id]),
                ..Default::default()
            })
        )
        .await
        .unwrap()
        .is_empty());

    not_found(
        r.documents
            .update(
                a,
                doc_b.id,
                UpdateDocument {
                    title: Some("changed".into()),
                    ..Default::default()
                },
            )
            .await,
    );
    not_found(r.documents.delete(a, doc_b.id).await);
    not_found(r.documents.set_file_path(a, doc_b.id, "wrong").await);
    not_found(r.documents.set_page_count(a, doc_b.id, 99).await);
    r.documents
        .set_stage_status(a, doc_b.id, ProcessingStage::Ocr, ProcessingStatus::Failed)
        .await
        .unwrap();
    not_found(r.documents.complete_ocr(a, doc_b.id, "wrong").await);
    let unchanged = r.documents.find_by_id(b, doc_b.id).await.unwrap().unwrap();
    assert_eq!(unchanged.content, doc_b.content);
    assert_eq!(unchanged.ocr_status, doc_b.ocr_status);
    not_found(
        r.tags
            .update(
                a,
                tag_b.id,
                UpdateTag {
                    name: Some("changed".into()),
                    ..Default::default()
                },
            )
            .await,
    );
    not_found(r.tags.delete(a, tag_b.id).await);
    not_found(
        r.correspondents
            .update(
                a,
                correspondent_b.id,
                UpdateCorrespondent {
                    name: Some("changed".into()),
                },
            )
            .await,
    );
    not_found(r.correspondents.delete(a, correspondent_b.id).await);
    not_found(
        r.document_types
            .update(
                a,
                type_b.id,
                UpdateDocumentType {
                    name: Some("changed".into()),
                },
            )
            .await,
    );
    not_found(r.document_types.delete(a, type_b.id).await);
    not_found(r.documents.set_tags(a, doc_a.id, &[tag_b.id]).await);
    not_found(r.documents.add_tag(a, doc_a.id, tag_b.id).await);
    not_found(
        r.documents
            .update(
                a,
                doc_a.id,
                UpdateDocument {
                    correspondent_id: Some(Some(correspondent_b.id)),
                    ..Default::default()
                },
            )
            .await,
    );
    not_found(
        r.documents
            .update(
                a,
                doc_a.id,
                UpdateDocument {
                    document_type_id: Some(Some(type_b.id)),
                    ..Default::default()
                },
            )
            .await,
    );
    not_found(
        r.documents
            .create(
                a,
                CreateDocument {
                    tag_ids: vec![tag_b.id],
                    ..document("rejected")
                },
            )
            .await,
    );
    not_found(r.documents.set_tags(a, doc_b.id, &[tag_a.id]).await);
    not_found(r.documents.remove_tag(a, doc_b.id, tag_b.id).await);
    assert!(r.documents.get_tags(a, doc_b.id).await.unwrap().is_empty());
    assert_eq!(
        r.documents.get_tags(a, doc_a.id).await.unwrap()[0].id,
        tag_a.id
    );
    assert_eq!(
        r.documents.get_tags(b, doc_b.id).await.unwrap()[0].id,
        tag_b.id
    );
    assert_eq!(
        r.documents
            .find_by_id(b, doc_b.id)
            .await
            .unwrap()
            .unwrap()
            .title,
        "Identical title"
    );
}

async fn search_and_move_contract(r: &Repositories) {
    let actor = user(r, "moving-editor", false).await;
    let a = r.vaults.create("Move A", None).await.unwrap().id;
    let b = r.vaults.create("Move B", None).await.unwrap().id;
    let tag_a = r
        .tags
        .create(
            a,
            CreateTag {
                name: "Remapped".into(),
                color: Some("#ff0000".into()),
            },
        )
        .await
        .unwrap();
    let tag_b = r
        .tags
        .create(
            b,
            CreateTag {
                name: "Remapped".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let correspondent = r
        .correspondents
        .create(
            a,
            CreateCorrespondent {
                name: "Only in source".into(),
            },
        )
        .await
        .unwrap();
    let document_type = r
        .document_types
        .create(
            a,
            CreateDocumentType {
                name: "Move type".into(),
            },
        )
        .await
        .unwrap();
    let doc = r
        .documents
        .create(
            a,
            CreateDocument {
                uploaded_by: Some(actor.clone()),
                tag_ids: vec![tag_a.id],
                correspondent_id: Some(correspondent.id),
                document_type_id: Some(document_type.id),
                ..document("Movable")
            },
        )
        .await
        .unwrap();
    let other = r
        .documents
        .create(b, document("Other vault"))
        .await
        .unwrap();
    let chunks = [EmbeddedChunk {
        index: 0,
        text: "matching content".into(),
        embedding: vec![1.0, 0.0],
    }];
    r.search
        .store_chunks(a, doc.id, "vault-model", &chunks)
        .await
        .unwrap();
    r.search
        .store_chunks(b, other.id, "vault-model", &chunks)
        .await
        .unwrap();
    not_found(
        r.search
            .store_chunks(a, other.id, "vault-model", &chunks)
            .await,
    );
    r.search.delete_embedding(a, other.id).await.unwrap();
    assert_eq!(
        r.search
            .search(a, "vault-model", &[1.0, 0.0], 10)
            .await
            .unwrap()
            .iter()
            .map(|h| h.document_id)
            .collect::<Vec<_>>(),
        [doc.id]
    );
    assert_eq!(
        r.search
            .search(b, "vault-model", &[1.0, 0.0], 10)
            .await
            .unwrap()
            .iter()
            .map(|h| h.document_id)
            .collect::<Vec<_>>(),
        [other.id]
    );

    let moved = r
        .documents
        .move_to(doc.id, a, b, Some(&actor))
        .await
        .unwrap();

    assert_eq!(moved.vault_id, b);
    assert_eq!(moved.uploaded_by, Some(actor.clone()));
    assert!(r.documents.find_by_id(a, doc.id).await.unwrap().is_none());
    assert_eq!(
        r.documents.get_tags(b, doc.id).await.unwrap()[0].id,
        tag_b.id
    );
    let target_correspondent = r
        .correspondents
        .find_by_name(b, "Only in source")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(target_correspondent.id, correspondent.id);
    assert_eq!(moved.correspondent_id, Some(target_correspondent.id));
    let target_type = r
        .document_types
        .find_by_name(b, "Move type")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(target_type.id, document_type.id);
    assert_eq!(moved.document_type_id, Some(target_type.id));
    assert!(r
        .search
        .search(a, "vault-model", &[1.0, 0.0], 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        r.search
            .search(b, "vault-model", &[1.0, 0.0], 10)
            .await
            .unwrap()
            .len(),
        2
    );
    let source_audit: Vec<_> = r
        .audit
        .list(a, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "document.moved")
        .collect();
    let target_audit: Vec<_> = r
        .audit
        .list(b, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "document.moved")
        .collect();
    assert_eq!(source_audit.len(), 1);
    assert_eq!(target_audit.len(), 1);
    assert_eq!(source_audit[0].actor, Some(actor));
    assert_eq!(source_audit[0].action, "document.moved");
    assert_eq!(
        target_audit[0].target_id.as_deref(),
        Some(doc.id.to_string().as_str())
    );
    assert_eq!(
        source_audit[0].details,
        serde_json::json!({ "from": a, "to": b })
    );
    r.documents.move_to(doc.id, b, b, None).await.unwrap();
    assert_eq!(
        r.audit
            .list(b, 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.action == "document.moved")
            .count(),
        1
    );
}

async fn membership_contract(r: &Repositories) {
    let alice = user(r, "membership-alice", false).await;
    let bob = user(r, "membership-bob", false).await;
    let vault = r.vaults.create("Membership", None).await.unwrap().id;
    let local = r.members.ensure_group("local", "family").await.unwrap();
    let external = r.members.ensure_group("external", "family").await.unwrap();
    assert_ne!(local, external);
    r.members
        .grant(
            vault,
            &Principal::User(alice.clone()),
            VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    r.members
        .grant(
            vault,
            &Principal::User(bob.clone()),
            VaultRole::Viewer,
            Some(&alice),
        )
        .await
        .unwrap();
    r.members
        .grant(
            vault,
            &Principal::Group(local),
            VaultRole::Owner,
            Some(&alice),
        )
        .await
        .unwrap();
    r.members
        .grant(
            vault,
            &Principal::Group(external.clone()),
            VaultRole::Editor,
            Some(&alice),
        )
        .await
        .unwrap();
    r.auth
        .set_external_groups(bob.as_str(), &["family".into()])
        .await
        .unwrap();
    assert_eq!(
        r.access.effective_role(&bob, vault).await.unwrap(),
        Some(VaultRole::Editor)
    );
    assert_eq!(r.members.direct_owner_count(vault).await.unwrap(), 1);

    assert!(matches!(
        r.members
            .revoke(vault, &Principal::User(alice.clone()))
            .await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        r.members
            .grant(
                vault,
                &Principal::User(alice.clone()),
                VaultRole::Editor,
                None
            )
            .await,
        Err(Error::Conflict(_))
    ));
    r.auth.set_external_groups(bob.as_str(), &[]).await.unwrap();
    assert_eq!(
        r.access.effective_role(&bob, vault).await.unwrap(),
        Some(VaultRole::Viewer)
    );
    r.members
        .revoke(vault, &Principal::User(bob.clone()))
        .await
        .unwrap();
    assert_eq!(r.access.effective_role(&bob, vault).await.unwrap(), None);
    assert!(r.vaults.list_for_user(&bob).await.unwrap().is_empty());
    r.members
        .grant(vault, &Principal::User(bob), VaultRole::Owner, None)
        .await
        .unwrap();
    r.members
        .grant(
            vault,
            &Principal::User(alice.clone()),
            VaultRole::Editor,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        r.access.effective_role(&alice, vault).await.unwrap(),
        Some(VaultRole::Editor)
    );
    assert_eq!(r.members.direct_owner_count(vault).await.unwrap(), 1);
}

async fn provisioning_contract(r: &Repositories) {
    let owner = user(r, "provision-owner", true).await;
    let service = VaultService::new(r.vaults.clone(), r.members.clone());
    let personal = service.default_vault(Some(&owner)).await.unwrap();
    assert_eq!(
        r.vaults.find(personal).await.unwrap().unwrap().name,
        "Personal"
    );
    assert_eq!(
        r.access.effective_role(&owner, personal).await.unwrap(),
        Some(VaultRole::Owner)
    );
    r.vaults.rename(personal, "Renamed personal").await.unwrap();
    assert_eq!(service.default_vault(Some(&owner)).await.unwrap(), personal);
    assert_eq!(r.vaults.list_for_user(&owner).await.unwrap().len(), 1);

    let newcomer = user(r, "provision-group-member", false).await;
    let shared = r
        .vaults
        .create("Shared before first login", None)
        .await
        .unwrap()
        .id;
    let group = r
        .members
        .ensure_group("external", "provision-group")
        .await
        .unwrap();
    r.members
        .grant(shared, &Principal::Group(group), VaultRole::Viewer, None)
        .await
        .unwrap();
    r.auth
        .set_external_groups(newcomer.as_str(), &["provision-group".into()])
        .await
        .unwrap();
    let newcomer_default = service.default_vault(Some(&newcomer)).await.unwrap();
    assert_ne!(newcomer_default, shared);
    assert_eq!(
        r.vaults.find(newcomer_default).await.unwrap().unwrap().name,
        "Personal"
    );
    assert_eq!(
        r.access
            .effective_role(&newcomer, newcomer_default)
            .await
            .unwrap(),
        Some(VaultRole::Owner)
    );
    r.vaults.set_default(&newcomer, shared).await.unwrap();
    r.vaults.delete(newcomer_default).await.unwrap();
    assert_eq!(
        service.default_vault(Some(&newcomer)).await.unwrap(),
        shared
    );
    r.auth
        .set_external_groups(newcomer.as_str(), &[])
        .await
        .unwrap();
    assert!(service.default_vault(Some(&newcomer)).await.is_err());
    assert!(r.vaults.list_for_user(&newcomer).await.unwrap().is_empty());

    let first_owner = user(r, "ownerless-first-owner", true).await;
    let archive = r.vaults.create("Archive", None).await.unwrap().id;
    let personal = service.default_vault(Some(&first_owner)).await.unwrap();
    assert_ne!(personal, archive);
    assert_eq!(
        r.access
            .effective_role(&first_owner, archive)
            .await
            .unwrap(),
        None
    );
    let owned = r.vaults.list_for_user(&first_owner).await.unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].0.name, "Personal");
    assert_eq!(owned[0].1, VaultRole::Owner);
}

async fn foundation_contract(db: &Database) {
    db.migrate().await.unwrap();
    let repos = db.repositories().await;
    scope_contract(&repos).await;
    search_and_move_contract(&repos).await;
    membership_contract(&repos).await;
    provisioning_contract(&repos).await;
}

async fn execute(db: &Database, sql: &str) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn migration_prefix(db: &Database) {
    let migrations = match db {
        Database::Sqlite(_) => &SQLITE_MIGRATIONS,
        Database::Postgres(_) => &POSTGRES_MIGRATIONS,
    };
    for migration in migrations
        .iter()
        .filter(|m| m.version > 0 && m.version < VAULT_MIGRATION)
    {
        execute(db, migration.sql.as_ref()).await;
    }
}

async fn migration_suffix(db: &Database) {
    let migrations = match db {
        Database::Sqlite(_) => &SQLITE_MIGRATIONS,
        Database::Postgres(_) => &POSTGRES_MIGRATIONS,
    };
    for migration in migrations.iter().filter(|m| m.version >= VAULT_MIGRATION) {
        execute(db, migration.sql.as_ref()).await;
    }
}

async fn seed_legacy_document(db: &Database) {
    execute(db, &format!(
        "INSERT INTO correspondents (id, name, created_at) VALUES ('{CORRESPONDENT}', 'Legacy bank', '2026-01-01T00:00:00Z');
         INSERT INTO document_types (id, name, created_at) VALUES ('{DOCUMENT_TYPE}', 'Legacy type', '2026-01-01T00:00:00Z');
         INSERT INTO tags (id, name, created_at) VALUES ('{TAG}', 'Legacy tag', '2026-01-01T00:00:00Z');
         INSERT INTO documents (id, title, content, correspondent_id, document_type_id, created_at, updated_at)
         VALUES ('{DOCUMENT}', 'Legacy document', 'legacy content', '{CORRESPONDENT}', '{DOCUMENT_TYPE}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
         INSERT INTO document_tags (document_id, tag_id) VALUES ('{DOCUMENT}', '{TAG}');"
    )).await;
    let embedding = match db {
        Database::Sqlite(_) => "X'0000803f00000000'",
        Database::Postgres(_) => "ARRAY[1.0,0.0]",
    };
    execute(db, &format!(
        "INSERT INTO document_chunks (document_id, chunk_index, text, model, dimensions, embedding, created_at)
         VALUES ('{DOCUMENT}', 0, 'legacy content', 'legacy-model', 2, {embedding}, '2026-01-01T00:00:00Z')"
    )).await;
}

#[derive(Clone, Copy)]
enum Fixture {
    Empty,
    ExistingAdmin,
    Ownerless,
    LedgerOnly,
}

async fn migration_contract(db: &Database, fixture: Fixture) {
    migration_prefix(db).await;
    match fixture {
        Fixture::Empty => {}
        Fixture::ExistingAdmin => {
            execute(db, "INSERT INTO users (id, username, username_normalized, password_hash, created_at, is_admin, is_owner)
                VALUES ('old-admin', 'old-admin', 'old-admin', '!', 1, TRUE, TRUE),
                       ('new-admin', 'new-admin', 'new-admin', '!', 2, TRUE, FALSE)").await;
            seed_legacy_document(db).await;
        }
        Fixture::Ownerless => seed_legacy_document(db).await,
        Fixture::LedgerOnly => execute(db, "INSERT INTO ingest_ledger (key, source_id, document_id, outcome, created_at)
            VALUES ('retained-message', '00000000-0000-4000-8000-000000000005', NULL, 'skipped', '2026-01-01T00:00:00Z')").await,
    }
    migration_suffix(db).await;
    let r = db.repositories().await;
    let query = "SELECT v.id FROM vaults v WHERE v.name = 'Archive' AND NOT EXISTS (
        SELECT 1 FROM vault_members m WHERE m.vault_id = v.id
        AND m.role = 'owner' AND m.user_id IS NOT NULL) ORDER BY v.created_at, v.id";
    let ownerless: Vec<VaultId> = match db {
        Database::Sqlite(pool) => sqlx::query_scalar::<_, String>(query)
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|id| VaultId(id.parse().unwrap()))
            .collect(),
        Database::Postgres(pool) => sqlx::query_scalar::<_, uuid::Uuid>(query)
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(VaultId)
            .collect(),
    };
    match fixture {
        Fixture::Empty => {
            assert!(ownerless.is_empty());
            match db {
                Database::Sqlite(pool) => assert_eq!(
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM vaults")
                        .fetch_one(pool)
                        .await
                        .unwrap(),
                    0
                ),
                Database::Postgres(pool) => assert_eq!(
                    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM vaults")
                        .fetch_one(pool)
                        .await
                        .unwrap(),
                    0
                ),
            }
        }
        Fixture::LedgerOnly => {
            assert_eq!(ownerless.len(), 1);
            assert!(r
                .ledger
                .find(ownerless[0], "retained-message")
                .await
                .unwrap()
                .is_some());
        }
        Fixture::ExistingAdmin | Fixture::Ownerless => {
            let id: Id = DOCUMENT.parse().unwrap();
            let archive = r.documents.locate(id).await.unwrap().unwrap();
            assert_eq!(
                r.vaults.find(archive).await.unwrap().unwrap().name,
                "Archive"
            );
            let document = r.documents.find_by_id(archive, id).await.unwrap().unwrap();
            assert_eq!(
                document.correspondent_id,
                Some(CORRESPONDENT.parse().unwrap())
            );
            assert_eq!(
                document.document_type_id,
                Some(DOCUMENT_TYPE.parse().unwrap())
            );
            assert_eq!(document.uploaded_by, None);
            assert_eq!(
                r.documents.get_tags(archive, id).await.unwrap()[0].id,
                TAG.parse::<Id>().unwrap()
            );
            assert_eq!(
                r.search
                    .search(archive, "legacy-model", &[1.0, 0.0], 10)
                    .await
                    .unwrap()[0]
                    .document_id,
                id
            );
            if matches!(fixture, Fixture::ExistingAdmin) {
                let old = UserId::from("old-admin");
                assert_eq!(
                    r.access.effective_role(&old, archive).await.unwrap(),
                    Some(VaultRole::Owner)
                );
                assert_eq!(
                    r.access
                        .effective_role(&UserId::from("new-admin"), archive)
                        .await
                        .unwrap(),
                    None
                );
                let service = VaultService::new(r.vaults.clone(), r.members.clone());
                assert_eq!(service.default_vault(Some(&old)).await.unwrap(), archive);
                assert_eq!(r.vaults.list_for_user(&old).await.unwrap().len(), 2);
            } else {
                assert_eq!(ownerless, [archive]);
                let first = user(&r, "first-after-upgrade", true).await;
                let service = VaultService::new(r.vaults.clone(), r.members.clone());
                let personal = service.default_vault(Some(&first)).await.unwrap();
                assert_ne!(personal, archive);
                assert_eq!(r.vaults.list_for_user(&first).await.unwrap().len(), 1);
                assert_eq!(
                    r.access.effective_role(&first, archive).await.unwrap(),
                    None
                );
            }
            if let Database::Sqlite(pool) = db {
                assert!(sqlx::query("PRAGMA foreign_key_check")
                    .fetch_all(pool)
                    .await
                    .unwrap()
                    .is_empty());
                assert_eq!(
                    sqlx::query_scalar::<_, i64>("PRAGMA foreign_keys")
                        .fetch_one(pool)
                        .await
                        .unwrap(),
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn sqlite_vault_isolation_membership_and_provisioning() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("vaults.db").display()
    ))
    .await
    .unwrap();
    foundation_contract(&db).await;
}

#[tokio::test]
async fn postgres_vault_isolation_membership_and_provisioning() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL vault contract",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    foundation_contract(&db).await;
    tag_assignment_waits_for_a_concurrent_vault_move(&db).await;
}

async fn tag_assignment_waits_for_a_concurrent_vault_move(db: &Database) {
    let Database::Postgres(pool) = db else {
        panic!("this contract requires PostgreSQL row locks");
    };
    let r = db.repositories().await;
    let from = r.vaults.create("Concurrent source", None).await.unwrap().id;
    let to = r.vaults.create("Concurrent target", None).await.unwrap().id;
    let doc = r.documents.create(from, document("Moving")).await.unwrap();
    let tag = r
        .tags
        .create(
            from,
            CreateTag {
                name: "Source tag".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let mut movement = pool.begin().await.unwrap();
    sqlx::query("UPDATE documents SET vault_id = $1 WHERE id = $2")
        .bind(to.0)
        .bind(doc.id)
        .execute(&mut *movement)
        .await
        .unwrap();
    let mut assignment = Box::pin(r.documents.add_tag(from, doc.id, tag.id));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut assignment)
            .await
            .is_err()
    );
    movement.commit().await.unwrap();
    not_found(assignment.await);
    assert!(r.documents.get_tags(to, doc.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_migration_preserves_existing_rows_and_handles_empty_ownerless_and_ledger_only_databases(
) {
    for fixture in [
        Fixture::Empty,
        Fixture::ExistingAdmin,
        Fixture::Ownerless,
        Fixture::LedgerOnly,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(&format!(
            "sqlite:{}?mode=rwc",
            dir.path().join("migration.db").display()
        ))
        .await
        .unwrap();
        migration_contract(&db, fixture).await;
    }
}

#[tokio::test]
async fn postgres_migration_preserves_existing_rows_and_handles_empty_ownerless_and_ledger_only_databases(
) {
    let Some(container) = common::container_or_skip(
        "PostgreSQL vault migration contract",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let admin = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    for (name, fixture) in [
        ("empty", Fixture::Empty),
        ("existing", Fixture::ExistingAdmin),
        ("ownerless", Fixture::Ownerless),
        ("ledger", Fixture::LedgerOnly),
    ] {
        execute(&admin, &format!("CREATE DATABASE {name}")).await;
        let db = Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/{name}"
        ))
        .await
        .unwrap();
        migration_contract(&db, fixture).await;
    }
}
