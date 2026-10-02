//! MySQL value → display string conversion.
//!
//! User statements are sent with the text protocol (see `exec`), where the
//! server renders every value itself — the same text the `mysql` client
//! prints. That sidesteps the binary-protocol decoders entirely, and with
//! them the cases they get wrong or panic on in sqlx 0.7: `FLOAT` read as an
//! 8-byte double, `YEAR`, `TIME` beyond 24 hours or negative, zero and
//! partial-zero dates, and fractional seconds.
//!
//! Only two families still need work on the client: binary strings, which are
//! shown as hex instead of mojibake, and `BIT`, which arrives as raw bytes.

use sqlx::{Row, TypeInfo, ValueRef};

/// Bytes of a binary value shown in full; longer ones are abbreviated.
const HEX_FULL_LIMIT: usize = 32;
/// Leading bytes shown for an abbreviated binary value.
const HEX_PREVIEW: usize = 16;

/// Render one column of a text-protocol row as a display string. "NULL" is
/// returned only for SQL NULL.
pub(super) fn mysql_value_to_string(row: &sqlx::mysql::MySqlRow, idx: usize) -> String {
    let Ok(raw) = row.try_get_raw(idx) else {
        return "NULL".to_string();
    };
    let type_name = raw.type_info().name().to_uppercase();
    // The raw bytes are taken instead of asking `is_null()`: sqlx reports zero
    // dates as NULL, while the server sent a real `0000-00-00`.
    match <&[u8] as sqlx::Decode<sqlx::MySql>>::decode(raw) {
        Ok(bytes) => text_value_to_string(&type_name, bytes),
        Err(_) => "NULL".to_string(),
    }
}

/// Render the text-protocol payload of a non-NULL value of type `type_name`
/// (upper case, as sqlx names it).
fn text_value_to_string(type_name: &str, bytes: &[u8]) -> String {
    match type_name {
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "GEOMETRY" => {
            hex(bytes)
        }
        "BIT" => bit_to_string(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `0x`-prefixed hex, abbreviated for long values.
fn hex(bytes: &[u8]) -> String {
    let render = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
    if bytes.len() <= HEX_FULL_LIMIT {
        format!("0x{}", render(bytes))
    } else {
        format!(
            "0x{}... ({} bytes)",
            render(&bytes[..HEX_PREVIEW]),
            bytes.len()
        )
    }
}

/// A `BIT(n)` value as an unsigned integer. The server sends it big-endian
/// in both protocols; `BIT` holds at most 64 bits.
fn bit_to_string(bytes: &[u8]) -> String {
    if bytes.len() > 8 {
        return hex(bytes);
    }
    bytes
        .iter()
        .fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_rendered_text_is_passed_through() {
        // Everything the binary decoders used to get wrong arrives as text.
        assert_eq!(text_value_to_string("FLOAT", b"1.5"), "1.5");
        assert_eq!(text_value_to_string("YEAR", b"2024"), "2024");
        assert_eq!(text_value_to_string("TIME", b"838:59:59"), "838:59:59");
        assert_eq!(
            text_value_to_string("TIME", b"-12:30:00.250000"),
            "-12:30:00.250000"
        );
        assert_eq!(text_value_to_string("DATE", b"0000-00-00"), "0000-00-00");
        assert_eq!(text_value_to_string("DATE", b"2024-00-00"), "2024-00-00");
        assert_eq!(
            text_value_to_string("DATETIME", b"2024-03-15 10:30:00.123456"),
            "2024-03-15 10:30:00.123456"
        );
        assert_eq!(
            text_value_to_string("BIGINT UNSIGNED", b"18446744073709551615"),
            "18446744073709551615"
        );
        assert_eq!(text_value_to_string("DECIMAL", b"-0.45"), "-0.45");
        assert_eq!(text_value_to_string("VARCHAR", "año".as_bytes()), "año");
    }

    #[test]
    fn empty_string_is_not_null() {
        assert_eq!(text_value_to_string("VARCHAR", b""), "");
    }

    #[test]
    fn invalid_utf8_is_replaced_not_dropped() {
        assert_eq!(
            text_value_to_string("VARCHAR", &[b'a', 0xff, b'b']),
            "a\u{fffd}b"
        );
    }

    #[test]
    fn binary_values_are_hex() {
        assert_eq!(
            text_value_to_string("VARBINARY", &[0xde, 0xad, 0x01]),
            "0xDEAD01"
        );
        assert_eq!(text_value_to_string("BLOB", &[]), "0x");
        let long = vec![0xabu8; 40];
        assert_eq!(
            text_value_to_string("LONGBLOB", &long),
            format!("0x{}... (40 bytes)", "AB".repeat(16))
        );
        let edge = vec![0x01u8; 32];
        assert_eq!(
            text_value_to_string("BINARY", &edge),
            format!("0x{}", "01".repeat(32))
        );
    }

    #[test]
    fn bit_is_big_endian() {
        assert_eq!(text_value_to_string("BIT", &[0x01]), "1");
        assert_eq!(text_value_to_string("BIT", &[0x01, 0x00]), "256");
        assert_eq!(
            text_value_to_string("BIT", &[0xff; 8]),
            u64::MAX.to_string()
        );
        assert_eq!(text_value_to_string("BIT", &[]), "0");
        assert_eq!(
            text_value_to_string("BIT", &[0x01; 9]),
            format!("0x{}", "01".repeat(9))
        );
    }
}
