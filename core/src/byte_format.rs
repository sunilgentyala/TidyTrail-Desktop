/// Formats a byte count the way TidyTrail's mobile app does (binary units,
/// one decimal place above 1 KB), kept as a single source of truth instead
/// of letting the frontend re-derive its own rounding rules.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit_index = 0;
    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn formats_bytes_below_a_kilobyte() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn formats_kilobytes() {
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn formats_gigabytes() {
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
