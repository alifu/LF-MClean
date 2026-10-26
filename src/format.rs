//! Sizes and ages as people read them.

use chrono::{DateTime, FixedOffset};

const KILO: u64 = 1000;
const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
const DAYS_PER_WEEK: i64 = 7;
const DAYS_PER_MONTH: i64 = 30;
const DAYS_PER_YEAR: i64 = 365;

/// A size the way Finder shows it: powers of 1000, one decimal from KB up (`1.9 GB`).
#[must_use]
pub fn human_size(bytes: u64) -> String {
    if bytes < KILO {
        return format!("{bytes} B");
    }
    let (value, unit) = scaled(bytes);
    format!("{value:.1} {unit}")
}

/// Divides by 1000 until the value, as it will be printed with one decimal, is below 1000.
#[allow(clippy::cast_precision_loss)] // a rounded display value; 53 bits is far more than needed
fn scaled(bytes: u64) -> (f64, &'static str) {
    let mut value = bytes as f64;
    let mut unit = 0;
    while rounded_tenths(value) >= KILO as f64 && unit < UNITS.len() - 1 {
        value /= KILO as f64;
        unit += 1;
    }
    (value, UNITS[unit])
}

fn rounded_tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// How long ago `then` was, in the largest fitting unit (`today`, `3 d ago`, `7 mo ago`).
#[must_use]
pub fn age(then: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> String {
    let days = now.signed_duration_since(then).num_days();
    match days {
        ..=0 => "today".to_string(),
        1 => "yesterday".to_string(),
        2..DAYS_PER_WEEK => format!("{days} d ago"),
        DAYS_PER_WEEK..DAYS_PER_MONTH => format!("{} w ago", days / DAYS_PER_WEEK),
        DAYS_PER_MONTH..DAYS_PER_YEAR => format!("{} mo ago", (days / DAYS_PER_MONTH).min(11)),
        _ => format!("{} y ago", days / DAYS_PER_YEAR),
    }
}

/// Text safe to print on a terminal: control characters, which a folder name from a cloned
/// repository could use to rewrite the screen, are shown escaped (`\u{1b}`).
#[must_use]
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(7 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 26, 12, 0, 0)
            .unwrap()
    }

    #[test]
    fn bytes_have_no_decimal() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
    }

    #[test]
    fn larger_sizes_have_one_decimal() {
        assert_eq!(human_size(1_000), "1.0 KB");
        assert_eq!(human_size(612_000), "612.0 KB");
        assert_eq!(human_size(1_900_000_000), "1.9 GB");
        assert_eq!(human_size(8_200_000_000), "8.2 GB");
        assert_eq!(human_size(2_500_000_000_000), "2.5 TB");
    }

    #[test]
    fn a_size_just_under_a_unit_does_not_round_up_to_1000() {
        assert_eq!(human_size(999_999), "1.0 MB");
        assert_eq!(human_size(999_949), "999.9 KB");
    }

    #[test]
    fn sizes_past_the_largest_unit_stay_in_tb() {
        assert_eq!(human_size(u64::MAX), "18446744.1 TB");
    }

    #[test]
    fn ages_use_the_largest_fitting_unit() {
        let ago = |days| age(now() - Duration::days(days), now());
        assert_eq!(ago(0), "today");
        assert_eq!(ago(1), "yesterday");
        assert_eq!(ago(3), "3 d ago");
        assert_eq!(ago(14), "2 w ago");
        assert_eq!(ago(210), "7 mo ago");
        assert_eq!(ago(364), "11 mo ago");
        assert_eq!(ago(800), "2 y ago");
    }

    #[test]
    fn a_time_in_the_future_reads_as_today() {
        assert_eq!(age(now() + Duration::days(2), now()), "today");
    }

    #[test]
    fn printable_text_escapes_control_characters_only() {
        assert_eq!(printable("App-abc ü 📦"), "App-abc ü 📦");
        assert_eq!(printable("a\u{1b}[2Jb\nc"), "a\\u{1b}[2Jb\\u{a}c");
    }
}
