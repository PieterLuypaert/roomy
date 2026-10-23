use std::path::Path;

use crate::volumes;

const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];

pub const ACCESS_HINT: &str = if cfg!(target_os = "macos") {
    "Give your terminal Full Disk Access to look inside."
} else {
    "Run ruim as a user who may read it to look inside."
};

pub fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    value /= 1000.0;
    while value >= 999.5 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 99.95 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn items(n: u64) -> String {
    match n {
        1 => "1 item".to_string(),
        n => format!("{} items", count(n)),
    }
}

pub fn access(is_blocked: bool, denied: u64) -> Option<&'static str> {
    match (is_blocked, denied) {
        (_, 0) => None,
        (true, _) => Some("no access"),
        (false, _) => Some("partly hidden"),
    }
}

pub fn places(n: usize) -> String {
    match n {
        1 => "1 place".to_string(),
        n => format!("{} places", count(n as u64)),
    }
}

pub fn percent(share: f64) -> String {
    match share {
        s if s <= 0.0 => String::new(),
        s if s < 0.005 => "<1%".to_string(),
        s => format!("{:.0}%", s * 100.0),
    }
}

pub fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

pub fn path(path: &Path) -> String {
    let path = volumes::logical(path);
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_decimal_units_like_finder() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1000), "1.0 kB");
        assert_eq!(size(1_234_567), "1.2 MB");
        assert_eq!(size(99_960_000), "100 MB");
        assert_eq!(size(123_456_789), "123 MB");
        assert_eq!(size(999_400_000), "999 MB");
        assert_eq!(size(999_700_000), "1.0 GB");
        assert_eq!(size(999_999_999), "1.0 GB");
        assert_eq!(size(12_345_678_901), "12.3 GB");
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(count(7), "7");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(312_402), "312,402");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn one_item_is_not_plural() {
        assert_eq!(items(1), "1 item");
        assert_eq!(items(2_400), "2,400 items");
    }

    #[test]
    fn percents_hide_zero_and_flag_tiny_shares() {
        assert_eq!(percent(0.0), "");
        assert_eq!(percent(0.001), "<1%");
        assert_eq!(percent(0.294), "29%");
        assert_eq!(percent(1.0), "100%");
    }
}
