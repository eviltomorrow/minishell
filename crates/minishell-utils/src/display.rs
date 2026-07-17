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

pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width {
            break;
        }
        result.push(c);
        current_width += cw;
    }
    result
}

pub fn pad_left(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

pub fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
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

    #[test]
    fn test_pad_left() {
        assert_eq!(pad_left("abc", 5), "  abc");
        assert_eq!(pad_left("abcde", 3), "abc");
        assert_eq!(pad_left("", 3), "   ");
    }

    #[test]
    fn test_pad_right() {
        assert_eq!(pad_right("abc", 5), "abc  ");
        assert_eq!(pad_right("abcde", 3), "abc");
        assert_eq!(pad_right("", 3), "   ");
    }

    #[test]
    fn test_truncate_to_width() {
        assert_eq!(truncate_to_width("hello", 3), "hel");
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("", 5), "");
    }
}
