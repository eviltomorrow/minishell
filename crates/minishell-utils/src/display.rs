use unicode_width::UnicodeWidthStr;

pub fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut s = size as f64;
    let mut unit_idx = 0;
    while s >= 1024.0 && unit_idx < UNITS.len() - 1 {
        s /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", size)
    } else if s >= 100.0 {
        format!("{:.0} {}", s, UNITS[unit_idx])
    } else if s >= 10.0 {
        format!("{:.1} {}", s, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", s, UNITS[unit_idx])
    }
}

pub fn pad_left(s: &str, width: usize) -> String {
    todo!()
}

pub fn pad_right(s: &str, width: usize) -> String {
    todo!()
}

pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kb() {
        assert_eq!(format_size(1024), "1.00 K");
        assert_eq!(format_size(1536), "1.50 K");
    }

    #[test]
    fn test_format_size_mb() {
        assert_eq!(format_size(1048576), "1.00 M");
    }

    #[test]
    fn test_format_size_gb() {
        assert_eq!(format_size(1073741824), "1.00 G");
    }
}
