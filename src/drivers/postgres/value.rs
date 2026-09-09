//! PostgreSQL value → display string conversion.
//!
//! Postgres returns values in the binary wire format, and sqlx only decodes a
//! type when a Rust type declares itself compatible with that OID. Any type
//! without a matching decoder here renders as "NULL", which is
//! indistinguishable from a real NULL — so every type this client is expected
//! to display needs an explicit branch.

use sqlx::{Row, TypeInfo, ValueRef};

/// Render one column of a row as a display string. "NULL" is returned both for
/// SQL NULL and for values no branch below can decode.
pub fn pg_value_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> String {
    let Ok(raw) = row.try_get_raw(idx) else {
        return "NULL".to_string();
    };
    if raw.is_null() {
        return "NULL".to_string();
    }
    let type_name = raw.type_info().name().to_uppercase();

    if let Some(v) = scalar_to_string(row, idx) {
        return v;
    }
    if let Some(v) = temporal_to_string(row, idx) {
        return v;
    }
    if let Some(v) = network_to_string(row, idx) {
        return v;
    }
    if let Some(v) = binary_to_string(row, idx, &type_name) {
        return v;
    }
    if let Some(v) = array_to_string(row, idx) {
        return v;
    }
    printable_fallback(row, idx).unwrap_or_else(|| "NULL".to_string())
}

/// Numbers, strings, booleans and the fixed-width money/oid types.
fn scalar_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    // TEXT, VARCHAR, BPCHAR, NAME, and any text-domain type
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return Some(v);
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<i32, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<i16, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<f32, _>(idx) {
        return Some(v.to_string());
    }
    // NUMERIC / DECIMAL: arbitrary precision, no primitive getter is compatible
    if let Ok(v) = row.try_get::<sqlx::types::BigDecimal, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return Some(v.to_string());
    }
    // MONEY: an i64 of minor units; the scale is lc_monetary, assumed 2 here.
    // The sign is formatted separately so -0.45 does not print as 0.45.
    if let Ok(v) = row.try_get::<sqlx::postgres::types::PgMoney, _>(idx) {
        let sign = if v.0 < 0 { "-" } else { "" };
        let abs = v.0.unsigned_abs();
        return Some(format!("{sign}{}.{:02}", abs / 100, abs % 100));
    }
    // OID and the oid-alias types (regclass, regproc, ...)
    if let Ok(v) = row.try_get::<sqlx::postgres::types::Oid, _>(idx) {
        return Some(v.0.to_string());
    }
    if let Ok(v) = row.try_get::<uuid::Uuid, _>(idx) {
        return Some(v.to_string());
    }
    // JSON / JSONB
    if let Ok(v) = row.try_get::<serde_json::Value, _>(idx) {
        return Some(v.to_string());
    }
    None
}

/// DATE, TIME, TIMESTAMP, TIMESTAMPTZ and INTERVAL.
fn temporal_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(idx) {
        return Some(v.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    if let Ok(v) = row.try_get::<chrono::DateTime<chrono::Utc>, _>(idx) {
        return Some(v.format("%Y-%m-%d %H:%M:%S%z").to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(idx) {
        return Some(v.format("%Y-%m-%d").to_string());
    }
    if let Ok(v) = row.try_get::<chrono::NaiveTime, _>(idx) {
        return Some(v.format("%H:%M:%S").to_string());
    }
    if let Ok(v) = row.try_get::<sqlx::postgres::types::PgInterval, _>(idx) {
        return Some(format_interval(&v));
    }
    None
}

/// Format a PgInterval the way psql prints one: `1 mon 2 days 03:04:05`.
fn format_interval(v: &sqlx::postgres::types::PgInterval) -> String {
    let total_secs = v.microseconds / 1_000_000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let mut parts = Vec::new();
    if v.months != 0 {
        parts.push(format!("{} mon", v.months));
    }
    if v.days != 0 {
        parts.push(format!("{} days", v.days));
    }
    if hours != 0 || mins != 0 || secs != 0 {
        parts.push(format!("{hours:02}:{mins:02}:{secs:02}"));
    }
    if parts.is_empty() {
        "00:00:00".to_string()
    } else {
        parts.join(" ")
    }
}

/// INET, CIDR and MACADDR.
fn network_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    if let Ok(v) = row.try_get::<sqlx::types::ipnetwork::IpNetwork, _>(idx) {
        return Some(v.to_string());
    }
    if let Ok(v) = row.try_get::<sqlx::types::mac_address::MacAddress, _>(idx) {
        return Some(v.to_string());
    }
    None
}

/// BYTEA, shown as a truncated hex literal.
fn binary_to_string(row: &sqlx::postgres::PgRow, idx: usize, type_name: &str) -> Option<String> {
    if type_name != "BYTEA" {
        return None;
    }
    let bytes = row.try_get::<Vec<u8>, _>(idx).ok()?;
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Some(if bytes.len() <= 32 {
        format!("\\x{}", hex(&bytes))
    } else {
        format!("\\x{}... ({} bytes)", hex(&bytes[..32]), bytes.len())
    })
}

/// Arrays of the common element types, rendered in psql's `{a,b,c}` form.
fn array_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    fn join<T: ToString>(items: Vec<Option<T>>) -> String {
        let inner = items
            .iter()
            .map(|v| v.as_ref().map_or("NULL".to_string(), |v| v.to_string()))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{{inner}}}")
    }

    if let Ok(v) = row.try_get::<Vec<Option<String>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<i64>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<i32>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<i16>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<f64>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<f32>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<sqlx::types::BigDecimal>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<bool>>, _>(idx) {
        return Some(join(v));
    }
    if let Ok(v) = row.try_get::<Vec<Option<uuid::Uuid>>, _>(idx) {
        return Some(join(v));
    }
    None
}

/// Last resort: the raw wire bytes, but only when they are printable text.
///
/// This catches text-format values and text-like types with no decoder (enums,
/// domains, custom types). Binary payloads are rejected so the grid never shows
/// mojibake — those fall through to "NULL".
fn printable_fallback(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    let raw = row.try_get_raw(idx).ok()?;
    let bytes = <&[u8] as sqlx::Decode<sqlx::Postgres>>::decode(raw).ok()?;
    let s = std::str::from_utf8(bytes).ok()?;
    if s.is_empty() || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return None;
    }
    Some(s.to_string())
}
