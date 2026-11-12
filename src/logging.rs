//! Minimal `log` backend: `RUST_LOG` level, UTC millisecond timestamp,
//! stderr. It replaces `simple_logger` (MIT but pulling MPL-2.0
//! `colored`) with the `log` crate we already depend on, so the binary
//! keeps the same diagnostic output without a non-permissive crate.
use log::{LevelFilter, Log, Metadata, Record};
use std::time::{SystemTime, UNIX_EPOCH};

/// `Info` by default: the bar's own lines are useful, the per-frame
/// `calloop`/`zbus`/`wgpu` trace is not (it can write tens of thousands
/// of lines in minutes). Trace-level debugging is opt-in with
/// `RUST_LOG=trace`.
const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

struct Logger {
    level: LevelFilter,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level().to_level_filter() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        eprintln!(
            "{} {:<5} [{}] {}",
            utc_timestamp(),
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {}
}

/// Installs the logger once; a second call is a no-op like the old
/// `SimpleLogger::init` (the error is ignored).
pub(crate) fn init() {
    let level = level_from_spec(std::env::var("RUST_LOG").ok().as_deref());
    if log::set_boxed_logger(Box::new(Logger { level })).is_ok() {
        log::set_max_level(level);
    }
}

/// `RUST_LOG` as a plain level (`error`, `warn`, `info`, `debug`,
/// `trace`, `off`); anything else keeps the `Info` default.
fn level_from_spec(spec: Option<&str>) -> LevelFilter {
    spec.and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_LEVEL)
}

fn utc_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    stamp(now.as_secs(), now.subsec_millis())
}

/// RFC 3339 UTC with milliseconds, the format the old logger printed.
fn stamp(seconds: u64, millis: u32) -> String {
    let days = seconds / 86_400;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's
/// `civil_from_days`, valid for the whole proleptic Gregorian range).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level filter mirrors the old `.env()` behaviour: a plain
    /// level is honoured, an absent or malformed value is `Info`.
    #[test]
    fn level_comes_from_rust_log() {
        assert_eq!(level_from_spec(None), LevelFilter::Info);
        assert_eq!(level_from_spec(Some("")), LevelFilter::Info);
        assert_eq!(level_from_spec(Some("wat")), LevelFilter::Info);
        assert_eq!(level_from_spec(Some("info")), LevelFilter::Info);
        assert_eq!(level_from_spec(Some("DEBUG")), LevelFilter::Debug);
        assert_eq!(level_from_spec(Some(" off ")), LevelFilter::Off);
    }

    /// The timestamp is the RFC 3339 UTC form the old logger printed.
    #[test]
    fn timestamp_is_rfc3339_utc() {
        assert_eq!(stamp(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(stamp(1_700_000_000, 123), "2023-11-14T22:13:20.123Z");
        assert_eq!(stamp(1_709_164_800, 0), "2024-02-29T00:00:00.000Z");
    }
}
