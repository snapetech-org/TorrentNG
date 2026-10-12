//! Display formatting shared by the table, detail pane and status bar.

const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

pub fn size(bytes: i64) -> String {
    if bytes <= 0 {
        return "0 B".into();
    }
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
}

pub fn speed(bytes_per_sec: i64) -> String {
    if bytes_per_sec <= 0 {
        "—".into()
    } else {
        format!("{}/s", size(bytes_per_sec))
    }
}

/// `seconds` of remaining time; `None`/non-positive renders as an infinity-free dash.
pub fn eta(seconds: Option<u64>) -> String {
    let Some(s) = seconds else {
        return "—".into();
    };
    match s {
        0 => "0s".into(),
        1..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        3600..=86_399 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        86_400..=8_639_999 => format!("{}d {}h", s / 86_400, (s % 86_400) / 3600),
        _ => "∞".into(),
    }
}

/// Unix seconds as `YYYY-MM-DD` (UTC) without pulling in a date crate.
pub fn date(unix_secs: i64) -> String {
    if unix_secs <= 0 {
        return "—".into();
    }
    // Howard Hinnant's civil_from_days.
    let days = unix_secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.50 KiB");
        assert_eq!(size(5 * 1024 * 1024 * 1024), "5.00 GiB");
        assert_eq!(size(250 * 1024 * 1024 * 1024 * 1024), "250 TiB");
    }

    #[test]
    fn speeds_and_eta() {
        assert_eq!(speed(0), "—");
        assert_eq!(speed(2048), "2.00 KiB/s");
        assert_eq!(eta(None), "—");
        assert_eq!(eta(Some(75)), "1m 15s");
        assert_eq!(eta(Some(7300)), "2h 1m");
        assert_eq!(eta(Some(100_000_000)), "∞");
    }

    #[test]
    fn dates() {
        assert_eq!(date(0), "—");
        assert_eq!(date(86_400), "1970-01-02");
        assert_eq!(date(1_700_000_000), "2023-11-14");
    }
}
