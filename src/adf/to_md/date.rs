//! `date` node timestamps (milliseconds since the Unix epoch, as a string) to `YYYY-MM-DD`.

use serde_json::Value;

const MS_PER_DAY: i64 = 86_400_000;

/// The timestamp in milliseconds: a decimal string (the schema form) or an integer.
pub(crate) fn timestamp_millis(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => {
            let s = s.trim();
            let digits = s.strip_prefix('-').unwrap_or(s);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            s.parse().ok()
        }
        Value::Number(n) => n.as_i64(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// `(YYYY-MM-DD, is_midnight_utc)` for a timestamp, or `None` outside years 0000–9999.
pub(crate) fn civil_date(millis: i64) -> Option<(String, bool)> {
    let days = millis.div_euclid(MS_PER_DAY);
    let midnight = millis.rem_euclid(MS_PER_DAY) == 0;
    let (y, m, d) = civil_from_days(days);
    (0..=9999)
        .contains(&y)
        .then(|| (format!("{y:04}-{m:02}-{d:02}"), midnight))
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_dates() {
        assert_eq!(civil_date(0), Some(("1970-01-01".into(), true)));
        assert_eq!(
            civil_date(1_792_368_000_000),
            Some(("2026-10-19".into(), true))
        );
        assert_eq!(
            civil_date(1_792_454_400_000),
            Some(("2026-10-20".into(), true))
        );
        assert_eq!(
            civil_date(951_782_400_000),
            Some(("2000-02-29".into(), true))
        );
        assert_eq!(civil_date(-86_400_000), Some(("1969-12-31".into(), true)));
        assert_eq!(
            civil_date(1_582_152_559),
            Some(("1970-01-19".into(), false))
        );
        assert_eq!(civil_date(i64::MAX), None);
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            timestamp_millis(&json!("1792454400000")),
            Some(1_792_454_400_000)
        );
        assert_eq!(timestamp_millis(&json!(5)), Some(5));
        assert_eq!(timestamp_millis(&json!("12a")), None);
        assert_eq!(timestamp_millis(&json!("")), None);
        assert_eq!(timestamp_millis(&json!(1.5)), None);
    }
}
