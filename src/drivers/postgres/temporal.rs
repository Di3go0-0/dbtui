//! Date/time rendering straight from PostgreSQL's binary wire format.
//!
//! sqlx's chrono decoders add the wire value to the 2000-01-01 epoch without
//! an overflow check, so `infinity`, `-infinity` and any timestamp outside
//! chrono's range panic inside the driver. Reading the integers here avoids
//! that and keeps what chrono cannot represent: the two sentinels, `24:00:00`
//! and the sign of an interval.

use chrono::{Datelike, NaiveDate, TimeDelta};

const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_DAY: i64 = 86_400 * MICROS_PER_SEC;

/// Render a binary-format temporal value, or `None` when `type_name` (upper
/// case, as sqlx reports it) is not a temporal type or the payload has an
/// unexpected length.
pub(super) fn binary_temporal_to_string(type_name: &str, bytes: &[u8]) -> Option<String> {
    match type_name {
        "TIMESTAMP" => Some(format_timestamp(read_i64(bytes, 0)?, "")),
        // Rendered in UTC: the binary format carries no session time zone.
        "TIMESTAMPTZ" => Some(format_timestamp(read_i64(bytes, 0)?, "+0000")),
        "DATE" => Some(format_date(read_i32(bytes, 0)?)),
        "TIME" => Some(format_time(read_i64(bytes, 0)?)),
        "TIMETZ" => {
            // The zone is stored as seconds *west* of UTC.
            let west = read_i32(bytes, 8)?;
            Some(format!(
                "{}{}",
                format_time(read_i64(bytes, 0)?),
                format_utc_offset(-i64::from(west))
            ))
        }
        "INTERVAL" => Some(format_interval(
            read_i32(bytes, 12)?,
            read_i32(bytes, 8)?,
            read_i64(bytes, 0)?,
        )),
        _ => None,
    }
}

fn read_i64(bytes: &[u8], at: usize) -> Option<i64> {
    let slice = bytes.get(at..at + 8)?;
    Some(i64::from_be_bytes(slice.try_into().ok()?))
}

fn read_i32(bytes: &[u8], at: usize) -> Option<i32> {
    let slice = bytes.get(at..at + 4)?;
    Some(i32::from_be_bytes(slice.try_into().ok()?))
}

fn epoch() -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(2000, 1, 1)
}

/// Microseconds since 2000-01-01 → `YYYY-MM-DD HH:MM:SS[.ffffff]` + `suffix`.
/// The sentinels carry no suffix, as in psql.
fn format_timestamp(micros: i64, suffix: &str) -> String {
    match micros {
        i64::MAX => return "infinity".to_string(),
        i64::MIN => return "-infinity".to_string(),
        _ => {}
    }
    let days = micros.div_euclid(MICROS_PER_DAY);
    let in_day = micros.rem_euclid(MICROS_PER_DAY);
    let date = epoch().and_then(|e| e.checked_add_signed(TimeDelta::try_days(days)?));
    match date {
        Some(date) => format!(
            "{} {}{suffix}{}",
            date_part(date),
            format_time(in_day),
            era(date)
        ),
        None => "<timestamp out of range>".to_string(),
    }
}

/// Days since 2000-01-01 → `YYYY-MM-DD`.
fn format_date(days: i32) -> String {
    match days {
        i32::MAX => return "infinity".to_string(),
        i32::MIN => return "-infinity".to_string(),
        _ => {}
    }
    let date = epoch().and_then(|e| e.checked_add_signed(TimeDelta::try_days(i64::from(days))?));
    match date {
        Some(date) => format!("{}{}", date_part(date), era(date)),
        None => "<date out of range>".to_string(),
    }
}

/// `YYYY-MM-DD`, with the year counted the way PostgreSQL prints BC dates
/// (there is no year zero: chrono's year 0 is 1 BC).
fn date_part(date: NaiveDate) -> String {
    let year = if date.year() <= 0 {
        1 - date.year()
    } else {
        date.year()
    };
    format!("{year:04}-{:02}-{:02}", date.month(), date.day())
}

fn era(date: NaiveDate) -> &'static str {
    if date.year() <= 0 { " BC" } else { "" }
}

/// Microseconds since midnight → `HH:MM:SS[.ffffff]`. Not routed through
/// chrono so `24:00:00`, which PostgreSQL accepts, does not wrap to midnight.
fn format_time(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    format!("{sign}{}", clock(micros.unsigned_abs()))
}

/// `HH:MM:SS[.ffffff]` for a non-negative microsecond count. Hours are not
/// capped, which is what an interval needs.
fn clock(micros: u64) -> String {
    let per_sec = MICROS_PER_SEC.unsigned_abs();
    let total_secs = micros / per_sec;
    format!(
        "{:02}:{:02}:{:02}{}",
        total_secs / 3600,
        (total_secs % 3600) / 60,
        total_secs % 60,
        fraction(micros % per_sec)
    )
}

/// Fractional seconds with trailing zeros trimmed: `.5`, `.000001`, or empty.
fn fraction(micros: u64) -> String {
    if micros == 0 {
        return String::new();
    }
    let digits = format!("{micros:06}");
    format!(".{}", digits.trim_end_matches('0'))
}

/// Seconds east of UTC → `+HH`, `+HH:MM` or `+HH:MM:SS`.
fn format_utc_offset(seconds_east: i64) -> String {
    let sign = if seconds_east < 0 { '-' } else { '+' };
    let abs = seconds_east.unsigned_abs();
    let (h, m, s) = (abs / 3600, (abs % 3600) / 60, abs % 60);
    match (m, s) {
        (0, 0) => format!("{sign}{h:02}"),
        (_, 0) => format!("{sign}{h:02}:{m:02}"),
        _ => format!("{sign}{h:02}:{m:02}:{s:02}"),
    }
}

/// Format an interval the way psql prints one in the default `postgres`
/// style: `1 year 2 mons -3 days +04:05:06.5`.
///
/// Each field keeps its own sign. A positive field that follows a negative
/// one gets an explicit `+`, and the time part is signed once as a whole
/// rather than per component.
pub(super) fn format_interval(months: i32, days: i32, micros: i64) -> String {
    let mut out = String::new();
    let mut after_negative = false;

    for (value, unit) in [(months / 12, "year"), (months % 12, "mon"), (days, "day")] {
        if value == 0 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        if after_negative && value > 0 {
            out.push('+');
        }
        let plural = if value == 1 { "" } else { "s" };
        out.push_str(&format!("{value} {unit}{plural}"));
        after_negative = value < 0;
    }

    if out.is_empty() || micros != 0 {
        let sign = match (micros < 0, after_negative) {
            (true, _) => "-",
            (false, true) => "+",
            (false, false) => "",
        };
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("{sign}{}", clock(micros.unsigned_abs())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(micros: i64) -> String {
        format_timestamp(micros, "")
    }

    #[test]
    fn timestamp_sentinels() {
        assert_eq!(ts(i64::MAX), "infinity");
        assert_eq!(ts(i64::MIN), "-infinity");
        assert_eq!(format_timestamp(i64::MAX, "+0000"), "infinity");
        assert_eq!(format_date(i32::MAX), "infinity");
        assert_eq!(format_date(i32::MIN), "-infinity");
    }

    #[test]
    fn timestamp_at_and_around_the_epoch() {
        assert_eq!(ts(0), "2000-01-01 00:00:00");
        assert_eq!(ts(-1), "1999-12-31 23:59:59.999999");
        assert_eq!(ts(MICROS_PER_DAY + 3_723_500_000), "2000-01-02 01:02:03.5");
    }

    #[test]
    fn timestamp_keeps_fractional_seconds() {
        // 2024-03-15 10:30:00.123456
        let days = 8840;
        let micros = days * MICROS_PER_DAY + (10 * 3600 + 30 * 60) * MICROS_PER_SEC + 123_456;
        assert_eq!(ts(micros), "2024-03-15 10:30:00.123456");
        assert_eq!(
            format_timestamp(micros, "+0000"),
            "2024-03-15 10:30:00.123456+0000"
        );
    }

    #[test]
    fn timestamp_beyond_chrono_range_does_not_panic() {
        // PostgreSQL's own maximum is 294276 AD; chrono stops at 262142.
        assert_eq!(ts(i64::MAX - 1), "<timestamp out of range>");
        assert_eq!(ts(i64::MIN + 1), "<timestamp out of range>");
        assert_eq!(format_date(i32::MAX - 1), "<date out of range>");
    }

    #[test]
    fn dates_before_the_common_era() {
        // chrono counts astronomically: its year -43 is 44 BC.
        let ides = NaiveDate::from_ymd_opt(-43, 3, 15)
            .zip(epoch())
            .map(|(d, e)| d.signed_duration_since(e).num_days());
        let days = ides.and_then(|d| i32::try_from(d).ok()).unwrap_or(0);
        assert_eq!(format_date(days), "0044-03-15 BC");
        assert_eq!(
            format_timestamp(i64::from(days) * MICROS_PER_DAY, "+0000"),
            "0044-03-15 00:00:00+0000 BC"
        );
        assert_eq!(format_date(0), "2000-01-01");
        assert_eq!(format_date(-1), "1999-12-31");
    }

    #[test]
    fn time_of_day() {
        assert_eq!(format_time(0), "00:00:00");
        assert_eq!(
            format_time(13 * 3600 * MICROS_PER_SEC + 250_000),
            "13:00:00.25"
        );
        assert_eq!(format_time(MICROS_PER_DAY), "24:00:00");
    }

    #[test]
    fn utc_offsets() {
        assert_eq!(format_utc_offset(0), "+00");
        assert_eq!(format_utc_offset(7200), "+02");
        assert_eq!(format_utc_offset(-18_000), "-05");
        assert_eq!(format_utc_offset(19_800), "+05:30");
        assert_eq!(format_utc_offset(-1_172), "-00:19:32");
    }

    #[test]
    fn interval_zero_and_simple() {
        assert_eq!(format_interval(0, 0, 0), "00:00:00");
        assert_eq!(format_interval(0, 0, 90 * 60 * MICROS_PER_SEC), "01:30:00");
        assert_eq!(format_interval(1, 2, 0), "1 mon 2 days");
        assert_eq!(format_interval(14, 1, 0), "1 year 2 mons 1 day");
    }

    #[test]
    fn negative_interval_is_signed_once() {
        // Used to print as "-1:-30:00".
        assert_eq!(
            format_interval(0, 0, -90 * 60 * MICROS_PER_SEC),
            "-01:30:00"
        );
        assert_eq!(format_interval(0, 0, -500_000), "-00:00:00.5");
        assert_eq!(format_interval(0, -1, 0), "-1 days");
    }

    #[test]
    fn interval_mixed_signs_follow_psql() {
        assert_eq!(
            format_interval(0, -1, 2 * 3600 * MICROS_PER_SEC),
            "-1 days +02:00:00"
        );
        assert_eq!(
            format_interval(
                14,
                -3,
                4 * 3600 * MICROS_PER_SEC + 5 * 60 * MICROS_PER_SEC + 6_500_000
            ),
            "1 year 2 mons -3 days +04:05:06.5"
        );
        assert_eq!(format_interval(-14, 3, 0), "-1 years -2 mons +3 days");
        assert_eq!(
            format_interval(0, 100, 25 * 3600 * MICROS_PER_SEC),
            "100 days 25:00:00"
        );
    }

    #[test]
    fn binary_dispatch_reads_big_endian_payloads() {
        assert_eq!(
            binary_temporal_to_string("TIMESTAMP", &i64::MAX.to_be_bytes()).as_deref(),
            Some("infinity")
        );
        assert_eq!(
            binary_temporal_to_string("TIMESTAMPTZ", &0i64.to_be_bytes()).as_deref(),
            Some("2000-01-01 00:00:00+0000")
        );
        assert_eq!(
            binary_temporal_to_string("DATE", &i32::MIN.to_be_bytes()).as_deref(),
            Some("-infinity")
        );

        let mut timetz = (10 * 3600 * MICROS_PER_SEC).to_be_bytes().to_vec();
        timetz.extend_from_slice(&(-7200i32).to_be_bytes());
        assert_eq!(
            binary_temporal_to_string("TIMETZ", &timetz).as_deref(),
            Some("10:00:00+02")
        );

        let mut interval = (-MICROS_PER_SEC).to_be_bytes().to_vec();
        interval.extend_from_slice(&2i32.to_be_bytes());
        interval.extend_from_slice(&1i32.to_be_bytes());
        assert_eq!(
            binary_temporal_to_string("INTERVAL", &interval).as_deref(),
            Some("1 mon 2 days -00:00:01")
        );
    }

    #[test]
    fn binary_dispatch_rejects_other_types_and_short_payloads() {
        assert_eq!(binary_temporal_to_string("INT8", &0i64.to_be_bytes()), None);
        assert_eq!(binary_temporal_to_string("TIMESTAMP", &[0, 1, 2]), None);
        assert_eq!(
            binary_temporal_to_string("TIMETZ", &0i64.to_be_bytes()),
            None
        );
        assert_eq!(binary_temporal_to_string("INTERVAL", &[]), None);
    }
}
