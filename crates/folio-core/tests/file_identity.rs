use folio_core::models::FileIdentity;

#[test]
fn identity_comes_from_actual_bytes() {
    let identity =
        FileIdentity::from_bytes("documents/doc/revisions/rev/original", "text/plain", b"abc");
    assert_eq!(
        identity.sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(identity.byte_size, 3);
    assert_eq!(identity.mime_type, "text/plain");
    assert_eq!(identity.storage_key, "documents/doc/revisions/rev/original");
}

#[test]
fn empty_file_has_real_digest_and_zero_size() {
    let identity = FileIdentity::from_bytes("original", "application/octet-stream", b"");
    assert_eq!(
        identity.sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(identity.byte_size, 0);
}

#[test]
fn multibyte_text_counts_bytes_not_characters() {
    let identity = FileIdentity::from_bytes("original", "text/plain", "ü".as_bytes());
    assert_eq!(identity.byte_size, 2);
}
