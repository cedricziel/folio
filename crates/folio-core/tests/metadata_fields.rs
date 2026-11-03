use folio_core::models::{MetadataFieldInput, MetadataOptionInput};

#[test]
fn field_input_normalizes_name_and_defaults_constraints_to_optional() {
    let input: MetadataFieldInput =
        serde_json::from_str(r#"{"name":"  Invoice amount  ","kind":"money"}"#).unwrap();
    let input = input.validate().unwrap();
    assert_eq!(input.name, "Invoice amount");
    assert!(!input.required);
    assert!(!input.unique);
}

#[test]
fn field_input_rejects_blank_names() {
    let input: MetadataFieldInput = serde_json::from_str(r#"{"name":"  ","kind":"text"}"#).unwrap();
    assert!(input.validate().is_err());
}

#[test]
fn field_input_rejects_ownership_overrides() {
    assert!(serde_json::from_str::<MetadataFieldInput>(
        r#"{"name":"Amount","kind":"money","vault_id":"00000000-0000-0000-0000-000000000000"}"#
    )
    .is_err());
}

#[test]
fn field_input_rejects_audit_actor_overrides() {
    assert!(serde_json::from_str::<MetadataFieldInput>(
        r#"{"name":"Amount","kind":"money","created_by":"another-user"}"#
    )
    .is_err());
}

#[test]
fn option_rename_input_cannot_replace_stable_identity() {
    assert!(serde_json::from_str::<MetadataOptionInput>(
        r#"{"label":"Paid","position":0,"id":"00000000-0000-0000-0000-000000000000"}"#
    )
    .is_err());
}

#[test]
fn option_input_trims_labels_without_changing_position() {
    let input: MetadataOptionInput =
        serde_json::from_str(r#"{"label":"  Paid  ","position":3}"#).unwrap();
    let input = input.validate().unwrap();
    assert_eq!(input.label, "Paid");
    assert_eq!(input.position, 3);
}

#[test]
fn field_name_limit_counts_unicode_characters() {
    let input: MetadataFieldInput =
        serde_json::from_value(serde_json::json!({"name": "é".repeat(128), "kind": "text"}))
            .unwrap();
    assert!(input.validate().is_ok());
}

#[test]
fn field_name_limit_rejects_excess_characters() {
    let input: MetadataFieldInput =
        serde_json::from_value(serde_json::json!({"name": "é".repeat(129), "kind": "text"}))
            .unwrap();
    assert!(input.validate().is_err());
}
