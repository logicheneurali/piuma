//! The `calendar` sensor's month grid (roadmap: calendar popup).
//!
//! Pure date math — the month's shape (days, first weekday, the
//! Monday-first matrix) — plus the localized labels fetched from `date`
//! at the moments they matter (popup open, month paging): nothing here
//! runs on the tick.

use std::process::Command;

/// One `date` invocation's trimmed stdout, `None` when the command is
/// missing or fails — the single spawn behind every label below.
fn date_output(args: &[&str]) -> Option<String> {
    let out = Command::new("date").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Days in `month` of `year` (Gregorian calendar, all leap rules);
/// `0` for a month outside `1..=12`.
pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The weekday of the month's first day, Monday-first (`0` = Monday),
/// via Sakamoto's algorithm (valid for months `1..=12`).
pub fn first_weekday(year: u16, month: u8) -> u8 {
    static TABLE: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let year = year as i32 - i32::from(month < 3);
    let dow = (year + year / 4 - year / 100 + year / 400 + TABLE[(month - 1) as usize] + 1) % 7;
    ((dow + 6) % 7) as u8 // Sakamoto counts Sunday = 0
}

/// The Monday-first month matrix: up to six weeks of optional days,
/// `None` padding the cells before the 1st and after the last.
pub fn month_matrix(year: u16, month: u8) -> [[Option<u8>; 7]; 6] {
    let mut weeks = [[None; 7]; 6];
    let (mut row, mut col) = (0, first_weekday(year, month) as usize);
    let mut day = 1;
    while day <= days_in_month(year, month) {
        weeks[row][col] = Some(day);
        day += 1;
        col += 1;
        if col == 7 {
            col = 0;
            row += 1;
        }
    }
    weeks
}

/// `(year, month)` shifted by `delta` whole months (month stays in
/// `1..=12`, the year rolls over in both directions).
pub fn shift_month(year: u16, month: u8, delta: i32) -> (u16, u8) {
    let index = year as i32 * 12 + i32::from(month) - 1 + delta;
    ((index / 12) as u16, (index % 12 + 1) as u8)
}

/// Today's local `(year, month, day)` from `date`, `None` when the
/// command fails — without it the popup has no month to open on.
pub fn today() -> Option<(u16, u8, u8)> {
    let raw = date_output(&["+%Y|%m|%d"])?;
    let mut parts = raw.split('|');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((year, month, day))
}

/// The Monday-first weekday labels for the header, localized by the
/// environment's `LC_TIME` ("lun", "mar", …): one `date` spawn per
/// column, fetched once per process (the locale never changes while
/// piuma runs) with an English fallback if `date -d` is unavailable.
pub fn weekday_labels() -> [String; 7] {
    static CACHE: std::sync::OnceLock<[String; 7]> = std::sync::OnceLock::new();
    const FALLBACK: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    CACHE
        .get_or_init(|| {
            // 2024-01-01 is a Monday: the seven days from it name the columns.
            std::array::from_fn(|i| {
                let day = format!("2024-01-0{}", i + 1);
                date_output(&["-d", &day, "+%a"]).unwrap_or_else(|| FALLBACK[i].to_string())
            })
        })
        .clone()
}

/// The header's localized month label ("ottobre 2026"); a numeric
/// fallback keeps the header readable if `date -d` fails.
pub fn month_label(year: u16, month: u8) -> String {
    let first = format!("{year:04}-{month:02}-01");
    date_output(&["-d", &first, "+%B %Y"])
        .unwrap_or_else(|| format!("{year:04}-{month:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Month lengths, the Gregorian leap rules included: century years
    /// only leap when divisible by 400 (1900 = 28, 2000 = 29).
    #[test]
    fn days_in_month_covers_leap_years() {
        assert_eq!(days_in_month(2026, 10), 31);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 13), 0, "no such month");
    }

    /// The grid's Monday-first anchor, cross-checked against dates the
    /// session shows: 2026-10-01 is a Thursday and 2024-01-01 a Monday.
    #[test]
    fn first_weekday_is_monday_first() {
        assert_eq!(first_weekday(2026, 10), 3, "1 Oct 2026 is a Thursday");
        assert_eq!(first_weekday(2024, 1), 0, "1 Jan 2024 is a Monday");
        assert_eq!(first_weekday(2026, 11), 6, "1 Nov 2026 is a Sunday");
        // Day 11 of that October is a Sunday: 3 + 10 ≡ 6 (mod 7).
        assert_eq!((first_weekday(2026, 10) + 10) % 7, 6, "11 Oct 2026");
    }

    /// October 2026: the first row starts on Thursday (the four cells
    /// before the 1st stay empty), the 31st lands in the fifth week and
    /// the sixth row is empty padding.
    #[test]
    fn month_matrix_shapes_october_2026() {
        let weeks = month_matrix(2026, 10);
        assert_eq!(weeks[0], [None, None, None, Some(1), Some(2), Some(3), Some(4)]);
        assert_eq!(weeks[0][3], Some(1), "first day on Thursday");
        assert_eq!(weeks[4][5], Some(31), "the 31st is the fifth week's Friday");
        assert!(weeks[5].iter().all(|d| d.is_none()), "sixth row empty");
        // Every day of the month appears exactly once, in order.
        let days: Vec<u8> = weeks
            .iter()
            .flatten()
            .flatten()
            .copied()
            .collect();
        assert_eq!(days, (1..=31).collect::<Vec<u8>>());
    }

    /// Paging across the year boundary in both directions, and far
    /// enough out to cross several at once.
    #[test]
    fn shift_month_rolls_the_year() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2024, 1, -1), (2023, 12));
        assert_eq!(shift_month(2026, 10, 14), (2027, 12));
        assert_eq!(shift_month(2026, 10, -10), (2025, 12));
        assert_eq!(shift_month(2026, 10, 0), (2026, 10));
    }
}
