use std::sync::Arc;

use chrono::{Duration, Utc};
use tokio::sync::Notify;
use uuid::Uuid;

use super::*;
use crate::events::{DeletedDocument, EventSnapshot};
use crate::models::{NotificationPreferences, VaultRole, VaultSelection};
use crate::processing_events::{
    processing_envelope, ProcessingEventOutcome, ProcessingEventStage, ProcessingOccurrence,
    ProcessingReason,
};
use crate::services::notifications::fakes::{
    device, document, MemoryDevices, MemoryDocuments, Recorder, Roles,
};

struct World {
    devices: Arc<MemoryDevices>,
    documents: Arc<MemoryDocuments>,
    roles: Arc<Roles>,
    transport: Arc<Recorder>,
    planner: NotificationPlanner,
    vault: VaultId,
}

fn world() -> World {
    let devices = Arc::new(MemoryDevices::default());
    let documents = Arc::new(MemoryDocuments::default());
    let roles = Arc::new(Roles::default());
    let transport = Arc::new(Recorder::default());
    let planner = NotificationPlanner::new(
        devices.clone(),
        documents.clone(),
        roles.clone(),
        transport.clone(),
        Arc::new(Notify::new()),
    );
    World {
        devices,
        documents,
        roles,
        transport,
        planner,
        vault: VaultId::new(),
    }
}

fn snapshot(document: &Document) -> EventSnapshot {
    EventSnapshot::from(&DeletedDocument {
        document: document.clone(),
        tags: vec![],
    })
}

fn processed(document: &Document, origin: EventOrigin) -> OutboxEvent {
    OutboxEvent {
        id: 1,
        event: Some(DocumentEvent::Processed {
            document_id: document.id,
        }),
        envelope: None,
        origin,
        context: Some(Arc::new(snapshot(document))),
        created_at: Utc::now(),
        attempts: 0,
    }
}

fn processing(
    document: &Document,
    stage: ProcessingEventStage,
    outcome: ProcessingEventOutcome,
) -> OutboxEvent {
    let envelope = processing_envelope(
        &snapshot(document),
        &ProcessingOccurrence {
            run_id: Uuid::new_v4(),
            revision_id: None,
            stage,
            attempt: 3,
            outcome,
        },
    )
    .unwrap();
    OutboxEvent {
        id: 2,
        event: None,
        envelope: Some(envelope),
        origin: EventOrigin::User,
        context: None,
        created_at: Utc::now(),
        attempts: 0,
    }
}

fn failed(document: &Document) -> OutboxEvent {
    processing(
        document,
        ProcessingEventStage::Ocr,
        ProcessingEventOutcome::Failed {
            reason: ProcessingReason::ProviderFailed,
        },
    )
}

/// A device for `user`, registered an hour ago, with an editor role in the vault
fn editor(w: &World, user: &str) -> PushDevice {
    let device = device(user, Utc::now() - Duration::hours(1));
    w.devices.add(device.clone());
    w.roles.grant(&device.user_id, w.vault, VaultRole::Editor);
    device
}

fn upload(w: &World, by: &str) -> Document {
    let document = document(w.vault, Some(by), Utc::now());
    w.documents.put(document.clone());
    document
}

fn kinds_for(w: &World, device: &PushDevice) -> Vec<(Id, NotificationKind)> {
    w.devices
        .all_pending()
        .into_iter()
        .filter(|p| p.device_id == device.id)
        .map(|p| (p.document_id, p.kind))
        .collect()
}

#[tokio::test]
async fn an_unreviewed_upload_needs_review_for_its_uploader() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();

    assert_eq!(
        kinds_for(&w, &alice),
        vec![(doc.id, NotificationKind::NeedsReview)]
    );
}

#[tokio::test]
async fn a_reviewed_document_is_processed_and_off_by_default() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");
    w.documents
        .update(doc.id, |d| d.reviewed_at = Some(Utc::now()));

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();
    assert!(kinds_for(&w, &alice).is_empty());

    let other = upload(&w, "alice");
    w.documents
        .update(other.id, |d| d.reviewed_at = Some(Utc::now()));
    w.devices.set_preferences(
        alice.id,
        NotificationPreferences {
            processed: true,
            ..Default::default()
        },
    );
    let alice = w.devices.device(alice.id).unwrap();
    w.planner
        .consume(&processed(&other, EventOrigin::User))
        .await
        .unwrap();
    assert_eq!(
        kinds_for(&w, &alice),
        vec![(other.id, NotificationKind::Processed)]
    );
}

#[tokio::test]
async fn only_the_first_completion_notifies() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");
    let event = processed(&doc, EventOrigin::User);

    w.planner.consume(&event).await.unwrap();
    w.devices
        .remove_pending(alice.id, &[(doc.id, NotificationKind::NeedsReview)])
        .await
        .unwrap();
    // A duplicate delivery, a reprocess or a re-embed completes again.
    w.planner.consume(&event).await.unwrap();
    w.planner
        .consume(&processed(&doc, EventOrigin::Workflow))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn documents_older_than_the_device_never_notify() {
    let w = world();
    let alice = editor(&w, "alice");
    let old = document(
        w.vault,
        Some("alice"),
        alice.created_at - Duration::days(30),
    );
    w.documents.put(old.clone());

    // An admin repairs or re-embeds the whole Inbox.
    w.planner
        .consume(&processed(&old, EventOrigin::User))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn imports_never_notify() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::Import))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn terminal_failures_notify_once_and_retries_never() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processing(
            &doc,
            ProcessingEventStage::Ocr,
            ProcessingEventOutcome::RetryScheduled {
                reason: ProcessingReason::ProviderFailed,
                retry_at: Utc::now(),
            },
        ))
        .await
        .unwrap();
    assert!(kinds_for(&w, &alice).is_empty());

    w.planner.consume(&failed(&doc)).await.unwrap();
    w.planner.consume(&failed(&doc)).await.unwrap();
    assert_eq!(
        kinds_for(&w, &alice),
        vec![(doc.id, NotificationKind::Failed)]
    );
}

#[tokio::test]
async fn thumbnail_failures_do_not_notify() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processing(
            &doc,
            ProcessingEventStage::Thumbnail,
            ProcessingEventOutcome::Failed {
                reason: ProcessingReason::ProviderFailed,
            },
        ))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn by_default_only_the_uploader_hears() {
    let w = world();
    let alice = editor(&w, "alice");
    let bob = editor(&w, "bob");
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();

    assert_eq!(kinds_for(&w, &alice).len(), 1);
    assert!(kinds_for(&w, &bob).is_empty());
}

#[tokio::test]
async fn editable_scope_reaches_editors_but_never_viewers() {
    let w = world();
    let editable = NotificationPreferences {
        scope: NotificationScope::Editable,
        ..Default::default()
    };
    let bob = editor(&w, "bob");
    w.devices.set_preferences(bob.id, editable.clone());
    let viewer = editor(&w, "carol");
    w.devices.set_preferences(viewer.id, editable.clone());
    w.roles.grant(&viewer.user_id, w.vault, VaultRole::Viewer);
    let contributor = editor(&w, "dave");
    w.devices.set_preferences(contributor.id, editable);
    w.roles
        .grant(&contributor.user_id, w.vault, VaultRole::Contributor);
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();

    assert_eq!(kinds_for(&w, &bob).len(), 1);
    assert!(kinds_for(&w, &viewer).is_empty());
    // Contributors may only edit their own uploads.
    assert!(kinds_for(&w, &contributor).is_empty());
}

#[tokio::test]
async fn no_role_in_the_vault_means_no_notification() {
    let w = world();
    let alice = editor(&w, "alice");
    w.roles.revoke(&alice.user_id, w.vault);
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn the_vault_filter_applies() {
    let w = world();
    let alice = editor(&w, "alice");
    w.devices.set_preferences(
        alice.id,
        NotificationPreferences {
            vaults: VaultSelection::Only(vec![VaultId::new()]),
            ..Default::default()
        },
    );
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn a_trashed_document_still_records_its_outcome_but_notifies_nobody() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");
    w.documents
        .update(doc.id, |d| d.deleted_at = Some(Utc::now()));
    let event = processed(&doc, EventOrigin::User);

    w.planner.consume(&event).await.unwrap();
    w.documents.update(doc.id, |d| d.deleted_at = None);
    w.planner.consume(&event).await.unwrap();

    assert!(kinds_for(&w, &alice).is_empty());
}

#[tokio::test]
async fn inbox_changes_mark_readers_widgets() {
    let w = world();
    let mut reader = device("erin", Utc::now() - Duration::hours(1));
    reader.widget_token = reader.alert_token.clone();
    w.devices.add(reader.clone());
    w.roles.grant(&reader.user_id, w.vault, VaultRole::Viewer);
    let mut stranger = device("frank", Utc::now() - Duration::hours(1));
    stranger.widget_token = stranger.alert_token.clone();
    w.devices.add(stranger.clone());
    let doc = upload(&w, "alice");

    w.planner
        .consume(&processed(&doc, EventOrigin::Import))
        .await
        .unwrap();

    assert!(w
        .devices
        .state(reader.id)
        .await
        .unwrap()
        .widgets_dirty_at
        .is_some());
    assert!(w
        .devices
        .state(stranger.id)
        .await
        .unwrap()
        .widgets_dirty_at
        .is_none());
}

#[tokio::test]
async fn contributor_access_is_judged_per_uploader() {
    let w = world();
    let editable = NotificationPreferences {
        scope: NotificationScope::Editable,
        ..Default::default()
    };
    let dave = editor(&w, "dave");
    w.devices.set_preferences(dave.id, editable);
    w.roles
        .grant(&dave.user_id, w.vault, VaultRole::Contributor);
    let own = upload(&w, "dave");
    let other = upload(&w, "alice");

    // Dave may edit his own upload, so the cache must not carry that over to Alice's.
    w.planner
        .consume(&processed(&own, EventOrigin::User))
        .await
        .unwrap();
    let mut both = processed(&other, EventOrigin::User);
    both.id = 3;
    w.planner.consume(&both).await.unwrap();

    assert_eq!(
        kinds_for(&w, &dave),
        vec![(own.id, NotificationKind::NeedsReview)]
    );
}

#[tokio::test]
async fn nothing_is_queued_while_push_is_off_but_first_outcomes_are_still_recorded() {
    let w = world();
    let alice = editor(&w, "alice");
    let doc = upload(&w, "alice");
    *w.transport.available.lock().unwrap() = false;

    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();
    assert!(kinds_for(&w, &alice).is_empty(), "no backlog builds up");

    // Turning push on later must not make a reprocess look like a first completion.
    *w.transport.available.lock().unwrap() = true;
    w.planner
        .consume(&processed(&doc, EventOrigin::User))
        .await
        .unwrap();
    assert!(kinds_for(&w, &alice).is_empty());
}
