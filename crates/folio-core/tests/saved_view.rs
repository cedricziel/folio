use folio_core::models::{SavedViewQuery, VaultId};
use serde_json::json;

#[test]
fn rejects_unknown_query_version() {
    let query: SavedViewQuery = serde_json::from_value(json!({"version": 2})).unwrap();
    assert!(query.page(VaultId::new(), 50, None).is_err());
}

#[test]
fn rejects_unknown_filters_instead_of_broadening_results() {
    assert!(serde_json::from_value::<SavedViewQuery>(json!({
        "version": 1, "filters": {"unsupported": "value"}
    }))
    .is_err());
}

#[test]
fn rejects_invalid_date_range() {
    let query: SavedViewQuery = serde_json::from_value(json!({
        "version": 1,
        "filters": {"created_after": "2026-11-02T00:00:00Z", "created_before": "2026-11-01T00:00:00Z"}
    })).unwrap();
    assert!(query.page(VaultId::new(), 50, None).is_err());
}

#[test]
fn restores_filters_sort_and_bounded_pagination() {
    let query: SavedViewQuery = serde_json::from_value(json!({
        "version": 1, "filters": {"search_query": "invoice paid"}, "sort": "title"
    }))
    .unwrap();
    let vault = VaultId::new();
    let page = query.page(vault, 25, None).unwrap();
    assert_eq!(page.filter.search_query.as_deref(), Some("invoice paid"));
    assert_eq!(serde_json::to_value(page.sort).unwrap(), "title");
    assert_eq!(page.limit, 25);
    assert!(query.page(vault, 101, None).is_err());
}

#[test]
fn rejects_client_supplied_owner_and_actor() {
    assert!(serde_json::from_value::<folio_core::models::SavedViewInput>(json!({
        "name": "Invoices", "query": {"version": 1}, "owner_id": "someone-else", "created_by": "someone-else"
    })).is_err());
}

fn private_view() -> folio_core::models::SavedView {
    serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-000000000001",
        "vault_id": "00000000-0000-0000-0000-000000000002",
        "owner_id": "alice", "name": "Private invoices", "visibility": "private",
        "query": {"version": 1}, "presentation": "list",
        "created_at": "2026-11-02T00:00:00Z", "updated_at": "2026-11-02T00:00:00Z",
        "deleted_at": null, "created_by": "alice", "updated_by": "alice"
    }))
    .unwrap()
}

#[test]
fn vault_owner_cannot_read_or_edit_another_members_private_view() {
    use folio_core::models::{UserId, VaultRole};
    let view = private_view();
    let other = UserId::from("bob");
    assert!(!view.visible_to(&other));
    assert!(!view.editable_by(&other, VaultRole::Owner));
}

#[test]
fn viewer_can_manage_own_private_view() {
    use folio_core::models::{UserId, VaultRole};
    assert!(private_view().editable_by(&UserId::from("alice"), VaultRole::Viewer));
}

#[test]
fn shared_view_creator_loses_edit_access_after_role_downgrade() {
    use folio_core::models::{UserId, VaultRole, ViewVisibility};
    let mut view = private_view();
    view.visibility = ViewVisibility::Shared;
    let creator = UserId::from("alice");
    assert!(view.visible_to(&creator));
    assert!(!view.editable_by(&creator, VaultRole::Viewer));
    assert!(view.editable_by(&creator, VaultRole::Editor));
}

#[test]
fn soft_deleted_view_is_not_visible_even_to_owner() {
    use folio_core::models::{UserId, VaultRole};
    let mut view = private_view();
    view.deleted_at = Some(view.updated_at);
    let owner = UserId::from("alice");
    assert!(!view.visible_to(&owner));
    assert!(!view.editable_by(&owner, VaultRole::Owner));
}
