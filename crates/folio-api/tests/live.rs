//! Live updates: document events become Turbo Streams.

mod common;

use common::harness;

#[tokio::test]
async fn event_stream_turns_a_new_document_into_turbo_streams() {
    let h = harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            folio_core::models::CreateDocument {
                title: "Fresh".into(),
                content: None,
                original_filename: None,
                mime_type: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let html = folio_api::routes::turbo_streams_for(
        folio_api::vault::DefaultVault(h.vault),
        &h.state,
        &folio_api::DocumentEvent::Created {
            document_id: doc.id,
        },
    )
    .await
    .unwrap();
    assert!(html.contains(r#"action="prepend" target="documents-rows""#));
    assert!(html.contains("Fresh"));

    let html = folio_api::routes::turbo_streams_for(
        folio_api::vault::DefaultVault(h.vault),
        &h.state,
        &folio_api::DocumentEvent::Deleted {
            document_id: doc.id,
            snapshot: Some(std::sync::Arc::new(folio_core::events::DeletedDocument {
                document: doc.clone(),
                tags: Vec::new(),
            })),
        },
    )
    .await
    .unwrap();
    assert!(html.contains(&format!(r#"action="remove" target="document-{}""#, doc.id)));
}

#[tokio::test]
async fn processing_updates_refresh_the_open_detail_page() {
    use folio_core::models::{ProcessingStage, ProcessingStatus};
    let h = harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            folio_core::models::CreateDocument {
                title: "Scan".into(),
                mime_type: Some("application/pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .set_page_count(h.vault, doc.id, 3)
        .await
        .unwrap();
    h.state
        .document_repo
        .update(
            h.vault,
            doc.id,
            folio_core::models::UpdateDocument {
                content: Some("Invoice total 42".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .set_stage_status(
            h.vault,
            doc.id,
            ProcessingStage::Ocr,
            ProcessingStatus::Done,
        )
        .await
        .unwrap();

    let html = folio_api::routes::turbo_streams_for(
        folio_api::vault::DefaultVault(h.vault),
        &h.state,
        &folio_api::DocumentEvent::Processed {
            document_id: doc.id,
        },
    )
    .await
    .unwrap();
    for section in ["facts", "pages", "processing", "text"] {
        assert!(
            html.contains(&format!(
                r#"action="replace" target="document-{}-{section}""#,
                doc.id
            )),
            "{section} is refreshed"
        );
    }
    assert!(html.contains("3 pages"));
    assert!(html.contains(r#"data-page="3""#));
    assert!(html.contains("Invoice total 42"));
    // Edit forms stay as the person left them
    assert!(!html.contains("correspondent_id"));
}

mod resume {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use folio_core::models::CreateDocument;
    use folio_core::services::{BroadcastConsumer, DispatcherConfig, OutboxDispatcher};

    use super::common::{harness, Harness};

    async fn create(h: &Harness, title: &str) {
        h.state
            .document_repo
            .create(
                h.vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    fn dispatcher(h: &Harness) -> OutboxDispatcher {
        OutboxDispatcher::new(
            h.state.outbox.clone(),
            vec![Arc::new(BroadcastConsumer(
                h.state.event_broadcaster.clone(),
            ))],
            DispatcherConfig::default(),
        )
    }

    /// Read the stream until `wanted` event ids were seen
    ///
    /// Each event is two frames (JSON, then Turbo Streams) sharing one id, so
    /// the second frame of an event already reported earlier may still be in
    /// flight; ids up to `after` are ignored.
    async fn ids_until(body: &mut Body, after: i64, wanted: usize) -> Vec<i64> {
        let mut text = String::new();
        let mut ids = Vec::new();
        while ids.len() < wanted {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
                .await
                .expect("the stream delivers the events in time")
                .expect("the stream stays open")
                .unwrap();
            if let Some(data) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(data));
            }
            ids = text
                .lines()
                .filter_map(|line| line.strip_prefix("id: "))
                .filter_map(|id| id.trim().parse().ok())
                .filter(|id| *id > after)
                .fold(Vec::new(), |mut ids, id| {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                    ids
                });
        }
        ids
    }

    #[tokio::test]
    async fn a_reconnecting_client_gets_the_events_it_missed_then_live_ones() {
        let h = harness().await;
        for title in ["one", "two", "three"] {
            create(&h, title).await;
        }
        let dispatch = dispatcher(&h);
        assert_eq!(dispatch.dispatch_once().await.unwrap(), 3);
        let known = h
            .state
            .outbox
            .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 10)
            .await
            .unwrap();
        let ids: Vec<i64> = known.iter().map(|e| e.id).collect();
        assert_eq!(ids.len(), 3);

        // The client saw the first event before it lost the connection.
        let response = h
            .app
            .clone()
            .oneshot(
                Request::get("/events")
                    .header("last-event-id", ids[0].to_string())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = response.into_body();
        assert_eq!(ids_until(&mut body, ids[0], 2).await, [ids[1], ids[2]]);

        // Live events follow, with ids from the same sequence.
        create(&h, "four").await;
        dispatch.dispatch_once().await.unwrap();
        let live = ids_until(&mut body, ids[2], 1).await;
        assert!(live[0] > ids[2]);
    }

    #[tokio::test]
    async fn a_client_without_last_event_id_starts_with_live_events() {
        let h = harness().await;
        create(&h, "before").await;
        let dispatch = dispatcher(&h);
        dispatch.dispatch_once().await.unwrap();

        let response = h
            .app
            .clone()
            .oneshot(Request::get("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mut body = response.into_body();
        create(&h, "after").await;
        dispatch.dispatch_once().await.unwrap();

        let ids = ids_until(&mut body, 0, 1).await;
        let newest = h
            .state
            .outbox
            .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 10)
            .await
            .unwrap()
            .last()
            .unwrap()
            .id;
        assert_eq!(ids, [newest]);
    }

    #[tokio::test]
    async fn a_failed_stage_refreshes_the_open_detail_page() {
        use folio_core::events::{EventOrigin, OutboxEvent};
        use folio_core::models::{ProcessingStage, ProcessingStatus};
        use folio_core::{DomainEventEnvelope, DomainEventSubject, EventAudience};

        let h = harness().await;
        let doc = h
            .state
            .document_repo
            .create(
                h.vault,
                CreateDocument {
                    title: "Broken".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let response = h
            .app
            .clone()
            .oneshot(Request::get("/events").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mut body = response.into_body();

        h.state
            .document_repo
            .set_stage_status(
                h.vault,
                doc.id,
                ProcessingStage::Ocr,
                ProcessingStatus::Failed,
            )
            .await
            .unwrap();
        // Stage failures are recorded as domain events without a legacy document event
        let envelope = DomainEventEnvelope::new(
            "document.processing.ocr.failed",
            h.vault,
            DomainEventSubject {
                subject_type: "document".into(),
                id: doc.id,
            },
            Some(doc.id),
            EventAudience::Vault,
            serde_json::json!({}),
        )
        .unwrap();
        h.state.event_broadcaster.send(OutboxEvent {
            id: 1_000_000,
            event: None,
            envelope: Some(envelope),
            origin: EventOrigin::User,
            context: None,
            created_at: chrono::Utc::now(),
            attempts: 0,
        });

        let mut text = String::new();
        while !text.contains("Text extraction did not finish") {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
                .await
                .expect("the stream refreshes the detail page in time")
                .expect("the stream stays open")
                .unwrap();
            if let Some(data) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(data));
            }
        }
        assert!(text.contains(&format!(r#"target="document-{}-processing""#, doc.id)));
        assert!(text.contains("id: 1000000"));
        assert!(!text.lines().any(|line| line.starts_with("event:")));
    }
}
