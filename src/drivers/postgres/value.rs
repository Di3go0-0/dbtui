//! PostgreSQL value → display string conversion.
//!
//! Prepared statements return values in the binary wire format, and sqlx only
//! decodes a type when a Rust type declares itself compatible with that OID.
//! A non-NULL value no branch here can decode is shown as `<typename>` —
//! never as "NULL", which would be indistinguishable from a real SQL NULL.

use sqlx::postgres::PgValueFormat;
use sqlx::{Row, TypeInfo, ValueRef};

use crate::drivers::postgres::temporal::binary_temporal_to_string;

/// Render one column of a row as a display string. "NULL" is returned only
/// for SQL NULL.
pub fn pg_value_to_string(row: &sqlx::postgres::PgRow, idx: usize) -> String {
    let Ok(raw) = row.try_get_raw(idx) else {
        return "NULL".to_string();
    };
    if raw.is_null() {
        return "NULL".to_string();
    }
    let type_name = raw.type_info().name().to_uppercase();

    // Unprepared statements come back in the text format, which is already
    // the server's own rendering of the value.
    if raw.format() == PgValueFormat::Text {
        return match raw.as_bytes() {
            Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Err(_) => placeholder(&type_name),
        };
    }

    // Temporal types are read from the raw payload before any typed getter
    // runs: sqlx's chrono decoders panic on `infinity` and out-of-range values.
    if let Ok(bytes) = raw.as_bytes()
        && let Some(v) = binary_temporal_to_string(&type_name, bytes)
    {
        return v;
    }
    if type_name == "NUMERIC"
        && let Ok(bytes) = raw.as_bytes()
        && let Some(v) = numeric_to_string(bytes)
    {
        return v;
    }
    if let Some(v) = scalar_to_string(row, idx) {
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
    printable_fallback(row, idx).unwrap_or_else(|| placeholder(&type_name))
}

/// Stand-in for a non-NULL value of a type this module cannot render.
fn placeholder(type_name: &str) -> String {
    format!("<{}>", type_name.to_lowercase())
}

/// Render a binary NUMERIC exactly as the server would print it.
///
/// The payload is four 16-bit fields — digit count, weight, sign, display
/// scale — followed by the value in base-10000 digits. It is decoded here
/// rather than through `BigDecimal` for two reasons: the sign field doubles as
/// the marker for `NaN` and the infinities, which `BigDecimal` cannot hold,
/// and `BigDecimal` rebuilds the scale from the base-10000 digits, so a
/// `NUMERIC(10,2)` holding `1.50` came out as `1.5000`.
fn numeric_to_string(bytes: &[u8]) -> Option<String> {
    let field = |i: usize| -> Option<i16> {
        let pair = bytes.get(i * 2..i * 2 + 2)?;
        Some(i16::from_be_bytes([pair[0], pair[1]]))
    };
    let digit_count = usize::try_from(field(0)?).ok()?;
    let weight = i64::from(field(1)?);
    let sign = field(2)? as u16;
    let scale = usize::try_from(field(3)?).ok()?;
    match sign {
        0xC000 => return Some("NaN".to_string()),
        0xD000 => return Some("Infinity".to_string()),
        0xF000 => return Some("-Infinity".to_string()),
        0x0000 | 0x4000 => {}
        _ => return None,
    }
    // Digit `i` is worth `digit * 10000^(weight - i)`; positions outside the
    // stored digits are zero.
    let digit_at = |exponent: i64| -> Option<i16> {
        match usize::try_from(weight - exponent) {
            Ok(i) if i < digit_count => field(4 + i),
            _ => Some(0),
        }
    };

    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    if weight < 0 {
        out.push('0');
    } else {
        for exponent in (0..=weight).rev() {
            let digit = digit_at(exponent)?;
            if exponent == weight {
                out.push_str(&digit.to_string());
            } else {
                out.push_str(&format!("{digit:04}"));
            }
        }
    }
    if scale > 0 {
        let mut fraction = String::with_capacity(scale + 4);
        let mut exponent = -1;
        while fraction.len() < scale {
            fraction.push_str(&format!("{:04}", digit_at(exponent)?));
            exponent -= 1;
        }
        out.push('.');
        out.push_str(&fraction[..scale]);
    }
    Some(out)
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
/// This catches text-like types with no decoder (enums, domains, custom
/// types), whose binary format is their text. Binary payloads are rejected so
/// the grid never shows mojibake — those get the `<typename>` placeholder.
fn printable_fallback(row: &sqlx::postgres::PgRow, idx: usize) -> Option<String> {
    let raw = row.try_get_raw(idx).ok()?;
    printable_text(raw.as_bytes().ok()?)
}

/// `bytes` as a string when it is valid UTF-8 free of control characters.
fn printable_text(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    if s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return None;
    }
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::{numeric_to_string, placeholder, printable_text};

    #[test]
    fn numeric_special_values() {
        assert_eq!(
            numeric_to_string(&[0, 0, 0, 0, 0xC0, 0, 0, 0]).as_deref(),
            Some("NaN")
        );
        assert_eq!(
            numeric_to_string(&[0, 0, 0, 0, 0xD0, 0, 0, 0]).as_deref(),
            Some("Infinity")
        );
        assert_eq!(
            numeric_to_string(&[0, 0, 0, 0, 0xF0, 0, 0, 0]).as_deref(),
            Some("-Infinity")
        );
        assert_eq!(numeric_to_string(&[0, 0]), None);
    }

    /// Build a binary NUMERIC payload from its fields.
    fn numeric(weight: i16, sign: u16, scale: i16, digits: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend((digits.len() as i16).to_be_bytes());
        bytes.extend(weight.to_be_bytes());
        bytes.extend(sign.to_be_bytes());
        bytes.extend(scale.to_be_bytes());
        for digit in digits {
            bytes.extend(digit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn numeric_keeps_the_declared_scale() {
        let render = |bytes: Vec<u8>| numeric_to_string(&bytes).unwrap_or_default();
        // 1.50 in NUMERIC(10,2): one integer digit, one fractional group.
        assert_eq!(render(numeric(0, 0, 2, &[1, 5000])), "1.50");
        // Trailing zero groups are not stored at all.
        assert_eq!(render(numeric(0, 0, 2, &[12])), "12.00");
        assert_eq!(render(numeric(0, 0, 0, &[])), "0");
        assert_eq!(render(numeric(0, 0, 3, &[])), "0.000");
        // 12345678.9 — integer groups after the first are zero-padded.
        assert_eq!(render(numeric(1, 0, 1, &[1234, 5678, 9000])), "12345678.9");
        assert_eq!(render(numeric(1, 0, 0, &[1, 2])), "10002");
        // 10000 is stored as a single digit with weight 1.
        assert_eq!(render(numeric(1, 0, 0, &[1])), "10000");
        // -0.00001234 — the first fractional group is an implied zero.
        assert_eq!(render(numeric(-2, 0x4000, 8, &[1234])), "-0.00001234");
    }

    #[test]
    fn placeholder_never_reads_as_null() {
        assert_eq!(placeholder("INT4RANGE"), "<int4range>");
        assert_eq!(placeholder("mood"), "<mood>");
        assert_ne!(placeholder("NULL"), "NULL");
    }

    #[test]
    fn printable_text_accepts_labels_and_multiline_text() {
        assert_eq!(printable_text(b"happy").as_deref(), Some("happy"));
        assert_eq!(
            printable_text("a\tb\nc".as_bytes()).as_deref(),
            Some("a\tb\nc")
        );
        assert_eq!(
            printable_text("caf\u{e9}".as_bytes()).as_deref(),
            Some("caf\u{e9}")
        );
        assert_eq!(printable_text(b"").as_deref(), Some(""));
    }

    #[test]
    fn printable_text_rejects_binary_payloads() {
        assert_eq!(printable_text(&[0, 0, 0, 1]), None);
        assert_eq!(printable_text(&[0xff, 0xfe]), None);
        assert_eq!(printable_text(b"a\x00b"), None);
    }
}
