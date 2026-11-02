//! The consume folder adapter against a real directory.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Document, FolderAfterImport, FolderConfig, Id, IngestItem, IngestOutcome, VaultId,
};
use folio_core::ports::{IngestSink, IngestionSource};
use folio_infra::FolderSource;

#[derive(Default)]
struct RecordingSink {
    items: Mutex<Vec<IngestItem>>,
    fail_with: Option<String>,
    busy_once: Mutex<bool>,
}

#[async_trait]
impl IngestSink for RecordingSink {
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
    assert!(path.exists(), "busy input remains in the consume folder");
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
    assert!(path.exists(), "busy input remains in the consume folder");
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
