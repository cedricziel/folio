use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use folio_infra::import::resolve_import_dir;

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

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.path("imports")]
    }
}

#[test]
fn root_guard_accepts_paths_inside_a_root() {
    let layout = Layout::new();
    let roots = layout.roots();
    assert_eq!(
        resolve_import_dir(&layout.path("imports/export"), &roots).unwrap(),
        layout.path("imports/export")
    );
    assert_eq!(
        resolve_import_dir(&layout.path("imports/alias"), &roots).unwrap(),
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
