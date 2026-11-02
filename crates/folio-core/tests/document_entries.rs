use folio_core::models::{EntryBody, EntryKind, EntryPermissions, UserId, VaultRole};

#[test]
fn entry_text_rejects_blank_and_limits_utf8_bytes() {
    assert!(EntryBody::parse(" \n\t".into()).is_err());
    assert!(EntryBody::parse("é".repeat(8193)).is_err());
    assert!(EntryBody::parse("é".repeat(8192)).is_ok());
    assert_eq!(
        EntryBody::parse(" A\nB ".into()).unwrap().as_str(),
        " A\nB "
    );
}

#[test]
fn only_writable_authors_can_edit_and_editors_can_moderate() {
    let author = UserId::from("author");
    let other = UserId::from("other");
    let own = EntryPermissions::for_member(VaultRole::Contributor, &author, Some(&author));
    assert!(own.can_edit && own.can_delete);
    let viewer = EntryPermissions::for_member(VaultRole::Viewer, &author, Some(&author));
    assert!(!viewer.can_edit && !viewer.can_delete);
    let moderator = EntryPermissions::for_member(VaultRole::Editor, &other, Some(&author));
    assert!(!moderator.can_edit && moderator.can_delete);
    let unknown = EntryPermissions::for_member(VaultRole::Contributor, &author, None);
    assert!(!unknown.can_edit && !unknown.can_delete);
}

#[test]
fn entry_kind_has_stable_wire_names() {
    assert_eq!(serde_json::to_string(&EntryKind::Note).unwrap(), "\"note\"");
    assert_eq!(
        serde_json::to_string(&EntryKind::Comment).unwrap(),
        "\"comment\""
    );
    assert!(serde_json::from_str::<EntryKind>("\"reply\"").is_err());
}

#[test]
fn continuation_is_bound_to_document_vault_and_kind() {
    use chrono::{TimeZone, Utc};
    use folio_core::models::{DocumentEntry, EntryPageQuery, Id, VaultId};
    let vault = VaultId::new();
    let document = Id::new_v4();
    let query = EntryPageQuery {
        kind: EntryKind::Comment,
        limit: 1,
        cursor: None,
    };
    let first = DocumentEntry {
        id: Id::new_v4(),
        vault_id: vault,
        document_id: document,
        kind: EntryKind::Comment,
        body: "First".into(),
        created_at: Utc.timestamp_opt(100, 0).unwrap(),
        updated_at: Utc.timestamp_opt(100, 0).unwrap(),
        deleted_at: None,
        created_by: None,
        updated_by: None,
    };
    let second = DocumentEntry {
        id: Id::new_v4(),
        body: "Second".into(),
        ..first.clone()
    };
    let page = query.finish(vault, document, vec![first.clone(), second]);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].id, first.id);
    let continuation = EntryPageQuery {
        cursor: page.next_cursor,
        ..query
    };
    assert!(continuation.cursor.is_some());
    assert!(continuation.validate(vault, document).is_ok());
    assert!(continuation.validate(VaultId::new(), document).is_err());
    assert!(continuation.validate(vault, Id::new_v4()).is_err());
    assert!(EntryPageQuery {
        kind: EntryKind::Note,
        ..continuation
    }
    .validate(vault, document)
    .is_err());
}

#[test]
fn entry_page_rejects_unbounded_reads() {
    use folio_core::models::{EntryPageQuery, Id, VaultId};
    let query = EntryPageQuery {
        kind: EntryKind::Note,
        limit: 0,
        cursor: None,
    };
    assert!(query.validate(VaultId::new(), Id::new_v4()).is_err());
    assert!(EntryPageQuery {
        limit: 101,
        ..query
    }
    .validate(VaultId::new(), Id::new_v4())
    .is_err());
}
