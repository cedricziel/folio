//! Ingestion adapters: consume folder and IMAP mailboxes

mod email;
mod folder;
mod mime;

pub use email::{parse_mail, EmailSource, ParsedMail};
pub use folder::{is_ignored_name, FolderSource};
pub use mime::guess_mime;

use std::sync::Arc;

use folio_core::error::Result;
use folio_core::models::{IngestionSourceDef, SourceConfig};
use folio_core::ports::{IngestLedger, IngestionSource, IngestionSourceFactory};

/// Builds folder and mailbox adapters from source definitions
pub struct SourceFactory {
    ledger: Arc<dyn IngestLedger>,
}

impl SourceFactory {
    pub fn new(ledger: Arc<dyn IngestLedger>) -> Self {
        Self { ledger }
    }
}

impl IngestionSourceFactory for SourceFactory {
    fn build(&self, def: &IngestionSourceDef) -> Result<Box<dyn IngestionSource>> {
        def.config.validate()?;
        Ok(match &def.config {
            SourceConfig::Folder(config) => {
                Box::new(FolderSource::new(def.id, &def.name, config.clone()))
            }
            SourceConfig::Email(config) => Box::new(EmailSource::new(
                def.id,
                &def.name,
                config.clone(),
                Arc::clone(&self.ledger),
            )),
        })
    }
}
