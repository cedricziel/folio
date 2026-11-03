use folio_core::models::{MetadataKind, MetadataValue};

#[test]
fn decimal_preserves_digits_beyond_binary_float_precision() {
    let value = MetadataValue::Decimal("9007199254740993.0100".into());
    assert_eq!(
        value.validate(MetadataKind::Decimal, false).unwrap(),
        MetadataValue::Decimal("9007199254740993.01".into())
    );
}

#[test]
fn decimal_canonicalizes_negative_zero() {
    assert_eq!(
        MetadataValue::Decimal("-000.000".into())
            .validate(MetadataKind::Decimal, false)
            .unwrap(),
        MetadataValue::Decimal("0".into())
    );
}

#[test]
fn decimal_rejects_exponents() {
    assert!(MetadataValue::Decimal("1e3".into())
        .validate(MetadataKind::Decimal, false)
        .is_err());
}

#[test]
fn decimal_rejects_excess_fractional_precision() {
    assert!(MetadataValue::Decimal("0.0000000000000000001".into())
        .validate(MetadataKind::Decimal, false)
        .is_err());
}

#[test]
fn money_requires_explicit_uppercase_currency() {
    assert!(MetadataValue::Money {
        amount: "12.25".into(),
        currency: "eur".into()
    }
    .validate(MetadataKind::Money, false)
    .is_err());
}

#[test]
fn money_normalizes_amount_without_converting_currency() {
    assert_eq!(
        MetadataValue::Money {
            amount: "0012.250".into(),
            currency: "EUR".into()
        }
        .validate(MetadataKind::Money, false)
        .unwrap(),
        MetadataValue::Money {
            amount: "12.25".into(),
            currency: "EUR".into()
        }
    );
}

#[test]
fn false_is_a_valid_required_boolean() {
    assert_eq!(
        MetadataValue::Boolean(false)
            .validate(MetadataKind::Boolean, true)
            .unwrap(),
        MetadataValue::Boolean(false)
    );
}

#[test]
fn integer_round_trip_preserves_signed_64_bit_limit() {
    let value: MetadataValue =
        serde_json::from_str(r#"{"kind":"integer","value":"9223372036854775807"}"#).unwrap();
    let validated = value.validate(MetadataKind::Integer, false).unwrap();
    assert_eq!(
        serde_json::to_string(&validated).unwrap(),
        r#"{"kind":"integer","value":"9223372036854775807"}"#
    );
}

#[test]
fn integer_rejects_overflow() {
    assert!(MetadataValue::Integer("9223372036854775808".into())
        .validate(MetadataKind::Integer, false)
        .is_err());
}

#[test]
fn invalid_calendar_date_is_rejected() {
    assert!(MetadataValue::Date("2026-02-29".into())
        .validate(MetadataKind::Date, false)
        .is_err());
}

#[test]
fn leap_day_is_retained_without_time_zone() {
    assert_eq!(
        MetadataValue::Date("2024-02-29".into())
            .validate(MetadataKind::Date, false)
            .unwrap(),
        MetadataValue::Date("2024-02-29".into())
    );
}

#[test]
fn url_rejects_script_scheme() {
    assert!(MetadataValue::Url("javascript:alert(1)".into())
        .validate(MetadataKind::Url, false)
        .is_err());
}

#[test]
fn url_rejects_embedded_credentials() {
    assert!(
        MetadataValue::Url("https://user:password@example.com".into())
            .validate(MetadataKind::Url, false)
            .is_err()
    );
}

#[test]
fn url_limit_applies_to_the_normalized_form() {
    let url = format!("https://example.com/{}", "\u{e4}".repeat(3_000));
    assert!(MetadataValue::Url(url)
        .validate(MetadataKind::Url, false)
        .is_err());
}

#[test]
fn value_cannot_be_reinterpreted_as_another_type() {
    assert!(MetadataValue::Text("true".into())
        .validate(MetadataKind::Boolean, false)
        .is_err());
}

#[test]
fn required_text_rejects_whitespace() {
    assert!(MetadataValue::Text(" \n\t".into())
        .validate(MetadataKind::Text, true)
        .is_err());
}

#[test]
fn optional_empty_text_remains_a_value() {
    assert_eq!(
        MetadataValue::Text(String::new())
            .validate(MetadataKind::Text, false)
            .unwrap(),
        MetadataValue::Text(String::new())
    );
}

#[test]
fn decimal_accepts_38_digits_without_rounding() {
    let value = MetadataValue::Decimal("12345678901234567890123456789012345678".into());
    assert_eq!(
        value
            .clone()
            .validate(MetadataKind::Decimal, false)
            .unwrap(),
        value
    );
}

#[test]
fn decimal_rejects_39_significant_digits() {
    assert!(
        MetadataValue::Decimal("123456789012345678901234567890123456789".into())
            .validate(MetadataKind::Decimal, false)
            .is_err()
    );
}

#[test]
fn date_rejects_noncanonical_padding() {
    assert!(MetadataValue::Date("2024-2-29".into())
        .validate(MetadataKind::Date, false)
        .is_err());
}

#[test]
fn wire_value_rejects_unknown_actor_fields() {
    assert!(serde_json::from_str::<MetadataValue>(
        r#"{"kind":"boolean","value":false,"updated_by":"someone-else"}"#
    )
    .is_err());
}

#[test]
fn clearing_a_required_value_is_rejected() {
    assert!(MetadataKind::Boolean.validate_value(None, true).is_err());
}

#[test]
fn clearing_an_optional_value_preserves_absence() {
    assert_eq!(
        MetadataKind::Boolean.validate_value(None, false).unwrap(),
        None
    );
}
