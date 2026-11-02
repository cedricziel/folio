//! Consume folder: imports files that appear in a watched directory

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::Notify;
use tracing::{debug, warn};

use folio_core::error::{Error, Result};
use folio_core::models::{
    DocumentSource, FolderAfterImport, FolderConfig, FolderOrigin, Id, IngestItem, IngestOutcome,
    PollReport,
};
use folio_core::ports::{IngestSink, IngestionSource};

use super::mime::guess_mime;

/// Directories inside the consume folder that Folio writes to
const PROCESSED: &str = "processed";
const FAILED: &str = "failed";
const SKIPPED: &str = "skipped";

/// File events arrive in bursts while a file is being written
const WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Suffixes of files that are still being written or are editor leftovers
const TEMP_SUFFIXES: [&str; 12] = [
    ".part",
    ".partial",
    ".tmp",
    ".temp",
    ".crdownload",
    ".download",
    ".filepart",
    ".opdownload",
    ".!qb",
    ".swp",
    ".swx",
    "~",
];

/// Whether a file name belongs to something other than a finished document
pub fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || matches!(lower.as_str(), "thumbs.db" | "desktop.ini")
        || TEMP_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    modified: Option<SystemTime>,
}

struct Observation {
    fingerprint: Fingerprint,
    stable_since: Instant,
    seen: u32,
}

struct Found {
    path: PathBuf,
    fingerprint: Fingerprint,
}

pub struct FolderSource {
    id: Id,
    name: String,
    config: FolderConfig,
    settle: Duration,
    observations: Mutex<HashMap<PathBuf, Observation>>,
    wake: Arc<Notify>,
    _watcher: Option<RecommendedWatcher>,
}

impl FolderSource {
    pub fn new(id: Id, name: &str, config: FolderConfig) -> Self {
        let wake = Arc::new(Notify::new());
        let watcher = start_watcher(&config, Arc::clone(&wake));
        Self {
            id,
            name: name.to_string(),
            settle: Duration::from_secs(config.settle_secs),
            config,
            observations: Mutex::new(HashMap::new()),
            wake,
            _watcher: watcher,
        }
    }

    /// Override how long a file has to stay unchanged (tests use milliseconds)
    pub fn with_settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }

    fn root(&self) -> &Path {
        Path::new(&self.config.path)
    }

    /// Record what is on disk and return the files that stopped changing
    fn settle_files(&self, found: Vec<Found>) -> (Vec<Found>, bool) {
        let now = Instant::now();
        let mut observations = self.observations.lock().unwrap_or_else(|e| e.into_inner());
        observations.retain(|path, _| found.iter().any(|f| &f.path == path));

        let mut ready = Vec::new();
        let mut waiting = false;
        for file in found {
            let entry = observations
                .entry(file.path.clone())
                .and_modify(|o| {
                    if o.fingerprint == file.fingerprint {
                        o.seen += 1;
                    } else {
                        *o = Observation {
                            fingerprint: file.fingerprint,
                            stable_since: now,
                            seen: 1,
                        };
                    }
                })
                .or_insert(Observation {
                    fingerprint: file.fingerprint,
                    stable_since: now,
                    seen: 1,
                });
            // Seen twice with the same size: a copy still in flight changes between looks.
            if entry.seen >= 2
                && file.fingerprint.size > 0
                && now.duration_since(entry.stable_since) >= self.settle
            {
                ready.push(file);
            } else {
                waiting = true;
            }
        }
        (ready, waiting)
    }

    fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(self.root()).unwrap_or(path).to_path_buf()
    }

    async fn handle(&self, file: &Found, sink: &dyn IngestSink) -> Result<IngestOutcome> {
        let relative = self.relative(&file.path);
        let original = relative
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let data = tokio::fs::read(&file.path)
            .await
            .map_err(|e| Error::Storage(format!("Cannot read {}: {e}", file.path.display())))?;

        let tags = if self.config.subfolders_as_tags {
            relative
                .parent()
                .into_iter()
                .flat_map(|p| p.components())
                .filter_map(|c| match c {
                    Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };

        sink.ingest(
            self.id,
            IngestItem {
                mime_type: guess_mime(&original).to_string(),
                source: DocumentSource::folder(
                    &self.name,
                    FolderOrigin {
                        relative_path: relative.to_string_lossy().replace('\\', "/"),
                        original_filename: original.clone(),
                    },
                ),
                filename: original,
                data,
                tags,
            },
        )
        .await
    }

    async fn after_success(&self, path: &Path) -> std::io::Result<()> {
        match self.config.after_import {
            FolderAfterImport::Delete => tokio::fs::remove_file(path).await,
            FolderAfterImport::Move => self.move_into(PROCESSED, path).await.map(|_| ()),
        }
    }

    /// Move a file below one of the reserved directories, keeping its relative path
    async fn move_into(&self, bucket: &str, path: &Path) -> std::io::Result<PathBuf> {
        let mut target = self.root().join(bucket).join(self.relative(path));
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut n = 1;
        while tokio::fs::try_exists(&target).await? {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (name.clone(), String::new()),
            };
            target.set_file_name(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        tokio::fs::rename(path, &target).await?;
        Ok(target)
    }

    async fn quarantine(&self, path: &Path, error: &Error) {
        match self.move_into(FAILED, path).await {
            Ok(target) => {
                let mut note = target.clone().into_os_string();
                note.push(".error.txt");
                if let Err(e) = tokio::fs::write(&note, format!("{error}\n")).await {
                    warn!("Could not write error note for {}: {e}", target.display());
                }
            }
            Err(e) => warn!("Could not move {} to failed/: {e}", path.display()),
        }
    }
}

fn start_watcher(config: &FolderConfig, wake: Arc<Notify>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            wake.notify_one();
        }
    })
    .map_err(|e| warn!("File watcher unavailable, relying on rescans: {e}"))
    .ok()?;
    let mode = if config.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher
        .watch(Path::new(&config.path), mode)
        .map_err(|e| warn!("Cannot watch {}: {e}", config.path))
        .ok()?;
    Some(watcher)
}

/// Walk the consume folder, skipping hidden entries, temp files, and Folio's own directories
fn scan(root: &Path, recursive: bool) -> std::io::Result<Vec<Found>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == root => return Err(e),
            Err(e) => {
                warn!("Skipping unreadable folder {}: {e}", dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                let reserved = dir == root && matches!(name.as_str(), PROCESSED | FAILED | SKIPPED);
                if recursive && !reserved && !name.starts_with('.') {
                    pending.push(path);
                }
            } else if file_type.is_file() && !is_ignored_name(&name) {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                found.push(Found {
                    path,
                    fingerprint: Fingerprint {
                        size: meta.len(),
                        modified: meta.modified().ok(),
                    },
                });
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

#[async_trait]
impl IngestionSource for FolderSource {
    async fn poll(&self, sink: &dyn IngestSink) -> Result<PollReport> {
        let root = self.root().to_path_buf();
        let recursive = self.config.recursive;
        let found = tokio::task::spawn_blocking(move || scan(&root, recursive))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
            .map_err(|e| Error::Storage(format!("Cannot scan {}: {e}", self.config.path)))?;

        let (ready, waiting) = self.settle_files(found);
        let mut report = PollReport::default();
        let mut busy = false;
        for file in ready {
            match self.handle(&file, sink).await {
                Ok(IngestOutcome::Busy) => {
                    debug!("{} is busy in the ingest pipeline", file.path.display());
                    busy = true;
                }
                Ok(IngestOutcome::Skipped { rule }) => {
                    debug!("{} skipped by rule '{rule}'", file.path.display());
                    report.skipped += 1;
                    if let Err(e) = self.move_into(SKIPPED, &file.path).await {
                        warn!("Could not move {} to skipped/: {e}", file.path.display());
                    }
                }
                Ok(outcome) => {
                    if matches!(outcome, IngestOutcome::Imported(_)) {
                        report.imported += 1;
                    } else {
                        report.skipped += 1;
                    }
                    if let Err(e) = self.after_success(&file.path).await {
                        warn!("Could not clean up {}: {e}", file.path.display());
                    }
                }
                Err(e) => {
                    warn!("Importing {} failed: {e}", file.path.display());
                    report.failed += 1;
                    self.quarantine(&file.path, &e).await;
                }
            }
        }
        if waiting || busy {
            report.recheck_in =
                Some((self.settle / 2).clamp(Duration::from_millis(100), Duration::from_secs(2)));
        }
        Ok(report)
    }

    async fn wake(&self) {
        self.wake.notified().await;
        tokio::time::sleep(WATCH_DEBOUNCE).await;
    }

    async fn test(&self) -> Result<String> {
        let root = self.root().to_path_buf();
        let count = tokio::task::spawn_blocking(move || scan(&root, true).map(|f| f.len()))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
            .map_err(|e| Error::Storage(format!("Cannot read {}: {e}", self.config.path)))?;
        Ok(format!("{count} file(s) waiting in {}", self.config.path))
    }

    fn interval_secs(&self) -> u64 {
        self.config.rescan_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_hidden_temporary_and_partial_files() {
        for name in [
            ".hidden",
            "~$report.docx",
            "scan.pdf.part",
            "download.crdownload",
            "notes.txt~",
            "x.TMP",
            "Thumbs.db",
            ".DS_Store",
        ] {
            assert!(is_ignored_name(name), "{name}");
        }
        for name in ["invoice.pdf", "a b.PNG", "report~final.pdf"] {
            assert!(!is_ignored_name(name), "{name}");
        }
    }
}
