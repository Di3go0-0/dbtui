//! SQL Server value → display string conversion.
//!
//! Unlike sqlx, tiberius hands back a typed `ColumnData` enum per cell, so
//! rendering is an exhaustive match rather than a chain of decoder guesses.
//! Temporal variants still go through `try_get`, because the TDS → chrono
//! conversion lives behind tiberius' private helpers.

use tiberius::{ColumnData, Row};

/// Render every cell of a row as a display string, in column order.
///
/// Rows are converted whole rather than cell-by-cell because `Row::cells()` is
/// a sequential iterator — indexing into it per column would be quadratic.
pub fn mssql_row_to_strings(row: &Row) -> Vec<String> {
    row.cells()
        .enumerate()
        .map(|(idx, (_, data))| cell_to_string(row, idx, data))
        .collect()
}

fn cell_to_string(row: &Row, idx: usize, data: &ColumnData<'static>) -> String {
    match data {
        ColumnData::U8(v) => opt(v.as_ref()),
        ColumnData::I16(v) => opt(v.as_ref()),
        ColumnData::I32(v) => opt(v.as_ref()),
        ColumnData::I64(v) => opt(v.as_ref()),
        ColumnData::F32(v) => opt(v.as_ref()),
        ColumnData::F64(v) => opt(v.as_ref()),
        // BIT is an integer type in SQL Server; SSMS and DBeaver both show 1/0
        ColumnData::Bit(v) => v.map_or_else(null, |b| if b { "1" } else { "0" }.to_string()),
        ColumnData::String(v) => v.as_ref().map_or_else(null, |s| s.to_string()),
        ColumnData::Guid(v) => opt(v.as_ref()),
        ColumnData::Binary(v) => v.as_ref().map_or_else(null, |b| hex(b)),
        ColumnData::Numeric(v) => v.map_or_else(null, format_numeric),
        ColumnData::Xml(v) => v.as_ref().map_or_else(null, |x| x.to_string()),
        ColumnData::DateTime(v) => temporal(v.is_some(), || datetime(row, idx)),
        ColumnData::SmallDateTime(v) => temporal(v.is_some(), || datetime(row, idx)),
        ColumnData::DateTime2(v) => temporal(v.is_some(), || datetime(row, idx)),
        ColumnData::Time(v) => temporal(v.is_some(), || {
            row.try_get::<chrono::NaiveTime, _>(idx)
                .ok()
                .flatten()
                .map(|t| t.format("%H:%M:%S%.f").to_string())
        }),
        ColumnData::Date(v) => temporal(v.is_some(), || {
            row.try_get::<chrono::NaiveDate, _>(idx)
                .ok()
                .flatten()
                .map(|d| d.format("%Y-%m-%d").to_string())
        }),
        ColumnData::DateTimeOffset(v) => temporal(v.is_some(), || {
            row.try_get::<chrono::DateTime<chrono::FixedOffset>, _>(idx)
                .ok()
                .flatten()
                .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f%:z").to_string())
        }),
    }
}

fn null() -> String {
    "NULL".to_string()
}

fn opt<T: ToString>(v: Option<&T>) -> String {
    v.map_or_else(null, |v| v.to_string())
}

/// Run `f` only when the cell is non-NULL, so a decoder miss on a real value
/// never renders as the same "NULL" a genuine SQL NULL does.
fn temporal(present: bool, f: impl FnOnce() -> Option<String>) -> String {
    if !present {
        return null();
    }
    f().unwrap_or_else(|| "<datetime?>".to_string())
}

fn datetime(row: &Row, idx: usize) -> Option<String> {
    row.try_get::<chrono::NaiveDateTime, _>(idx)
        .ok()
        .flatten()
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f").to_string())
}

fn hex(bytes: &[u8]) -> String {
    let render = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
    if bytes.len() <= 32 {
        format!("0x{}", render(bytes))
    } else {
        format!("0x{}... ({} bytes)", render(&bytes[..16]), bytes.len())
    }
}

/// Format a DECIMAL/NUMERIC from its scaled i128.
///
/// tiberius' own `Display` delegates to a `Debug` that prints `int_part` and
/// `dec_part` separately; both are negative for a negative value, so -0.45
/// comes out as "0.-45". Formatting from the raw value avoids that entirely.
fn format_numeric(n: tiberius::numeric::Numeric) -> String {
    let scale = n.scale() as u32;
    let value = n.value();
    let sign = if value < 0 { "-" } else { "" };
    let abs = value.unsigned_abs();
    if scale == 0 {
        return format!("{sign}{abs}");
    }
    let divisor = 10u128.pow(scale);
    format!(
        "{sign}{}.{:0width$}",
        abs / divisor,
        abs % divisor,
        width = scale as usize
    )
}

#[cfg(test)]
mod tests {
    use super::format_numeric;
    use tiberius::numeric::Numeric;

    #[test]
    fn positive_with_scale() {
        assert_eq!(
            format_numeric(Numeric::new_with_scale(193996, 2)),
            "1939.96"
        );
    }

    #[test]
    fn zero_scale() {
        assert_eq!(format_numeric(Numeric::new_with_scale(42, 0)), "42");
    }

    #[test]
    fn trailing_zeros_preserved() {
        assert_eq!(format_numeric(Numeric::new_with_scale(15000, 4)), "1.5000");
    }

    #[test]
    fn negative_below_one_keeps_sign() {
        // tiberius' own Display renders this as "0.-45"
        assert_eq!(format_numeric(Numeric::new_with_scale(-45, 2)), "-0.45");
    }

    #[test]
    fn negative_with_integer_part() {
        assert_eq!(
            format_numeric(Numeric::new_with_scale(-193996, 2)),
            "-1939.96"
        );
    }
}
