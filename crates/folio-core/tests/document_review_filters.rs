use folio_core::models::{
    Document, DocumentFilter, DocumentPageQuery, Id, ProcessingStatus, VaultId,
};

#[test]
fn review_and_date_filters_bind_continuations_and_allow_one_day() {
    let vault = VaultId::new();
    let date = "2000-02-29".parse().unwrap();
    let query = DocumentPageQuery {
        limit: 1,
        filter: DocumentFilter {
            reviewed: Some(false),
            document_date_from: Some(date),
            document_date_to: Some(date),
            ..Default::default()
        },
        ..Default::default()
    };
    query.validate(vault).unwrap();
    let page = query.finish(
        vault,
        vec![Document::new(vault, "A"), Document::new(vault, "B")],
    );
    let mut next = query.clone();
    next.cursor = page.next_cursor;
    next.validate(vault).unwrap();
    next.filter.reviewed = Some(true);
    assert!(next.validate(vault).is_err());
    next.filter.reviewed = Some(false);
    next.filter.document_date_to = Some("2000-03-01".parse().unwrap());
    assert!(next.validate(vault).is_err());
}

#[test]
fn reversed_document_date_range_is_invalid() {
    let filter = DocumentFilter {
        document_date_from: Some("2026-11-03".parse().unwrap()),
        document_date_to: Some("2026-11-02".parse().unwrap()),
        ..Default::default()
    };
    assert!(filter.validate().is_err());
}

#[test]
fn classification_does_not_review_and_review_does_not_hide_processing() {
    use folio_core::models::DocumentStatus;
    let mut doc = Document::new(VaultId::new(), "Classified").with_correspondent(Id::new_v4());
    doc.ocr_status = ProcessingStatus::Done;
    doc.embed_status = ProcessingStatus::Done;
    assert_eq!(doc.status(), DocumentStatus::NeedsReview);
    doc.reviewed_at = Some(chrono::Utc::now());
    assert_eq!(doc.status(), DocumentStatus::Filed);
    doc.embed_status = ProcessingStatus::Pending;
    assert_eq!(doc.status(), DocumentStatus::Processing);
    doc.ocr_status = ProcessingStatus::Failed;
    assert_eq!(doc.status(), DocumentStatus::Failed);
}
