use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};

use super::*;
use crate::models::{
    Document, DocumentOutcome, NotificationPreferences, PendingNotification, PushToken, VaultId,
    VaultRole,
};
use crate::ports::PushLimits;
use crate::services::notifications::fakes::{
    device, document, Grants, MemoryDevices, MemoryDocuments, Recorder, Roles,
};

struct World {
    devices: Arc<MemoryDevices>,
    documents: Arc<MemoryDocuments>,
    roles: Arc<Roles>,
    grants: Arc<Grants>,
    transport: Arc<Recorder>,
    sender: NotificationSender,
    vault: VaultId,
    alice: PushDevice,
    start: DateTime<Utc>,
}

fn world() -> World {
    let devices = Arc::new(MemoryDevices::default());
    let documents = Arc::new(MemoryDocuments::default());
    let roles = Arc::new(Roles::default());
    let grants = Arc::new(Grants::default());
    let transport = Arc::new(Recorder::default());
    let sender = NotificationSender::new(
        devices.clone(),
        documents.clone(),
        roles.clone(),
        grants.clone(),
        transport.clone(),
        Arc::new(PushLimits::default()),
        SenderConfig::default(),
    );
    let start = Utc::now();
    let vault = VaultId::new();
    let alice = device("alice", start - Duration::days(1));
    devices.add(alice.clone());
    roles.grant(&alice.user_id, vault, VaultRole::Editor);
    World {
        devices,
        documents,
        roles,
        grants,
        transport,
        sender,
        vault,
        alice,
        start,
    }
}

/// Queue a notification for Alice about a new upload of hers in `vault`, at `at`
async fn queue(w: &World, vault: VaultId, kind: NotificationKind, at: DateTime<Utc>) -> Document {
    let doc = document(vault, Some("alice"), at);
    w.documents.put(doc.clone());
    let outcome = match kind {
        NotificationKind::Failed => DocumentOutcome::Failed,
        _ => DocumentOutcome::Completed,
    };
    w.devices
        .record_first_outcome(
            doc.id,
            outcome,
            &[PendingNotification {
                device_id: w.alice.id,
                document_id: doc.id,
                kind,
                vault_id: vault,
                created_at: at,
            }],
        )
        .await
        .unwrap();
    doc
}

fn bodies(w: &World) -> Vec<String> {
    w.transport
        .sent()
        .iter()
        .map(|m| {
            m.payload["aps"]["alert"]["body"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn waits_for_the_quiet_window() {
    let w = world();
    let doc = queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;

    assert_eq!(
        w.sender
            .flush(w.start + Duration::seconds(10))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        w.sender
            .flush(w.start + Duration::seconds(31))
            .await
            .unwrap(),
        1
    );

    let sent = w.transport.sent();
    assert_eq!(
        sent[0].payload["folio"]["document"],
        serde_json::json!(doc.id)
    );
    assert_eq!(sent[0].priority, 10);
    assert_eq!(sent[0].push_type, PushType::Alert);
    assert!(!sent[0].payload.to_string().contains(&doc.title));
    assert!(w.devices.pending(w.alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_bulk_upload_is_a_handful_of_alerts() {
    let w = world();
    let mut sent = 0;
    // 200 documents finish over 3 minutes, a little apart, with the sender ticking every 5 s.
    for second in 0..180 {
        let now = w.start + Duration::seconds(second);
        if second % 9 < 10 && second < 179 {
            queue(&w, w.vault, NotificationKind::NeedsReview, now).await;
            if second % 2 == 0 {
                queue(&w, w.vault, NotificationKind::NeedsReview, now).await;
            }
        }
        if second % 5 == 0 {
            sent += w.sender.flush(now).await.unwrap();
        }
    }
    sent += w
        .sender
        .flush(w.start + Duration::minutes(4))
        .await
        .unwrap();

    assert!(sent <= 2, "sent {sent} alerts");
    let counted: usize = w
        .transport
        .sent()
        .iter()
        .map(|m| {
            m.payload["folio"]["counts"]["needs_review"]
                .as_u64()
                .unwrap() as usize
        })
        .sum();
    assert_eq!(counted, 179 + 90);
}

#[tokio::test]
async fn one_alert_covers_several_vaults() {
    let w = world();
    let other = VaultId::new();
    w.roles.grant(&w.alice.user_id, other, VaultRole::Owner);
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    queue(&w, other, NotificationKind::NeedsReview, w.start).await;
    queue(&w, other, NotificationKind::Failed, w.start).await;

    w.sender
        .flush(w.start + Duration::minutes(1))
        .await
        .unwrap();

    let sent = w.transport.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].payload["aps"].get("thread-id").is_none());
    assert_eq!(bodies(&w), vec!["2 documents need review, 1 failed"]);
    assert!(sent[0].collapse_id.as_ref().unwrap().len() <= 64);
}

#[tokio::test]
async fn the_hourly_cap_holds_and_carries_over() {
    let w = world();
    let mut now = w.start;
    for _ in 0..12 {
        queue(&w, w.vault, NotificationKind::NeedsReview, now).await;
        now += Duration::minutes(1);
        w.sender.flush(now).await.unwrap();
    }
    assert_eq!(w.transport.sent().len(), 12);

    queue(&w, w.vault, NotificationKind::NeedsReview, now).await;
    queue(&w, w.vault, NotificationKind::Failed, now).await;
    w.sender.flush(now + Duration::minutes(1)).await.unwrap();
    assert_eq!(w.transport.sent().len(), 12, "the cap is reached");

    w.sender
        .flush(w.start + Duration::minutes(61))
        .await
        .unwrap();
    assert_eq!(w.transport.sent().len(), 13);
    assert_eq!(
        bodies(&w).last().unwrap(),
        "1 document needs review, 1 failed"
    );
}

#[tokio::test]
async fn reviewed_trashed_and_inaccessible_documents_are_dropped() {
    let w = world();
    let reviewed = queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    w.documents
        .update(reviewed.id, |d| d.reviewed_at = Some(Utc::now()));
    let trashed = queue(&w, w.vault, NotificationKind::Failed, w.start).await;
    w.documents
        .update(trashed.id, |d| d.deleted_at = Some(Utc::now()));
    let elsewhere = VaultId::new();
    queue(&w, elsewhere, NotificationKind::NeedsReview, w.start).await;

    assert_eq!(
        w.sender
            .flush(w.start + Duration::minutes(1))
            .await
            .unwrap(),
        0
    );
    assert!(w.transport.sent().is_empty());
    assert!(w.devices.pending(w.alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_document_is_kept_when_reviewed() {
    let w = world();
    let doc = queue(&w, w.vault, NotificationKind::Failed, w.start).await;
    w.documents
        .update(doc.id, |d| d.reviewed_at = Some(Utc::now()));

    assert_eq!(
        w.sender
            .flush(w.start + Duration::minutes(1))
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn an_ended_grant_sends_nothing_and_deletes_the_device() {
    let w = world();
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    w.grants.end(&w.alice.grant_id);

    w.sender
        .flush(w.start + Duration::minutes(1))
        .await
        .unwrap();

    assert!(w.transport.sent().is_empty());
    assert!(w.devices.device(w.alice.id).is_none());
}

#[tokio::test]
async fn two_senders_send_once() {
    let w = world();
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    let second = NotificationSender::new(
        w.devices.clone(),
        w.documents.clone(),
        w.roles.clone(),
        w.grants.clone(),
        w.transport.clone(),
        Arc::new(PushLimits::default()),
        SenderConfig::default(),
    );
    let now = w.start + Duration::minutes(1);
    // Both claim before either sends.
    let first = w
        .devices
        .claim_due(
            now,
            Duration::seconds(30),
            Duration::minutes(5),
            Duration::seconds(60),
            10,
        )
        .await
        .unwrap();
    let again = second.flush(now).await.unwrap();

    assert_eq!(first, vec![w.alice.id]);
    assert_eq!(again, 0);
}

#[tokio::test]
async fn nothing_is_claimed_while_push_is_unavailable() {
    let w = world();
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    *w.transport.available.lock().unwrap() = false;

    assert_eq!(
        w.sender
            .flush(w.start + Duration::minutes(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(w.devices.pending(w.alice.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn responses_are_handled_per_apns_guidance() {
    let cases: Vec<(PushOutcome, bool, bool, bool)> = vec![
        // (outcome, token kept, pending kept, backing off)
        (PushOutcome::Delivered, true, false, false),
        (
            PushOutcome::TokenInvalid { since: None },
            false,
            false,
            false,
        ),
        (
            PushOutcome::Rejected {
                reason: "PayloadTooLarge".into(),
            },
            true,
            false,
            false,
        ),
        (
            PushOutcome::Retry {
                reason: "ServiceUnavailable".into(),
            },
            true,
            true,
            true,
        ),
        (
            PushOutcome::CredentialError {
                reason: "InvalidProviderToken".into(),
            },
            true,
            true,
            false,
        ),
    ];
    for (outcome, token_kept, pending_kept, backing_off) in cases {
        let w = world();
        let mut alice = w.alice.clone();
        alice.widget_token = alice.alert_token.clone();
        w.devices.add(alice);
        queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
        w.transport.answer(outcome.clone());

        let now = w.start + Duration::minutes(1);
        w.sender.flush(now).await.unwrap();

        let device = w.devices.device(w.alice.id).unwrap();
        assert_eq!(device.alert_token.is_some(), token_kept, "{outcome:?}");
        assert_eq!(
            !w.devices.pending(w.alice.id).await.unwrap().is_empty(),
            pending_kept,
            "{outcome:?}"
        );
        let state = w.devices.state(w.alice.id).await.unwrap();
        assert_eq!(
            state.backoff_until.is_some_and(|until| until > now),
            backing_off,
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_dead_alert_token_is_cleared_even_after_a_later_widget_refresh() {
    let w = world();
    let mut alice = w.alice.clone();
    alice.widget_token = Some(PushToken {
        value: "cd".repeat(32),
        registered_at: w.start,
    });
    w.devices.add(alice);
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    // APNs saw the alert token die after it was registered, before the widget refresh.
    w.transport.answer(PushOutcome::TokenInvalid {
        since: Some(w.alice.created_at + Duration::hours(1)),
    });

    w.sender
        .flush(w.start + Duration::minutes(1))
        .await
        .unwrap();

    let device = w.devices.device(w.alice.id).unwrap();
    assert!(device.alert_token.is_none());
    assert!(device.widget_token.is_some());
}

#[tokio::test]
async fn a_token_registered_after_apns_invalidated_it_is_kept() {
    let w = world();
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    w.transport.answer(PushOutcome::TokenInvalid {
        since: Some(w.alice.created_at - Duration::hours(1)),
    });

    w.sender
        .flush(w.start + Duration::minutes(1))
        .await
        .unwrap();

    assert!(w.devices.device(w.alice.id).unwrap().alert_token.is_some());
}

#[tokio::test]
async fn widget_pushes_are_spaced_by_fifteen_minutes() {
    let w = world();
    let mut alice = w.alice.clone();
    alice.widget_token = Some(PushToken {
        value: "cd".repeat(32),
        registered_at: w.start,
    });
    w.devices.add(alice.clone());

    for minute in 0..3 {
        w.devices
            .mark_widgets_dirty(&[alice.id], w.start + Duration::minutes(minute))
            .await
            .unwrap();
        w.sender
            .flush(w.start + Duration::minutes(minute))
            .await
            .unwrap();
    }
    let widgets: Vec<_> = w
        .transport
        .sent()
        .into_iter()
        .filter(|m| m.push_type == PushType::Widgets)
        .collect();
    assert_eq!(widgets.len(), 1);
    assert_eq!(
        widgets[0].payload,
        serde_json::json!({"aps": {"content-changed": true}})
    );

    w.sender
        .flush(w.start + Duration::minutes(16))
        .await
        .unwrap();
    let widgets = w
        .transport
        .sent()
        .into_iter()
        .filter(|m| m.push_type == PushType::Widgets)
        .count();
    assert_eq!(widgets, 2);
}

#[tokio::test]
async fn preferences_changed_while_pending_apply() {
    let w = world();
    queue(&w, w.vault, NotificationKind::NeedsReview, w.start).await;
    w.devices.set_preferences(
        w.alice.id,
        NotificationPreferences {
            needs_review: false,
            ..Default::default()
        },
    );

    assert_eq!(
        w.sender
            .flush(w.start + Duration::minutes(1))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn test_notifications_are_limited_to_one_a_minute() {
    let w = world();
    let user = w.alice.user_id.clone();
    let id = w.alice.installation_id.clone();

    assert_eq!(w.sender.send_test(&user, &id, w.start).await, Ok(()));
    assert_eq!(
        w.sender
            .send_test(&user, &id, w.start + Duration::seconds(30))
            .await,
        Err(TestRefusal::TooSoon)
    );
    assert_eq!(
        w.sender.send_test(&user, "unknown", w.start).await,
        Err(TestRefusal::NotFound)
    );
    *w.transport.available.lock().unwrap() = false;
    assert_eq!(
        w.sender
            .send_test(&user, &id, w.start + Duration::minutes(2))
            .await,
        Err(TestRefusal::Unavailable)
    );
}
