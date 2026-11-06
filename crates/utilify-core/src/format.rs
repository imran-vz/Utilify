//! Human-readable sizes and rates. Decimal units (1 KB = 1000 B), like
//! Finder and Activity Monitor.

const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Scale a value to the largest unit that keeps it >= 1, with a precision
/// that keeps about three significant digits.
fn scaled(value: f64, units: &[&'static str]) -> (String, &'static str) {
    let mut v = value.max(0.0);
    let mut i = 0;
    while v >= 999.5 && i + 1 < units.len() {
        v /= 1000.0;
        i += 1;
    }
    let digits = if i == 0 || v >= 99.95 {
        0
    } else if v >= 9.995 {
        1
    } else {
        2
    };
    (format!("{v:.digits$}"), units[i])
}

/// `1.23 GB`
pub fn bytes(n: u64) -> String {
    let (v, u) = scaled(n as f64, &UNITS);
    format!("{v} {u}")
}

/// `12.3 MB/s`
pub fn rate(bytes_per_sec: f64) -> String {
    let (v, u) = scaled(bytes_per_sec, &UNITS);
    format!("{v} {u}/s")
}

/// Fixed-width (4 character) rate for the menu bar, so the status item
/// doesn't jitter: `  0K`, ` 12K`, `1.2M`, `120M`.
pub fn rate_compact(bytes_per_sec: f64) -> String {
    let units = ["K", "M", "G", "T"];
    let mut v = bytes_per_sec.max(0.0) / 1000.0;
    let mut i = 0;
    while v >= 999.5 && i + 1 < units.len() {
        v /= 1000.0;
        i += 1;
    }
    let num = if v < 9.95 && i > 0 {
        format!("{v:.1}")
    } else {
        format!("{v:.0}")
    };
    format!("{num:>3}{}", units[i])
}

/// Fixed-width (7 character) rate with its unit, for the menu bar:
/// `  0KB/s`, ` 12KB/s`, `9.2MB/s`, `120MB/s`.
pub fn rate_menu(bytes_per_sec: f64) -> String {
    format!("{}B/s", rate_compact(bytes_per_sec))
}

/// Link speed: `1 Gbps`, `866 Mbps`, `2.5 Gbps`.
pub fn bits_per_sec(bps: u64) -> String {
    let units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
    let mut v = bps as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < units.len() {
        v /= 1000.0;
        i += 1;
    }
    let s = if v >= 10.0 || (v - v.round()).abs() < 0.05 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    };
    format!("{s} {}", units[i])
}

/// `42%`, or `<1%` for small but non-zero shares.
pub fn percent(fraction: f64) -> String {
    let p = fraction * 100.0;
    if p > 0.0 && p < 1.0 {
        "<1%".into()
    } else {
        format!("{:.0}%", p.round())
    }
}

/// A change in size: `+1.23 GB`, `−340 MB`, `±0 B`.
pub fn bytes_delta(delta: i64) -> String {
    let sign = match delta.signum() {
        1 => "+",
        -1 => "−",
        _ => "±",
    };
    format!("{sign}{}", bytes(delta.unsigned_abs()))
}

/// A coarse span of time: `45 s`, `12 min`, `5 h`, `3 d`.
pub fn span(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3_600 => format!("{} min", secs / 60),
        3_600..86_400 => format!("{} h", secs / 3_600),
        _ => format!("{} d", secs / 86_400),
    }
}

/// A precise span, largest units first: `22 d 19 h 37 min`, `3 h 5 min`,
/// `45 s`.
pub fn span_long(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    if secs < 60 {
        return format!("{secs} s");
    }
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "min")]
        .into_iter()
        .skip_while(|(v, _)| *v == 0)
        .filter(|(v, _)| *v > 0)
        .map(|(v, u)| format!("{v} {u}"))
        .collect();
    parts.join(" ")
}

/// A refresh interval: `0.5 s`, `2 s`.
pub fn interval(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{} s", ms as f64 / 1000.0)
    }
}

/// A countdown: `4:59`, `1:02:03`.
pub fn countdown(secs: u64) -> String {
    let (h, m, s) = (secs / 3_600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_234), "1.23 KB");
        assert_eq!(bytes(12_345_678), "12.3 MB");
        assert_eq!(bytes(494_384_795_648), "494 GB");
        assert_eq!(bytes(999_999), "1.00 MB");
    }

    #[test]
    fn formats_rates() {
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(1_500_000.0), "1.50 MB/s");
        assert_eq!(rate(-5.0), "0 B/s");
    }

    #[test]
    fn compact_rates_have_a_fixed_width() {
        for (v, s) in [
            (0.0, "  0K"),
            (12_000.0, " 12K"),
            (1_200_000.0, "1.2M"),
            (120_000_000.0, "120M"),
            (2_500_000_000.0, "2.5G"),
        ] {
            assert_eq!(rate_compact(v), s);
            assert_eq!(rate_compact(v).chars().count(), 4);
            assert_eq!(rate_menu(v), format!("{s}B/s"));
        }
    }

    #[test]
    fn formats_link_speeds() {
        assert_eq!(bits_per_sec(1_000_000_000), "1 Gbps");
        assert_eq!(bits_per_sec(866_700_000), "867 Mbps");
        assert_eq!(bits_per_sec(2_500_000_000), "2.5 Gbps");
    }

    #[test]
    fn formats_percent() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.004), "<1%");
        assert_eq!(percent(0.426), "43%");
    }

    #[test]
    fn formats_deltas_and_spans() {
        assert_eq!(bytes_delta(1_234_000_000), "+1.23 GB");
        assert_eq!(bytes_delta(-340_000_000), "−340 MB");
        assert_eq!(bytes_delta(0), "±0 B");
        assert_eq!(span(45), "45 s");
        assert_eq!(span(720), "12 min");
        assert_eq!(span(5 * 3_600 + 10), "5 h");
        assert_eq!(span(3 * 86_400), "3 d");
        assert_eq!(span_long(45), "45 s");
        assert_eq!(span_long(3 * 3_600 + 5 * 60 + 9), "3 h 5 min");
        assert_eq!(
            span_long(22 * 86_400 + 19 * 3_600 + 37 * 60 + 47),
            "22 d 19 h 37 min"
        );
        assert_eq!(span_long(86_400 + 60), "1 d 1 min");
        assert_eq!(countdown(299), "4:59");
        assert_eq!(countdown(3_723), "1:02:03");
    }
}
