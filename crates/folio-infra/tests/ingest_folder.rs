//! The consume folder adapter against a real directory.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Document, FolderAfterImport, FolderConfig, Id, IngestItem, IngestItemKey, IngestOutcome,
    PollReport, VaultId,
};
use folio_core::ports::{IngestSink, IngestionSource};
use folio_infra::FolderSource;

#[derive(Default)]
struct RecordingSink {
    items: Mutex<Vec<IngestItem>>,
    observations: Mutex<Vec<IngestItemKey>>,
    fail_with: Option<String>,
    busy_once: Mutex<bool>,
}

#[async_trait]
impl IngestSink for RecordingSink {
    async fn ingest_observed(
        &self,
        source: Id,
        key: IngestItemKey,
        item: IngestItem,
    ) -> Result<IngestOutcome> {
        self.observations.lock().unwrap().push(key);
        self.ingest(source, item).await
    }

    async fn ingest(&self, _source: Id, item: IngestItem) -> Result<IngestOutcome> {
        if let Some(message) = &self.fail_with {
            return Err(Error::Storage(message.clone()));
        }
        let mut busy_once = self.busy_once.lock().unwrap();
        if *busy_once {
            *busy_once = false;
            return Ok(IngestOutcome::Busy);
        }
        self.items.lock().unwrap().push(item);
        Ok(IngestOutcome::Imported(Box::new(Document::new(
            VaultId::new(),
            "t",
        ))))
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("folio-consume-{}", Id::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn source(&self, after: FolderAfterImport) -> FolderSource {
        let config = FolderConfig {
            path: self.root.to_str().unwrap().to_string(),
            recursive: true,
            subfolders_as_tags: true,
            after_import: after,
            settle_secs: 0,
            rescan_secs: 60,
        };
        FolderSource::new(Id::new_v4(), "inbox", config).with_settle(Duration::ZERO)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn names(sink: &RecordingSink) -> Vec<String> {
    let mut names: Vec<_> = sink
        .items
        .lock()
        .unwrap()
        .iter()
        .map(|i| i.filename.clone())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_file_is_imported_only_after_its_size_stays_stable() {
    let fx = Fixture::new();
    let path = fx.write("scan.pdf", b"%PDF-1");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();

    let first = source.poll(&sink).await.unwrap();
    assert_eq!(first.imported, 0, "seen once is not yet settled");
    assert!(
        first.recheck_in.is_some(),
        "asks to be looked at again soon"
    );

    // The copy is still going: the size changes between looks.
    std::fs::write(&path, b"%PDF-1.7 more bytes").unwrap();
    assert_eq!(source.poll(&sink).await.unwrap().imported, 0);

    let settled = source.poll(&sink).await.unwrap();
    assert_eq!(settled.imported, 1);
    assert_eq!(sink.items.lock().unwrap()[0].data, b"%PDF-1.7 more bytes");
}

#[tokio::test]
async fn the_settle_time_must_elapse() {
    let fx = Fixture::new();
    fx.write("a.pdf", b"x");
    let source = fx
        .source(FolderAfterImport::Move)
        .with_settle(Duration::from_millis(400));
    let sink = RecordingSink::default();

    source.poll(&sink).await.unwrap();
    assert_eq!(source.poll(&sink).await.unwrap().imported, 0, "too soon");
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(source.poll(&sink).await.unwrap().imported, 1);
}

#[tokio::test]
async fn hidden_temporary_and_partial_files_are_left_alone() {
    let fx = Fixture::new();
    for name in [
        ".hidden.pdf",
        "~$draft.docx",
        "download.pdf.part",
        "scan.tmp",
        ".git/config",
    ] {
        fx.write(name, b"data");
    }
    fx.write("real.pdf", b"data");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();

    source.poll(&sink).await.unwrap();
    source.poll(&sink).await.unwrap();

    assert_eq!(names(&sink), ["real.pdf"]);
    assert!(fx.root.join(".hidden.pdf").exists());
    assert!(fx.root.join("download.pdf.part").exists());
}

#[tokio::test]
async fn subfolders_become_tags_and_the_origin_is_recorded() {
    let fx = Fixture::new();
    fx.write("Insurance/2025/x.pdf", b"data");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();

    source.poll(&sink).await.unwrap();
    source.poll(&sink).await.unwrap();

    let items = sink.items.lock().unwrap();
    assert_eq!(items[0].tags, ["Insurance", "2025"]);
    assert_eq!(items[0].mime_type, "application/pdf");
    let origin = items[0].source.folder.as_ref().unwrap();
    assert_eq!(origin.relative_path, "Insurance/2025/x.pdf");
    assert_eq!(origin.original_filename, "x.pdf");
}

#[tokio::test]
async fn imported_files_move_to_processed_or_are_deleted() {
    let fx = Fixture::new();
    fx.write("Tax/a.pdf", b"data");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();
    source.poll(&sink).await.unwrap();
    source.poll(&sink).await.unwrap();
    assert!(!fx.root.join("Tax/a.pdf").exists());
    assert!(fx.root.join("processed/Tax/a.pdf").exists());

    // Files below processed/ are never imported again.
    source.poll(&sink).await.unwrap();
    assert_eq!(sink.items.lock().unwrap().len(), 1);

    let fx = Fixture::new();
    let path = fx.write("b.pdf", b"data");
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();
    source.poll(&sink).await.unwrap();
    source.poll(&sink).await.unwrap();
    assert!(!path.exists());
    assert!(!fx.root.join("processed").exists());
}

#[tokio::test]
async fn failed_files_are_quarantined_with_an_error_note() {
    let fx = Fixture::new();
    fx.write("Tax/broken.pdf", b"data");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink {
        fail_with: Some("disk on fire".into()),
        ..Default::default()
    };

    source.poll(&sink).await.unwrap();
    let report = source.poll(&sink).await.unwrap();

    assert_eq!(report.failed, 1);
    assert!(!fx.root.join("Tax/broken.pdf").exists());
    assert!(Path::new(&fx.root.join("failed/Tax/broken.pdf")).exists());
    let note = std::fs::read_to_string(fx.root.join("failed/Tax/broken.pdf.error.txt")).unwrap();
    assert!(note.contains("disk on fire"));

    // The quarantined file and its note stay put on later scans.
    let again = source.poll(&sink).await.unwrap();
    assert_eq!(again.failed, 0);
}

#[tokio::test]
async fn busy_sink_keeps_file_for_a_later_delete_import() {
    let fx = Fixture::new();
    let path = fx.write("busy.pdf", b"data");
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink {
        busy_once: Mutex::new(true),
        ..Default::default()
    };

    source.poll(&sink).await.unwrap();
    let busy = source.poll(&sink).await.unwrap();

    assert_eq!(busy.imported, 0);
    assert_eq!(busy.failed, 0);
    assert_eq!(busy.skipped, 0);
    assert_eq!(busy.recheck_in, Some(Duration::from_millis(100)));
    assert!(!path.exists(), "busy input stays in private staging");
    assert!(sink.items.lock().unwrap().is_empty());

    let imported = source.poll(&sink).await.unwrap();

    assert_eq!(imported.imported, 1);
    assert!(!path.exists(), "successful delete import removes the file");
    assert_eq!(sink.items.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn busy_sink_keeps_file_for_a_later_move_import() {
    let fx = Fixture::new();
    let path = fx.write("busy.pdf", b"data");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink {
        busy_once: Mutex::new(true),
        ..Default::default()
    };

    source.poll(&sink).await.unwrap();
    let busy = source.poll(&sink).await.unwrap();

    assert_eq!(busy.imported, 0);
    assert_eq!(busy.failed, 0);
    assert_eq!(busy.skipped, 0);
    assert_eq!(busy.recheck_in, Some(Duration::from_millis(100)));
    assert!(!path.exists(), "busy input stays in private staging");
    assert!(sink.items.lock().unwrap().is_empty());

    let imported = source.poll(&sink).await.unwrap();

    assert_eq!(imported.imported, 1);
    assert!(
        !path.exists(),
        "successful move import removes the inbox entry"
    );
    assert!(fx.root.join("processed/busy.pdf").exists());
    assert_eq!(sink.items.lock().unwrap().len(), 1);
}

struct BlockingSink {
    items: Mutex<Vec<IngestItem>>,
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    fail: bool,
}

impl BlockingSink {
    fn new(fail: bool) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            started: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
            fail,
        }
    }
}

#[async_trait]
impl IngestSink for BlockingSink {
    async fn ingest(&self, _source: Id, item: IngestItem) -> Result<IngestOutcome> {
        self.items.lock().unwrap().push(item);
        self.started.notify_one();
        self.resume.notified().await;
        if self.fail {
            Err(Error::Storage("import failed".into()))
        } else {
            Ok(IngestOutcome::Imported(Box::new(Document::new(
                VaultId::new(),
                "t",
            ))))
        }
    }
}

async fn import_with_replacement(
    after: FolderAfterImport,
    fail: bool,
) -> (Fixture, BlockingSink, PollReport) {
    let fx = Fixture::new();
    let path = fx.write("Tax/scan.pdf", b"first scan");
    let source = fx.source(after);
    let sink = BlockingSink::new(fail);
    source.poll(&sink).await.unwrap();

    let (report, ()) = tokio::join!(source.poll(&sink), async {
        sink.started.notified().await;
        assert!(!path.exists(), "the first scan is claimed before import");
        fx.write("Tax/scan.pdf", b"replacement scan");
        sink.resume.notify_one();
    });

    (fx, sink, report.unwrap())
}

#[tokio::test]
async fn deleting_an_imported_claim_preserves_a_replacement() {
    let (fx, sink, report) = import_with_replacement(FolderAfterImport::Delete, false).await;

    assert_eq!(report.imported, 1);
    assert_eq!(
        std::fs::read(fx.root.join("Tax/scan.pdf")).unwrap(),
        b"replacement scan"
    );
    assert_eq!(sink.items.lock().unwrap()[0].data, b"first scan");
}

#[tokio::test]
async fn moving_an_imported_claim_preserves_a_replacement() {
    let (fx, sink, report) = import_with_replacement(FolderAfterImport::Move, false).await;

    assert_eq!(report.imported, 1);
    assert_eq!(
        std::fs::read(fx.root.join("Tax/scan.pdf")).unwrap(),
        b"replacement scan"
    );
    assert_eq!(
        std::fs::read(fx.root.join("processed/Tax/scan.pdf")).unwrap(),
        b"first scan"
    );
    let items = sink.items.lock().unwrap();
    assert_eq!(items[0].filename, "scan.pdf");
    assert_eq!(items[0].tags, ["Tax"]);
    assert_eq!(
        items[0].source.folder.as_ref().unwrap().relative_path,
        "Tax/scan.pdf"
    );
}

#[tokio::test]
async fn quarantining_a_failed_claim_preserves_a_replacement() {
    let (fx, _, report) = import_with_replacement(FolderAfterImport::Move, true).await;

    assert_eq!(report.failed, 1);
    assert_eq!(
        std::fs::read(fx.root.join("Tax/scan.pdf")).unwrap(),
        b"replacement scan"
    );
    assert_eq!(
        std::fs::read(fx.root.join("failed/Tax/scan.pdf")).unwrap(),
        b"first scan"
    );
    assert!(
        std::fs::read_to_string(fx.root.join("failed/Tax/scan.pdf.error.txt"))
            .unwrap()
            .contains("import failed")
    );
}

#[tokio::test]
async fn a_busy_claim_survives_source_restart_beside_a_replacement() {
    let fx = Fixture::new();
    fx.write("Tax/scan.pdf", b"first scan");
    let source = fx.source(FolderAfterImport::Delete);
    let busy_sink = RecordingSink {
        busy_once: Mutex::new(true),
        ..Default::default()
    };
    source.poll(&busy_sink).await.unwrap();
    assert!(source.poll(&busy_sink).await.unwrap().recheck_in.is_some());
    drop(source);
    fx.write("Tax/scan.pdf", b"replacement scan");
    let restarted = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    let recovered = restarted.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 1);
    assert_eq!(
        std::fs::read(fx.root.join("Tax/scan.pdf")).unwrap(),
        b"replacement scan"
    );
    {
        let items = sink.items.lock().unwrap();
        assert_eq!(items[0].data, b"first scan");
        assert_eq!(items[0].filename, "scan.pdf");
        assert_eq!(items[0].tags, ["Tax"]);
        assert_eq!(
            items[0].source.folder.as_ref().unwrap().relative_path,
            "Tax/scan.pdf"
        );
    }

    assert_eq!(restarted.poll(&sink).await.unwrap().imported, 1);
    assert_eq!(sink.items.lock().unwrap()[1].data, b"replacement scan");
}

#[tokio::test]
async fn concurrent_sources_do_not_recover_an_active_claim() {
    let fx = Fixture::new();
    fx.write("scan.pdf", b"first scan");
    let source = fx.source(FolderAfterImport::Delete);
    let other = fx.source(FolderAfterImport::Delete);
    let blocking = BlockingSink::new(false);
    let sink = RecordingSink::default();
    source.poll(&blocking).await.unwrap();

    let (imported, ()) = tokio::join!(source.poll(&blocking), async {
        blocking.started.notified().await;
        let blocked = other.poll(&sink).await.unwrap();
        assert_eq!(blocked.imported, 0);
        assert!(blocked.recheck_in.is_some());
        assert!(sink.items.lock().unwrap().is_empty());
        blocking.resume.notify_one();
    });

    assert_eq!(imported.unwrap().imported, 1);
    assert_eq!(other.poll(&sink).await.unwrap().imported, 0);
    assert!(sink.items.lock().unwrap().is_empty());
}

#[tokio::test]
async fn moving_a_claim_does_not_overwrite_an_existing_destination() {
    let fx = Fixture::new();
    fx.write("scan.pdf", b"new scan");
    fx.write("processed/scan.pdf", b"previous scan");
    let source = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();
    source.poll(&sink).await.unwrap();

    assert_eq!(source.poll(&sink).await.unwrap().imported, 1);

    assert_eq!(
        std::fs::read(fx.root.join("processed/scan.pdf")).unwrap(),
        b"previous scan"
    );
    assert_eq!(
        std::fs::read(fx.root.join("processed/scan-1.pdf")).unwrap(),
        b"new scan"
    );
}

#[tokio::test]
async fn a_cancelled_import_is_recovered_without_touching_a_replacement() {
    let fx = Fixture::new();
    fx.write("Tax/scan.pdf", b"first scan");
    let source = fx.source(FolderAfterImport::Move);
    let blocking = BlockingSink::new(false);
    source.poll(&blocking).await.unwrap();

    tokio::select! {
        result = source.poll(&blocking) => panic!("import unexpectedly finished: {result:?}"),
        () = blocking.started.notified() => {},
    }
    drop(source);
    fx.write("Tax/scan.pdf", b"replacement scan");
    let restarted = fx.source(FolderAfterImport::Move);
    let sink = RecordingSink::default();

    let recovered = restarted.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 1);
    assert_eq!(sink.items.lock().unwrap()[0].data, b"first scan");
    assert_eq!(
        std::fs::read(fx.root.join("processed/Tax/scan.pdf")).unwrap(),
        b"first scan"
    );
    assert_eq!(
        std::fs::read(fx.root.join("Tax/scan.pdf")).unwrap(),
        b"replacement scan"
    );
}

#[tokio::test]
async fn recovery_preserves_unexpected_files_beside_the_claimed_input() {
    let fx = Fixture::new();
    let stage = format!(".folio-staging/{}", Id::new_v4());
    fx.write(&format!("{stage}/.origin.json"), br#""Tax/a.pdf""#);
    let first = fx.write(&format!("{stage}/Tax/a.pdf"), b"first scan");
    let second = fx.write(&format!("{stage}/Tax/b.pdf"), b"second scan");
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    let recovered = source.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 1);
    assert_eq!(recovered.failed, 0);
    assert_eq!(names(&sink), ["a.pdf"]);
    assert_eq!(sink.items.lock().unwrap()[0].data, b"first scan");
    assert!(!first.exists());
    assert_eq!(std::fs::read(second).unwrap(), b"second scan");
    assert!(fx.root.join(stage).exists());
    assert_eq!(source.poll(&sink).await.unwrap().imported, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_ignores_staged_file_and_directory_symlinks() {
    let fx = Fixture::new();
    let outside = Fixture::new();
    let external = outside.write("scan.pdf", b"outside staging");
    let stage = fx
        .root
        .join(".folio-staging")
        .join(Id::new_v4().to_string());
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join(".origin.json"), br#""link.pdf""#).unwrap();
    std::os::unix::fs::symlink(&external, stage.join("link.pdf")).unwrap();
    let directory_stage = fx
        .root
        .join(".folio-staging")
        .join(Id::new_v4().to_string());
    std::fs::create_dir(&directory_stage).unwrap();
    std::fs::write(
        directory_stage.join(".origin.json"),
        br#""linked-folder/scan.pdf""#,
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside.root, directory_stage.join("linked-folder")).unwrap();
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    let recovered = source.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 0);
    assert_eq!(recovered.failed, 0);
    assert!(sink.items.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(external).unwrap(), b"outside staging");
    assert!(stage
        .join("link.pdf")
        .symlink_metadata()
        .unwrap()
        .is_symlink());
    assert!(directory_stage
        .join("linked-folder")
        .symlink_metadata()
        .unwrap()
        .is_symlink());
}

#[tokio::test]
async fn recovery_does_not_import_children_of_a_claim_replaced_with_a_directory() {
    let fx = Fixture::new();
    let stage = format!(".folio-staging/{}", Id::new_v4());
    fx.write(&format!("{stage}/.origin.json"), br#""Tax/scan.pdf""#);
    let child = fx.write(
        &format!("{stage}/Tax/scan.pdf/child.pdf"),
        b"unexpected child",
    );
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    let recovered = source.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 0);
    assert!(sink.items.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(child).unwrap(), b"unexpected child");
}

#[tokio::test]
async fn recovery_rejects_origins_outside_the_claim_directory() {
    let fx = Fixture::new();
    let stage = format!(".folio-staging/{}", Id::new_v4());
    fx.write(&format!("{stage}/.origin.json"), br#""../../external.pdf""#);
    let external = fx.write("external.pdf", b"outside claim");
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    let recovered = source.poll(&sink).await.unwrap();

    assert_eq!(recovered.imported, 0);
    assert!(sink.items.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(external).unwrap(), b"outside claim");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_non_utf8_filename_is_retained_when_its_origin_cannot_be_recorded() {
    use std::os::unix::ffi::OsStringExt;

    let fx = Fixture::new();
    let path = fx
        .root
        .join(std::ffi::OsString::from_vec(b"scan-\xff.pdf".to_vec()));
    std::fs::write(&path, b"first scan").unwrap();
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();
    source.poll(&sink).await.unwrap();

    let report = source.poll(&sink).await.unwrap();

    assert_eq!(report.imported, 0);
    assert!(report.recheck_in.is_some());
    assert!(sink.items.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"first scan");
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_staging_root_leaves_input_and_external_files_untouched() {
    let fx = Fixture::new();
    let input = fx.write("scan.pdf", b"source input");
    let outside = Fixture::new();
    let external = outside.write("scan.pdf", b"external input");
    std::os::unix::fs::symlink(&outside.root, fx.root.join(".folio-staging")).unwrap();
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink::default();

    assert!(source.poll(&sink).await.is_err());

    assert!(sink.items.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(input).unwrap(), b"source input");
    assert_eq!(std::fs::read(external).unwrap(), b"external input");
    assert_eq!(std::fs::read_dir(&outside.root).unwrap().count(), 1);
}

#[tokio::test]
async fn folder_item_identity_survives_restart_and_distinguishes_replacement() {
    let fx = Fixture::new();
    fx.write("same.pdf", b"same bytes");
    let source = fx.source(FolderAfterImport::Delete);
    let sink = RecordingSink {
        busy_once: Mutex::new(true),
        ..Default::default()
    };
    source.poll(&sink).await.unwrap();
    source.poll(&sink).await.unwrap();
    drop(source);
    fx.write("same.pdf", b"same bytes");
    let restarted = fx.source(FolderAfterImport::Delete);

    restarted.poll(&sink).await.unwrap();
    restarted.poll(&sink).await.unwrap();
    restarted.poll(&sink).await.unwrap();

    let observations = sink.observations.lock().unwrap();
    assert_eq!(observations.len(), 3);
    assert_eq!(observations[0], observations[1]);
    assert_ne!(observations[1], observations[2]);
}
