use folio_core::models::DocumentDate;

#[test]
fn dates_round_trip_without_time_or_timezone() {
    for value in [
        "0001-01-01",
        "1582-10-10",
        "2000-02-29",
        "2024-03-10",
        "9999-12-31",
    ] {
        let date: DocumentDate = value.parse().unwrap();
        assert_eq!(date.to_string(), value);
        let json = serde_json::to_string(value).unwrap();
        assert_eq!(serde_json::to_string(&date).unwrap(), json);
        assert_eq!(serde_json::from_str::<DocumentDate>(&json).unwrap(), date);
    }
}

#[test]
fn malformed_and_impossible_dates_are_rejected() {
    for value in [
        "0000-01-01",
        "1900-02-29",
        "2025-02-29",
        "2026-04-31",
        "2026-1-01",
        "2026-01-00",
        "2026-13-01",
        "2026-11-02T00:00:00Z",
        " 2026-11-02",
        "2026-11-02 ",
        "２０２６-１１-０２",
    ] {
        assert!(value.parse::<DocumentDate>().is_err(), "accepted {value}");
        let json = serde_json::to_string(value).unwrap();
        assert!(
            serde_json::from_str::<DocumentDate>(&json).is_err(),
            "decoded {value}"
        );
    }
}
