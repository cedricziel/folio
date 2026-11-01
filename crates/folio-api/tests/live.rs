//! Live updates: document events become Turbo Streams.

mod common;

use common::harness;

#[tokio::test]
async fn event_stream_turns_a_new_document_into_turbo_streams() {
    let h = harness().await;
    let doc = h
        .state
        .document_repo
        .create(folio_core::models::CreateDocument {
            title: "Fresh".into(),
            content: None,
            original_filename: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
        })
        .await
        .unwrap();

    let html = folio_api::routes::turbo_streams_for(
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
        &h.state,
        &folio_api::DocumentEvent::Deleted {
            document_id: doc.id,
            snapshot: None,
        },
    )
    .await
    .unwrap();
    assert!(html.contains(&format!(r#"action="remove" target="document-{}""#, doc.id)));
}
