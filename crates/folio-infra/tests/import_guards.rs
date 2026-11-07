use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use folio_core::ports::{PaperlessReadError, PaperlessReader, PaperlessRecord};
use folio_infra::import::{open_original_file, resolve_import_dir, PaperlessFsReader};
use futures::TryStreamExt;

struct Layout {
    _tmp: tempfile::TempDir,
    base: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        for dir in ["imports/export", "imports-x/export", "outside"] {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        symlink(base.join("outside"), base.join("imports/escape")).unwrap();
        symlink(base.join("imports/export"), base.join("imports/alias")).unwrap();
        Self { _tmp: tmp, base }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.base.join(relative)
    }

    /// An unreadable file outside the root, so a guard that opened it would fail differently
    fn outside_secret(&self) -> PathBuf {
        let secret = self.path("outside/secret.pdf");
        fs::write(&secret, b"secret").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        secret
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.path("imports")]
    }
}

#[test]
fn root_guard_accepts_paths_inside_a_root() {
    let layout = Layout::new();
    let roots = layout.roots();
    assert_eq!(
        resolve_import_dir(&layout.path("imports/export"), &roots)
            .unwrap()
            .path(),
        layout.path("imports/export")
    );
    assert_eq!(
        resolve_import_dir(&layout.path("imports/alias"), &roots)
            .unwrap()
            .path(),
        layout.path("imports/export")
    );
}

#[test]
fn root_guard_rejects_escapes_siblings_and_missing_paths_alike() {
    let layout = Layout::new();
    let roots = layout.roots();
    let errors: Vec<_> = [
        layout.path("imports/../outside"),
        layout.path("imports/escape"),
        layout.path("imports-x/export"),
        layout.path("imports/missing"),
        layout.path("outside/missing"),
        PathBuf::from("imports/export"),
    ]
    .iter()
    .map(|path| resolve_import_dir(path, &roots).unwrap_err().to_string())
    .collect();
    assert!(errors.iter().all(|e| *e == errors[0]), "{errors:?}");
    assert!(!errors[0].contains(layout.base.to_str().unwrap()));
}

#[test]
fn root_guard_with_no_roots_rejects_everything() {
    let layout = Layout::new();
    assert!(resolve_import_dir(&layout.path("imports/export"), &[]).is_err());
}

fn reader_at(dir: &Path) -> PaperlessFsReader {
    PaperlessFsReader::new(
        resolve_import_dir(dir, std::slice::from_ref(&dir.to_path_buf())).unwrap(),
    )
}

fn open_in(export: &Path, relative: &str) -> Result<(fs::File, u64), PaperlessReadError> {
    open_original_file(&fs::File::open(export).unwrap(), relative)
}

fn assert_unsafe(result: Result<impl Sized, PaperlessReadError>) {
    assert!(
        matches!(result, Err(PaperlessReadError::UnsafePath { .. })),
        "expected unsafe_path"
    );
}

#[test]
fn original_guard_refuses_absolute_and_parent_paths() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    for path in [
        "../etc/passwd",
        "/etc/passwd",
        "a/../../outside",
        "",
        "./x.pdf",
    ] {
        assert_unsafe(open_in(&export, path));
    }
}

#[test]
fn original_guard_refuses_symlinks_without_opening_them() {
    let layout = Layout::new();
    let export = layout.path("imports/export");

    let secret = layout.outside_secret();
    symlink(&secret, export.join("linked.pdf")).unwrap();
    symlink(layout.path("outside"), export.join("dir")).unwrap();
    fs::write(export.join("inner.pdf"), b"inner").unwrap();
    symlink(export.join("inner.pdf"), export.join("internal-link.pdf")).unwrap();
    fs::create_dir_all(export.join("real/sub")).unwrap();
    fs::write(export.join("real/sub/doc.pdf"), b"doc").unwrap();
    symlink(export.join("real"), export.join("swapped")).unwrap();

    assert_unsafe(open_in(&export, "linked.pdf"));
    assert_unsafe(open_in(&export, "dir/secret.pdf"));
    assert_unsafe(open_in(&export, "internal-link.pdf"));
    // A directory swapped for a symlink mid-path is refused even when it points inside
    assert_unsafe(open_in(&export, "swapped/sub/doc.pdf"));
}

#[test]
fn original_guard_resolves_plain_files_and_reports_missing_ones() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    fs::create_dir_all(export.join("originals/2024")).unwrap();
    fs::write(export.join("originals/2024/a b.pdf"), b"pdf").unwrap();

    let (_, size) = open_in(&export, "originals/2024/a b.pdf").unwrap();
    assert_eq!(size, 3);
    assert!(matches!(
        open_in(&export, "originals/missing.pdf"),
        Err(PaperlessReadError::Missing { .. })
    ));
    assert_unsafe(open_in(&export, "originals"));
}

/// Paperless writes `model` before `fields`; `json!` would sort them the other way
fn manifest_with_originals(handles: &[&str]) -> String {
    let docs: Vec<_> = handles
        .iter()
        .enumerate()
        .map(|(i, handle)| {
            format!(
                r#"{{"model": "documents.document", "pk": {}, "fields": {{"title": "t", "content": "", "mime_type": "application/pdf", "checksum": "{}", "created": "2024-01-01", "added": "2024-01-01T00:00:00Z", "modified": "2024-01-01T00:00:00Z", "tags": []}}, "__exported_file_name__": {}}}"#,
                i + 1,
                "0".repeat(32),
                serde_json::to_string(handle).unwrap()
            )
        })
        .collect();
    format!("[{}]", docs.join(","))
}

#[tokio::test]
async fn reader_refuses_unsafe_manifest_paths_without_opening_them() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    let secret = layout.outside_secret();
    symlink(&secret, export.join("linked.pdf")).unwrap();
    fs::write(
        export.join("manifest.json"),
        manifest_with_originals(&["linked.pdf", "../outside/secret.pdf", "/etc/passwd"]),
    )
    .unwrap();

    let reader = reader_at(&export);
    let records: Vec<_> = reader.scan().await.unwrap().try_collect().await.unwrap();
    assert_eq!(records.len(), 3);
    for record in &records {
        let PaperlessRecord::Document(doc) = record else {
            panic!("expected a document");
        };
        assert_eq!(doc.size, None);
        assert_unsafe(reader.open_original(doc).await.map(|_| ()));
    }
}

#[tokio::test]
async fn reader_refuses_a_symlinked_manifest() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    fs::write(layout.path("outside/manifest.json"), b"[]").unwrap();
    symlink(
        layout.path("outside/manifest.json"),
        export.join("manifest.json"),
    )
    .unwrap();
    let reader = reader_at(&export);
    assert_unsafe(reader.scan().await.map(|_| ()));
}

#[tokio::test]
async fn reader_keeps_reading_the_directory_it_opened_after_the_path_is_replaced() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    fs::write(export.join("metadata.json"), r#"{"version": "3.4.2"}"#).unwrap();
    fs::write(
        layout.path("outside/metadata.json"),
        r#"{"version": "2.0.0"}"#,
    )
    .unwrap();

    let reader = reader_at(&export);
    fs::rename(&export, layout.path("imports/moved")).unwrap();
    symlink(layout.path("outside"), &export).unwrap();

    let info = reader.source_info().await.unwrap();
    assert_eq!(info.version.to_string(), "3.4.2");
}
