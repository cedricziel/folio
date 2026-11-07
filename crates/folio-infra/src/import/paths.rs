//! Path checks for import sources

use std::fs::{File, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use folio_core::ports::PaperlessReadError;

/// One error for paths outside every root and for paths that don't exist, so
/// callers can't probe which server paths exist
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the path is not an existing directory inside an allowed import root")]
pub struct OutsideAllowedRoots;

/// Resolves `..` and symlinks in `path`, then requires the result to sit
/// inside one of `roots`, compared component by component
pub fn resolve_import_dir(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, OutsideAllowedRoots> {
    if !path.is_absolute() {
        return Err(OutsideAllowedRoots);
    }
    let resolved = path.canonicalize().map_err(|_| OutsideAllowedRoots)?;
    let inside = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| resolved.starts_with(root));
    if inside && resolved.is_dir() {
        Ok(resolved)
    } else {
        Err(OutsideAllowedRoots)
    }
}

/// Resolves a manifest path under the canonical export directory without
/// opening anything, returning the file's metadata. Absolute paths, `.` or
/// `..` components, symlinks on any component and anything but a regular
/// file are refused.
pub fn resolve_original(
    export_dir: &Path,
    relative: &str,
) -> Result<(PathBuf, Metadata), PaperlessReadError> {
    let unsafe_path = || PaperlessReadError::UnsafePath {
        what: relative.to_owned(),
    };
    let components = Path::new(relative).components();
    if relative.is_empty()
        || !components
            .clone()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(unsafe_path());
    }

    let mut resolved = export_dir.to_path_buf();
    let mut last = None;
    for component in components {
        resolved.push(component);
        let meta = std::fs::symlink_metadata(&resolved).map_err(|e| io_error(relative, e))?;
        if meta.file_type().is_symlink() {
            return Err(unsafe_path());
        }
        last = Some(meta);
    }
    match last {
        Some(meta) if meta.is_file() => Ok((resolved, meta)),
        _ => Err(unsafe_path()),
    }
}

/// Opens a file checked by [`resolve_original`], refusing it when the path
/// was swapped between the check and the open. Returns the opened file's size.
pub fn open_original_file(
    export_dir: &Path,
    relative: &str,
) -> Result<(File, u64), PaperlessReadError> {
    let (path, checked) = resolve_original(export_dir, relative)?;
    let file = File::open(path).map_err(|e| io_error(relative, e))?;
    let opened = file.metadata().map_err(|e| io_error(relative, e))?;
    if !opened.is_file() || opened.dev() != checked.dev() || opened.ino() != checked.ino() {
        return Err(PaperlessReadError::UnsafePath {
            what: relative.to_owned(),
        });
    }
    Ok((file, opened.len()))
}

pub(super) fn io_error(what: &str, source: io::Error) -> PaperlessReadError {
    if source.kind() == io::ErrorKind::NotFound {
        PaperlessReadError::Missing {
            what: what.to_owned(),
        }
    } else {
        PaperlessReadError::Io {
            what: what.to_owned(),
            source,
        }
    }
}
