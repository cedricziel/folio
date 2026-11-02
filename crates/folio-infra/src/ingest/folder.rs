//! Consume folder: imports files that appear in a watched directory

use std::collections::HashMap;
use std::io::Write;
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
const STAGING: &str = ".folio-staging";
const ORIGIN: &str = ".origin.json";

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

struct Claimed {
    path: PathBuf,
    relative: PathBuf,
    directory: PathBuf,
}

pub struct FolderSource {
    id: Id,
    name: String,
    config: FolderConfig,
    settle: Duration,
    observations: Mutex<HashMap<PathBuf, Observation>>,
    poll_lock: tokio::sync::Mutex<()>,
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
            poll_lock: tokio::sync::Mutex::new(()),
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

    fn lock_staging(&self) -> std::io::Result<Option<std::fs::File>> {
        let staging = self.root().join(STAGING);
        match std::fs::create_dir(&staging) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        if !std::fs::symlink_metadata(&staging)?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Folder staging must be a directory, not a symlink or file",
            ));
        }
        // Keep this inode permanently so every source instance locks the same file.
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(staging.join("lock"))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    fn claim(&self, file: Found) -> std::io::Result<Claimed> {
        let relative = file.path.strip_prefix(self.root()).unwrap().to_path_buf();
        let origin = serde_json::to_vec(&relative)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let directory = self.root().join(STAGING).join(Id::new_v4().to_string());
        std::fs::create_dir(&directory)?;
        let path = directory.join(&relative);
        let claimed = Claimed {
            path,
            relative,
            directory,
        };
        let marker = claimed.directory.join(ORIGIN);
        let mut metadata = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)?;
        let result = (|| {
            metadata.write_all(&origin)?;
            metadata.sync_all()?;
            std::fs::create_dir_all(claimed.path.parent().unwrap())?;
            std::fs::rename(&file.path, &claimed.path)
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&marker);
            let _ = prune_staging(&claimed);
            return Err(error);
        }
        self.observations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&file.path);
        Ok(claimed)
    }

    async fn handle(&self, file: &Claimed, sink: &dyn IngestSink) -> Result<IngestOutcome> {
        if !std::fs::symlink_metadata(&file.path)
            .map_err(|e| Error::Storage(format!("Cannot inspect {}: {e}", file.path.display())))?
            .is_file()
        {
            return Err(Error::Storage(format!(
                "Not a regular file: {}",
                file.path.display()
            )));
        }
        let relative = &file.relative;
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

    fn after_success(&self, file: &Claimed) -> std::io::Result<()> {
        match self.config.after_import {
            FolderAfterImport::Delete => std::fs::remove_file(&file.path),
            FolderAfterImport::Move => self.move_into(PROCESSED, file).map(|_| ()),
        }
    }

    fn move_into(&self, bucket: &str, file: &Claimed) -> std::io::Result<PathBuf> {
        let mut target = self.root().join(bucket).join(&file.relative);
        std::fs::create_dir_all(target.parent().unwrap())?;
        let name = file.relative.file_name().unwrap().to_string_lossy();
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.to_string(), String::new()),
        };
        let mut n = 1;
        loop {
            // A hard link reserves the destination atomically without replacing an existing file.
            match std::fs::hard_link(&file.path, &target) {
                Ok(()) => {
                    std::fs::remove_file(&file.path)?;
                    return Ok(target);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    target.set_file_name(format!("{stem}-{n}{ext}"));
                    n += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn quarantine(&self, file: &Claimed, error: &Error) -> std::io::Result<()> {
        let target = self.move_into(FAILED, file)?;
        let mut note = target.clone().into_os_string();
        note.push(".error.txt");
        let write_note = || -> std::io::Result<()> {
            let mut note = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&note)?;
            writeln!(note, "{error}")
        };
        if let Err(e) = write_note() {
            warn!("Could not write error note for {}: {e}", target.display());
        }
        Ok(())
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

fn prune_staging(file: &Claimed) -> std::io::Result<()> {
    let mut parent = file.path.parent();
    while let Some(directory) = parent.filter(|p| p.starts_with(&file.directory)) {
        match std::fs::remove_dir(directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => return Ok(()),
            Err(e) => return Err(e),
        }
        parent = directory.parent();
    }
    Ok(())
}

fn recover_claims(root: &Path) -> std::io::Result<Vec<Claimed>> {
    let mut claimed = Vec::new();
    for entry in std::fs::read_dir(root.join(STAGING))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || Id::parse_str(&entry.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        let directory = entry.path();
        let marker = directory.join(ORIGIN);
        if !marker.symlink_metadata().is_ok_and(|m| m.is_file()) {
            continue;
        }
        let relative: PathBuf = match std::fs::read(&marker).and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }) {
            Ok(relative) => relative,
            Err(e) => {
                warn!("Cannot recover staging origin {}: {e}", marker.display());
                continue;
            }
        };
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            warn!("Ignoring invalid staging origin {}", marker.display());
            continue;
        }
        let path = directory.join(&relative);
        // Scanning excludes symlinks at every level, including parents of the intended file.
        if scan(&directory, true)?.iter().any(|file| file.path == path) {
            claimed.push(Claimed {
                path,
                relative,
                directory,
            });
        }
    }
    claimed.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(claimed)
}

#[async_trait]
impl IngestionSource for FolderSource {
    async fn poll(&self, sink: &dyn IngestSink) -> Result<PollReport> {
        let _poll = self.poll_lock.lock().await;
        let recheck = (self.settle / 2).clamp(Duration::from_millis(100), Duration::from_secs(2));
        let Some(_staging_lock) = self
            .lock_staging()
            .map_err(|e| Error::Storage(format!("Cannot lock {}: {e}", self.config.path)))?
        else {
            return Ok(PollReport {
                recheck_in: Some(recheck),
                ..PollReport::default()
            });
        };
        let root = self.root().to_path_buf();
        let recursive = self.config.recursive;
        let (found, mut claimed) = tokio::task::spawn_blocking(move || {
            Ok::<_, std::io::Error>((scan(&root, recursive)?, recover_claims(&root)?))
        })
        .await
        .map_err(|e| Error::Storage(e.to_string()))?
        .map_err(|e| Error::Storage(format!("Cannot scan {}: {e}", self.config.path)))?;

        let (ready, waiting) = self.settle_files(found);
        let mut report = PollReport::default();
        let mut retry = waiting;
        for file in ready {
            match self.claim(file) {
                Ok(file) => claimed.push(file),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    warn!("Could not claim a file in {}: {e}", self.config.path);
                    retry = true;
                }
            }
        }
        for file in claimed {
            let cleanup = match self.handle(&file, sink).await {
                Ok(IngestOutcome::Busy) => {
                    debug!("{} is busy in the ingest pipeline", file.relative.display());
                    retry = true;
                    continue;
                }
                Ok(IngestOutcome::Skipped { rule }) => {
                    debug!("{} skipped by rule '{rule}'", file.relative.display());
                    report.skipped += 1;
                    self.move_into(SKIPPED, &file).map(|_| ())
                }
                Ok(outcome) => {
                    if matches!(outcome, IngestOutcome::Imported(_)) {
                        report.imported += 1;
                    } else {
                        report.skipped += 1;
                    }
                    self.after_success(&file)
                }
                Err(e) => {
                    warn!("Importing {} failed: {e}", file.relative.display());
                    report.failed += 1;
                    self.quarantine(&file, &e)
                }
            };
            match cleanup {
                Ok(()) => {
                    if let Err(e) = std::fs::remove_file(file.directory.join(ORIGIN))
                        .and_then(|()| prune_staging(&file))
                    {
                        warn!(
                            "Could not remove staging directory {}: {e}",
                            file.directory.display()
                        );
                    }
                }
                Err(e) => {
                    warn!("Could not clean up {}: {e}", file.relative.display());
                    retry = true;
                }
            }
        }
        if retry {
            report.recheck_in = Some(recheck);
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
