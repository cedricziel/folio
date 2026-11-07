//! Path checks for import sources

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use folio_core::ports::PaperlessReadError;
use rustix::fs::{openat, Mode, OFlags};
use rustix::io::Errno;

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

/// Opens a manifest path under the export directory one component at a time,
/// each relative to the directory opened before it and none followed if it is
/// a symlink, so swapping a component mid-walk can't redirect the open.
/// Returns the opened file and its size.
pub fn open_original_file(
    export_dir: &Path,
    relative: &str,
) -> Result<(File, u64), PaperlessReadError> {
    let unsafe_path = || PaperlessReadError::UnsafePath {
        what: relative.to_owned(),
    };
    let components = normal_components(relative).ok_or_else(unsafe_path)?;
    let open_error = |errno: Errno| match errno {
        Errno::NOENT => PaperlessReadError::Missing {
            what: relative.to_owned(),
        },
        Errno::LOOP | Errno::NOTDIR | Errno::MLINK => unsafe_path(),
        errno => io_error(relative, errno.into()),
    };

    let (last, parents) = components.split_last().ok_or_else(unsafe_path)?;
    let mut dir = File::open(export_dir).map_err(|e| io_error(relative, e))?;
    for name in parents {
        let fd = openat(
            &dir,
            *name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(open_error)?;
        dir = File::from(fd);
    }
    // Non-blocking so a FIFO planted in the export can't stall the open
    let fd = openat(
        &dir,
        *last,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(open_error)?;
    let file = File::from(fd);
    let meta = file.metadata().map_err(|e| io_error(relative, e))?;
    if !meta.is_file() {
        return Err(unsafe_path());
    }
    Ok((file, meta.len()))
}

fn normal_components(relative: &str) -> Option<Vec<&OsStr>> {
    let mut names = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(name) => names.push(name),
            _ => return None,
        }
    }
    (!names.is_empty()).then_some(names)
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
