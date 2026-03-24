#[test]
fn test_generate_id_returns_valid_uuidv7() {
    let id = temper_cli::ids::generate_id();
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "7", "version nibble should be 7");
}

#[test]
fn test_generate_id_from_date_uses_timestamp() {
    let id1 = temper_cli::ids::generate_id_from_date("2026-01-01");
    let id2 = temper_cli::ids::generate_id_from_date("2026-06-15");
    assert!(
        id2 > id1,
        "later date should produce lexically greater UUID"
    );
}

#[test]
fn test_generate_id_from_date_invalid_falls_back() {
    let id = temper_cli::ids::generate_id_from_date("not-a-date");
    assert_eq!(id.len(), 36);
}
