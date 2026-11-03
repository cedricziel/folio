use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::{SavedView, ViewVisibility};
use folio_core::Result;

pub(super) fn event(
    event_type: &str,
    view: &SavedView,
    previous: Option<ViewVisibility>,
) -> Result<DomainEventEnvelope> {
    let audience = if view.visibility == ViewVisibility::Private
        || previous == Some(ViewVisibility::Private)
    {
        EventAudience::User {
            user_id: view.owner_id.clone(),
        }
    } else {
        EventAudience::Vault
    };
    DomainEventEnvelope::new(
        event_type,
        view.vault_id,
        DomainEventSubject {
            subject_type: "saved_view".into(),
            id: view.id,
        },
        None,
        audience,
        serde_json::json!({"name": view.name, "visibility": view.visibility, "presentation": view.presentation}),
    )
}
