use uuid::{timestamp::Timestamp, Uuid};

/// Generate a new UUIDv7 using the current timestamp.
pub fn generate_id() -> String {
    Uuid::now_v7().to_string()
}

/// Generate a UUIDv7 from a date string (YYYY-MM-DD).
/// Falls back to current timestamp if parsing fails.
pub fn generate_id_from_date(date_str: &str) -> String {
    if let Some(ts) = parse_date_to_timestamp(date_str) {
        Uuid::new_v7(ts).to_string()
    } else {
        generate_id()
    }
}

fn parse_date_to_timestamp(date_str: &str) -> Option<Timestamp> {
    let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let secs = datetime.and_utc().timestamp() as u64;
    Some(Timestamp::from_unix(uuid::NoContext, secs, 0))
}
