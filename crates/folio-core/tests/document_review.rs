use chrono::{DateTime, Utc};
use folio_core::models::{DocumentReview, ReviewOrigin, UserId};

fn at(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}

#[test]
fn repeated_review_preserves_original_actor_and_time() {
    let mut review = DocumentReview::default();
    let first = at("2026-11-02T10:00:00Z");
    assert!(review.set_reviewed(true, Some(UserId("alice".into())), first));
    assert!(!review.set_reviewed(true, Some(UserId("bob".into())), at("2026-11-02T11:00:00Z")));
    let decision = review.decision().unwrap();
    assert_eq!(decision.reviewed_at, first);
    assert_eq!(decision.reviewed_by, Some(UserId("alice".into())));
    assert_eq!(decision.origin, ReviewOrigin::Human);
}

#[test]
fn unreview_clears_attribution_and_repeated_unreview_is_unchanged() {
    let mut review = DocumentReview::default();
    review.set_reviewed(
        true,
        Some(UserId("alice".into())),
        at("2026-11-02T10:00:00Z"),
    );
    assert!(review.set_reviewed(
        false,
        Some(UserId("bob".into())),
        at("2026-11-02T11:00:00Z")
    ));
    assert!(review.decision().is_none());
    assert!(!review.set_reviewed(false, None, at("2026-11-02T12:00:00Z")));
}

#[test]
fn system_and_legacy_review_never_invent_an_actor() {
    let time = at("2026-11-02T10:00:00Z");
    let legacy = DocumentReview::legacy_classification(time);
    assert_eq!(
        legacy.decision().unwrap().origin,
        ReviewOrigin::LegacyClassification
    );
    assert_eq!(legacy.decision().unwrap().reviewed_by, None);
    let mut system = DocumentReview::default();
    system.set_reviewed(true, None, time);
    assert_eq!(system.decision().unwrap().origin, ReviewOrigin::System);
    assert_eq!(system.decision().unwrap().reviewed_by, None);
}
