use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;

use folio_core::ports::PaperlessReadError;
use folio_infra::import::resolve_original;

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
        assert_unsafe(resolve_original(&export, path));
    }
}

#[test]
fn original_guard_refuses_symlinks_without_opening_them() {
    let layout = Layout::new();
    let export = layout.path("imports/export");

    let secret = layout.path("outside/secret.pdf");
    fs::write(&secret, b"secret").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    symlink(&secret, export.join("linked.pdf")).unwrap();
    symlink(layout.path("outside"), export.join("dir")).unwrap();
    fs::write(export.join("inner.pdf"), b"inner").unwrap();
    symlink(export.join("inner.pdf"), export.join("internal-link.pdf")).unwrap();

    assert_unsafe(resolve_original(&export, "linked.pdf"));
    assert_unsafe(resolve_original(&export, "dir/secret.pdf"));
    assert_unsafe(resolve_original(&export, "internal-link.pdf"));
}

#[test]
fn original_guard_resolves_plain_files_and_reports_missing_ones() {
    let layout = Layout::new();
    let export = layout.path("imports/export");
    fs::create_dir_all(export.join("originals/2024")).unwrap();
    fs::write(export.join("originals/2024/a b.pdf"), b"pdf").unwrap();

    assert_eq!(
        resolve_original(&export, "originals/2024/a b.pdf").unwrap(),
        export.join("originals/2024/a b.pdf")
    );
    assert!(matches!(
        resolve_original(&export, "originals/missing.pdf"),
        Err(PaperlessReadError::Missing { .. })
    ));
    assert_unsafe(resolve_original(&export, "originals"));
}
