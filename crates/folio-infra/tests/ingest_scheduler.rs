use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use folio_core::models::{
    FolderAfterImport, FolderConfig, Id, IngestItem, IngestOutcome, IngestionSourceDef,
    IngestionSourceInput, PollReport, SourceConfig, VaultId,
};
use folio_core::ports::{IngestSink, IngestionSource, IngestionSourceFactory, VaultIngest};
use folio_core::services::IngestScheduler;
use folio_core::{Error, Result};
use folio_infra::Database;
use tokio::sync::{mpsc, watch, Mutex, Notify};

struct ControlledSource {
    reports: Arc<Mutex<mpsc::Receiver<Result<PollReport>>>>,
    wake: Arc<Notify>,
}

#[async_trait]
impl IngestionSource for ControlledSource {
    async fn poll(&self, _: &dyn IngestSink) -> Result<PollReport> {
        self.reports
            .lock()
            .await
            .recv()
            .await
            .expect("test source remains open")
    }

    async fn wake(&self) {
        self.wake.notified().await;
    }

    async fn test(&self) -> Result<String> {
        Ok("Available".into())
    }

    fn interval_secs(&self) -> u64 {
        3600
    }
}

impl IngestionSourceFactory for ControlledSource {
    fn build(&self, _: &IngestionSourceDef) -> Result<Box<dyn IngestionSource>> {
        Ok(Box::new(Self {
            reports: self.reports.clone(),
            wake: self.wake.clone(),
        }))
    }
}

struct UnusedSink;

#[async_trait]
impl VaultIngest for UnusedSink {
    async fn ingest(&self, _: VaultId, _: Id, _: IngestItem) -> Result<IngestOutcome> {
        panic!("health polling must not import a document")
    }
}

#[tokio::test]
async fn scheduler_commits_failure_and_recovery_in_the_managed_sources_vault() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.create("Source vault", None).await.unwrap().id;
    let source = repos
        .sources
        .create(
            vault,
            IngestionSourceInput {
                name: "Controlled source".into(),
                enabled: true,
                config: SourceConfig::Folder(FolderConfig {
                    path: "/unused-test-source".into(),
                    recursive: false,
                    subfolders_as_tags: false,
                    after_import: FolderAfterImport::Delete,
                    settle_secs: 0,
                    rescan_secs: 3600,
                }),
            },
        )
        .await
        .unwrap();
    let (send, receive) = mpsc::channel(4);
    let wake = Arc::new(Notify::new());
    let factory = Arc::new(ControlledSource {
        reports: Arc::new(Mutex::new(receive)),
        wake: wake.clone(),
    });
    let (shutdown, stopped) = watch::channel(false);
    let scheduler = IngestScheduler::new(
        repos.sources.clone(),
        vec![],
        factory,
        Arc::new(UnusedSink),
        repos.vaults.clone(),
    )
    .with_outcomes(repos.ingest_outcomes.clone())
    .with_reload_interval(Duration::from_millis(10));
    let task = tokio::spawn(scheduler.run(stopped));
    let mut health_events = vec![];
    let mut previous_run = None;
    for failure in [Some("private:first"), Some("private:second"), None] {
        let report = match failure {
            Some(message) => Err(Error::Repository(message.into())),
            None => Ok(PollReport {
                failed: 1,
                ..Default::default()
            }),
        };
        send.send(report).await.unwrap();
        wake.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = repos.sources.status(source.id).await.unwrap() {
                    let fresh = status.last_run_at.is_some() && status.last_run_at != previous_run;
                    let matches = match failure {
                        Some(message) => status.last_error.as_ref().is_some_and(|error| {
                            assert!(!error.contains(message));
                            error == "Folio could not process this source run"
                        }),
                        None => status.last_error.is_none() && status.last_failed == 1,
                    };
                    if fresh && matches {
                        previous_run = status.last_run_at;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("scheduler must persist each controlled poll");
        for event in repos
            .outbox
            .claim(100, chrono::Duration::seconds(30), 10)
            .await
            .unwrap()
        {
            if let Some(envelope) = &event.envelope {
                if matches!(envelope.event_type(), "source.failed" | "source.recovered") {
                    health_events.push(envelope.clone());
                }
            }
            repos.outbox.mark_dispatched(event.id).await.unwrap();
        }
    }
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(health_events.len(), 2);
    assert_eq!(health_events[0].event_type(), "source.failed");
    assert_eq!(health_events[1].event_type(), "source.recovered");
    assert!(health_events
        .iter()
        .all(|event| event.vault_id() == vault && event.subject().id == source.id));
    assert!(health_events
        .iter()
        .all(|event| !event.data().to_string().contains("private")));
}
