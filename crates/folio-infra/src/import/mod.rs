//! Readers for libraries imported from other document systems

mod paperless_fs;
mod paperless_http;
mod paths;

pub use paperless_fs::PaperlessFsReader;
pub use paperless_http::{InvalidPaperlessUrl, PaperlessHttpReader};
pub use paths::{open_original_file, resolve_import_dir, ImportDir, OutsideAllowedRoots};
