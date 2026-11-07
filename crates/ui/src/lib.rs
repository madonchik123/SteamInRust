//! The design system. Pages and extensions should take colors and sizes from
//! [`Theme`] and build with [`widgets`], so a theme change restyles everything.

pub mod brand;
pub mod icons;
pub mod theme;
pub mod widgets;

pub use theme::{Theme, install_fonts};

use i18n::t;

/// Format minutes as "12.5 hrs" / "45 min".
pub fn playtime(minutes: u64) -> String {
    if minutes < 60 {
        t!("{n} min", n = minutes)
    } else {
        t!("{n} hrs", n = format!("{:.1}", minutes as f64 / 60.0))
    }
}

/// Format bytes as "12.3 GB".
pub fn bytes(n: u64) -> String {
    let units = [t!("B"), t!("KB"), t!("MB"), t!("GB"), t!("TB")];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} {}", units[0]) } else { format!("{v:.1} {}", units[i]) }
}

/// "3 days ago" style relative time from a unix timestamp.
pub fn ago(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = now.saturating_sub(unix);
    match s {
        0..60 => t!("just now").to_string(),
        60..3600 => t!("{n} min ago", n = s / 60),
        3600..86400 => t!("{n} hr ago", n = s / 3600),
        86400..2_592_000 => t!("{n} days ago", n = s / 86400),
        2_592_000..31_536_000 => t!("{n} months ago", n = s / 2_592_000),
        _ => t!("{n} years ago", n = s / 31_536_000),
    }
}

/// A unix time as a short date, e.g. "7 Oct 2026" (UTC, month names translated).
pub fn date(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let months = [t!("Jan"), t!("Feb"), t!("Mar"), t!("Apr"), t!("May"), t!("Jun"), t!("Jul"), t!("Aug"), t!("Sep"), t!("Oct"), t!("Nov"), t!("Dec")];
    format!("{d} {} {y}", months[(m - 1) as usize])
}
