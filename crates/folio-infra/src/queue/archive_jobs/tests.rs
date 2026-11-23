use super::*;
use crate::archival::{ToolPaths, PROFILE};
use crate::database::{test_pool, test_vault};
use crate::queue::{JobClass, JobQueue};
use crate::storage::ObjectStoreDocumentStore;
use folio_core::models::CreateDocument;
use folio_core::ports::DocumentRepository;
use std::os::unix::fs::PermissionsExt;

const ORIGINAL: &[u8] = include_bytes!("../../../../../static/icons/icon-192.png");
const COPY: &[u8] = b"%PDF-test archival bytes";

struct Fixture {
    archive: ArchiveService,
    docs: Arc<dyn DocumentRepository>,
    queue: JobQueue,
    document: Id,
    original: FileIdentity,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn new(tools: impl FnOnce(&std::path::Path) -> ToolPaths) -> Self {
        let (pool, dir) = test_pool().await;
        let database = Database::Sqlite(pool.clone());
        let repos = database.repositories().await;
        let docs = repos.documents;
        let vault = test_vault();
        let document = docs
            .create_for_upload(
                vault,
                CreateDocument {
                    title: "Original image".into(),
                    mime_type: Some("image/png".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        let original = FileIdentity::from_bytes(keys::original(document), "image/png", ORIGINAL);
        let store = Arc::new(ObjectStoreDocumentStore::in_memory());
        store
            .put(&original.storage_key, ORIGINAL.into())
            .await
            .unwrap();
        docs.finalize_verified_upload(vault, document, &original)
            .await
            .unwrap();
        let converter = ArchiveConverter::with_tools("eng", tools(dir.path()));
        let archive = ArchiveService::new(database, repos.archival, store, converter, true);
        Self {
            archive,
            docs,
            queue: JobQueue::sqlite(pool),
            document,
            original,
            _dir: dir,
        }
    }

    async fn claim(&self) -> super::super::Job {
        self.queue
            .dequeue_class(JobClass::Archive)
            .await
            .unwrap()
            .unwrap()
    }

    async fn run(&self) {
        let job = self.claim().await;
        self.archive.generate(&self.queue, &job).await.unwrap();
        assert!(self.queue.complete(&job).await.unwrap());
    }

    async fn status(&self) -> ArchiveAttempt {
        self.archive
            .status(test_vault(), self.document)
            .await
            .unwrap()
            .unwrap()
    }

    async fn assert_original(&self) {
        assert_eq!(
            self.archive
                .store
                .get_bytes(&self.original.storage_key)
                .await
                .unwrap(),
            ORIGINAL
        );
        assert!(self
            .docs
            .find_by_id(test_vault(), self.document)
            .await
            .unwrap()
            .is_some());
    }
}

#[tokio::test]
async fn backfill_duplicate_delivery_and_explicit_retry_have_one_job_per_attempt() {
    let fixture = Fixture::new(|path| ToolPaths {
        ocrmypdf: path.join("missing"),
        ..ToolPaths::default()
    })
    .await;
    let vault = test_vault();
    assert_eq!(
        fixture
            .archive
            .backfill(VaultId(Uuid::new_v4()), 32)
            .await
            .unwrap(),
        0
    );
    assert_eq!(fixture.archive.backfill(vault, 32).await.unwrap(), 1);
    assert_eq!(fixture.archive.backfill(vault, 32).await.unwrap(), 0);
    let first = fixture.status().await.id;
    assert_eq!(
        fixture
            .archive
            .enqueue(vault, fixture.document)
            .await
            .unwrap(),
        first
    );
    assert!(fixture
        .queue
        .dequeue_class(JobClass::Processing)
        .await
        .unwrap()
        .is_none());
    fixture.run().await;
    assert!(matches!(fixture.status().await.state,
        ArchiveState::Completed(ArchiveCompletion::Failed(reason))
        if reason.contains("OcrMyPdf") && reason.contains("unavailable")));
    let retry = || fixture.archive.retry(vault, fixture.document, first);
    let (one, two) = tokio::join!(retry(), retry());
    let second = match (one, two) {
        (Ok(id), Err(Error::Conflict(_))) | (Err(Error::Conflict(_)), Ok(id)) => id,
        outcomes => panic!("expected one retry admission: {outcomes:?}"),
    };
    assert_ne!(second, first);
    fixture.run().await;
    assert!(fixture
        .queue
        .dequeue_class(JobClass::Archive)
        .await
        .unwrap()
        .is_none());
    fixture.assert_original().await;
}

fn tool(dir: &std::path::Path, name: &str, script: &str) -> std::path::PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    file
}

fn tools(dir: &std::path::Path, compliant: bool) -> ToolPaths {
    let passed = usize::from(compliant);
    let failed = 1 - passed;
    ToolPaths {
        qpdf: tool(
            dir,
            "qpdf",
            r#"if [ "$1" = "--is-encrypted" ]; then exit 2; fi
echo '{"qpdf":[{"jsonversion":2},{"trailer":{"value":{}}}],"pages":[{}]}'"#,
        ),
        ocrmypdf: tool(
            dir,
            "ocrmypdf",
            r#"if [ "$1" = "--version" ]; then echo 14.0.0; exit; fi
for output do :; done
printf '%%PDF-test archival bytes' > "$output""#,
        ),
        verapdf: tool(
            dir,
            "verapdf",
            &format!(
                r#"cat <<'REPORT'
<report><buildInformation><releaseDetails id="core" version="1.30.4"/></buildInformation>
<jobs><job><item size="{}"/><validationReport profileName="PDF/A-2b validation profile" isCompliant="{compliant}"/></job></jobs>
<batchSummary totalJobs="1" failedToParse="0" encrypted="0" outOfMemory="0" veraExceptions="0">
<validationReports compliant="{passed}" nonCompliant="{failed}" failedJobs="0">1</validationReports>
</batchSummary></report>
REPORT
exit {failed}"#,
                COPY.len()
            ),
        ),
        ..ToolPaths::default()
    }
}

#[tokio::test]
async fn publishes_only_validated_bytes_and_preserves_original_on_validation_failure() {
    for compliant in [true, false] {
        let fixture = Fixture::new(|dir| tools(dir, compliant)).await;
        let attempt = fixture
            .archive
            .enqueue(test_vault(), fixture.document)
            .await
            .unwrap();
        fixture.run().await;
        let status = fixture.status().await;
        let key = keys::revision_artifact(fixture.document, status.source.revision_id, attempt);
        match status.state {
            ArchiveState::Completed(ArchiveCompletion::Ready(output)) if compliant => {
                assert_eq!(
                    output.file,
                    FileIdentity::from_bytes(&key, "application/pdf", COPY)
                );
                assert_eq!(output.profile, PROFILE);
                assert_eq!(fixture.archive.store.get_bytes(&key).await.unwrap(), COPY);
            }
            ArchiveState::Completed(ArchiveCompletion::Failed(reason)) if !compliant => {
                assert!(reason.contains("validation rejected"), "{reason}");
                assert!(!fixture.archive.store.exists(&key).await.unwrap());
            }
            state => panic!("unexpected archive result: {state:?}"),
        }
        fixture.assert_original().await;
    }
}

#[tokio::test]
async fn interrupted_writer_cannot_publish_after_replay_fences_it() {
    let fixture = Fixture::new(|dir| tools(dir, true)).await;
    fixture
        .archive
        .enqueue(test_vault(), fixture.document)
        .await
        .unwrap();
    let status = fixture.status().await;
    let job = fixture.claim().await;
    assert!(fixture
        .archive
        .database
        .reserve_archive_output(&status.source, &job)
        .await
        .unwrap());
    let key = keys::revision_artifact(fixture.document, status.source.revision_id, status.id);
    assert!(fixture
        .archive
        .store
        .put_if_absent(&key, COPY.into())
        .await
        .unwrap());
    fixture
        .archive
        .generate(&fixture.queue, &job)
        .await
        .unwrap();
    assert!(fixture.queue.complete(&job).await.unwrap());
    assert!(matches!(
        fixture.status().await.state,
        ArchiveState::Completed(ArchiveCompletion::Failed(_))
    ));
    let output = ValidatedArchive {
        file: FileIdentity::from_bytes(&key, "application/pdf", COPY),
        profile: PROFILE.into(),
        converter: ArchiveTool {
            name: "converter".into(),
            version: "1".into(),
        },
        validator: ArchiveTool {
            name: "validator".into(),
            version: "1".into(),
        },
    };
    assert!(!fixture
        .archive
        .repository
        .complete_archive(&status.source, status.id, &ArchiveCompletion::Ready(output))
        .await
        .unwrap());
    fixture.archive.repair().await.unwrap();
    assert!(!fixture.archive.store.exists(&key).await.unwrap());
    fixture.assert_original().await;
}

#[derive(Clone, Copy)]
enum Settlement {
    CommittedButRejected,
    CommittedButAmbiguous,
    RejectedWithUnavailableRetention,
}

struct UncertainRepository {
    inner: Arc<dyn ArchivalRepository>,
    settlement: Settlement,
}

#[async_trait::async_trait]
impl ArchivalRepository for UncertainRepository {
    async fn current_source(&self, vault: VaultId, document: Id) -> Result<Option<ArchiveSource>> {
        self.inner.current_source(vault, document).await
    }

    async fn current_attempt(
        &self,
        vault: VaultId,
        document: Id,
    ) -> Result<Option<ArchiveAttempt>> {
        self.inner.current_attempt(vault, document).await
    }

    async fn enqueue_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        expected: Option<Id>,
    ) -> Result<bool> {
        self.inner.enqueue_archive(source, attempt, expected).await
    }

    async fn complete_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        completion: &ArchiveCompletion,
    ) -> Result<bool> {
        match self.settlement {
            Settlement::RejectedWithUnavailableRetention => Ok(false),
            Settlement::CommittedButRejected | Settlement::CommittedButAmbiguous => {
                assert!(
                    self.inner
                        .complete_archive(source, attempt, completion)
                        .await?
                );
                match self.settlement {
                    Settlement::CommittedButAmbiguous => {
                        Err(Error::Repository("Commit response lost".into()))
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    async fn retains_archive(&self, document: Id, revision: Id, attempt: Id) -> Result<bool> {
        match self.settlement {
            Settlement::RejectedWithUnavailableRetention => {
                Err(Error::Repository("Lookup unavailable".into()))
            }
            _ => {
                self.inner
                    .retains_archive(document, revision, attempt)
                    .await
            }
        }
    }
}

#[tokio::test]
async fn rejected_or_ambiguous_publication_keeps_committed_or_unresolved_bytes() {
    for settlement in [
        Settlement::CommittedButRejected,
        Settlement::CommittedButAmbiguous,
        Settlement::RejectedWithUnavailableRetention,
    ] {
        let mut fixture = Fixture::new(|dir| tools(dir, true)).await;
        fixture
            .archive
            .enqueue(test_vault(), fixture.document)
            .await
            .unwrap();
        let status = fixture.status().await;
        let inner = fixture.archive.repository.clone();
        fixture.archive.repository = Arc::new(UncertainRepository {
            inner: inner.clone(),
            settlement,
        });
        let job = fixture.claim().await;
        let result = fixture.archive.generate(&fixture.queue, &job).await;
        assert_eq!(
            result.is_err(),
            matches!(settlement, Settlement::CommittedButAmbiguous)
        );
        let key = keys::revision_artifact(fixture.document, status.source.revision_id, status.id);
        assert_eq!(fixture.archive.store.get_bytes(&key).await.unwrap(), COPY);

        if matches!(settlement, Settlement::RejectedWithUnavailableRetention) {
            // When delivery is terminal, repair retires the unpublished attempt
            // and rechecks retention before removing its staged object.
            assert!(fixture.queue.complete(&job).await.unwrap());
            fixture.archive.repository = inner;
            fixture.archive.repair().await.unwrap();
            assert!(!fixture.archive.store.exists(&key).await.unwrap());
        }
        fixture.assert_original().await;
    }
}

#[tokio::test]
async fn converter_failure_changes_only_the_archive_attempt() {
    let fixture = Fixture::new(|dir| {
        let mut paths = tools(dir, true);
        paths.ocrmypdf = tool(
            dir,
            "failing-ocr",
            r#"if [ "$1" = "--version" ]; then echo 14.0.0; exit; fi
exit 1"#,
        );
        paths
    })
    .await;
    fixture
        .archive
        .enqueue(test_vault(), fixture.document)
        .await
        .unwrap();
    fixture.run().await;
    let status = fixture.status().await;
    assert!(matches!(
        status.state,
        ArchiveState::Completed(ArchiveCompletion::Failed(reason)) if reason == "OcrMyPdf failed"
    ));
    let key = keys::revision_artifact(fixture.document, status.source.revision_id, status.id);
    assert!(!fixture.archive.store.exists(&key).await.unwrap());
    fixture.assert_original().await;
}
