//! Path checks for import sources

use std::path::{Path, PathBuf};

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
