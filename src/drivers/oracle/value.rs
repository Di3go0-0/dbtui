//! Oracle value → display string conversion.

/// Extract a column value as a display string, handling Oracle-specific types
/// that don't convert directly to String (TIMESTAMP, DATE, NUMBER, BLOB, etc.).
pub(super) fn oracle_col_to_string(row: &oracle::Row, idx: usize) -> String {
    // Try String first (covers VARCHAR2, CHAR, CLOB, NUMBER-as-string, etc.)
    if let Ok(Some(s)) = row.get::<usize, Option<String>>(idx) {
        return s;
    }
    // NULL check
    if row.get::<usize, Option<String>>(idx).is_ok() {
        return "NULL".to_string();
    }
    // Try Timestamp (DATE, TIMESTAMP, TIMESTAMP WITH TIME ZONE, etc.)
    // Oracle DATE also includes time — the oracle crate decodes it as Timestamp
    if let Ok(Some(ts)) = row.get::<usize, Option<oracle::sql_type::Timestamp>>(idx) {
        let ns = ts.nanosecond();
        return if ns > 0 {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
                ts.year(),
                ts.month(),
                ts.day(),
                ts.hour(),
                ts.minute(),
                ts.second(),
                ns / 1_000_000
            )
        } else {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                ts.year(),
                ts.month(),
                ts.day(),
                ts.hour(),
                ts.minute(),
                ts.second()
            )
        };
    }
    // Try IntervalDS (INTERVAL DAY TO SECOND)
    if let Ok(Some(iv)) = row.get::<usize, Option<oracle::sql_type::IntervalDS>>(idx) {
        return format!("{iv}");
    }
    // Try IntervalYM (INTERVAL YEAR TO MONTH)
    if let Ok(Some(iv)) = row.get::<usize, Option<oracle::sql_type::IntervalYM>>(idx) {
        return format!("{iv}");
    }
    // Try raw bytes as hex (BLOB, RAW)
    if let Ok(Some(bytes)) = row.get::<usize, Option<Vec<u8>>>(idx) {
        if bytes.len() <= 32 {
            return bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
        }
        return format!(
            "{}... ({} bytes)",
            bytes[..16]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>(),
            bytes.len()
        );
    }
    // Fallback
    "NULL".to_string()
}
