//! Readers for libraries imported from other document systems

mod paperless_fs;
mod paths;

pub use paperless_fs::PaperlessFsReader;
pub use paths::{open_original_file, resolve_import_dir, OutsideAllowedRoots};
