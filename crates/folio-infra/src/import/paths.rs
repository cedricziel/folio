//! Path checks for import sources

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

use folio_core::ports::PaperlessReadError;
use rustix::fs::{fstat, openat, statat, AtFlags, FileType, Mode, OFlags, Stat};
use rustix::io::Errno;

/// One error for paths outside every root and for paths that don't exist, so
/// callers can't probe which server paths exist
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the path is not an existing directory inside an allowed import root")]
pub struct OutsideAllowedRoots;

/// An export directory inside an allowed root, opened while it was checked
#[derive(Debug)]
pub struct ImportDir {
    path: PathBuf,
    dir: File,
}

impl ImportDir {
    /// The canonical path at the time of the check, for display only
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn into_file(self) -> File {
        self.dir
    }
}

/// Resolves `..` and symlinks in `path`, requires the result to sit inside
/// one of `roots` (compared component by component), and opens it by walking
/// down from the opened root without following symlinks. A directory swapped
/// after the path was resolved is refused, not followed.
pub fn resolve_import_dir(
    path: &Path,
    roots: &[PathBuf],
) -> Result<ImportDir, OutsideAllowedRoots> {
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    if roots.is_empty() || !path.is_absolute() {
        return Err(OutsideAllowedRoots);
    }
    let resolved = path.canonicalize().map_err(|_| OutsideAllowedRoots)?;
    for root in roots {
        let Ok(below) = resolved.strip_prefix(&root) else {
            continue;
        };
        let mut dir = rustix::fs::open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| OutsideAllowedRoots)?;
        for component in below.components() {
            let Component::Normal(name) = component else {
                return Err(OutsideAllowedRoots);
            };
            dir = File::from(open_subdir(&dir, name).map_err(|_| OutsideAllowedRoots)?);
        }
        return Ok(ImportDir {
            path: resolved,
            dir,
        });
    }
    Err(OutsideAllowedRoots)
}

/// Opens a manifest path under the opened export directory one component at
/// a time, each relative to the directory opened before it and none followed
/// if it is a symlink, so swapping a component mid-walk can't redirect the
/// open. The target must be a regular file with a single link both before
/// the open and, as the same inode, after it. Returns the file and its size.
pub fn open_original_file(
    export_dir: &File,
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
    let mut parent: Option<File> = None;
    for name in parents {
        let fd = open_subdir(parent.as_ref().unwrap_or(export_dir), name).map_err(open_error)?;
        parent = Some(File::from(fd));
    }
    let parent = parent.as_ref().unwrap_or(export_dir);

    let before = statat(parent, *last, AtFlags::SYMLINK_NOFOLLOW).map_err(open_error)?;
    if !is_single_regular_file(&before) {
        return Err(unsafe_path());
    }
    // Non-blocking in case the entry was swapped for a FIFO after the check
    let fd = openat(
        parent,
        *last,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(open_error)?;
    let after = fstat(&fd).map_err(open_error)?;
    if !is_single_regular_file(&after)
        || (after.st_dev, after.st_ino) != (before.st_dev, before.st_ino)
    {
        return Err(unsafe_path());
    }
    Ok((File::from(fd), after.st_size as u64))
}

/// A hard link could pull in a file from outside the export
fn is_single_regular_file(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode as _) == FileType::RegularFile && stat.st_nlink == 1
}

/// Opens a directory entry as a directory, refusing a symlink
pub(super) fn open_subdir(parent: &File, name: &OsStr) -> rustix::io::Result<OwnedFd> {
    openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
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
