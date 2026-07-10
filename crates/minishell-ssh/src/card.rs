use unicode_width::UnicodeWidthStr;

const CARD_LINE_COLOR: &str = "\x1b[37m";
const CARD_LABEL: &str = "\x1b[1;37m";
const CARD_GREEN: &str = "\x1b[32m";
const CARD_RED: &str = "\x1b[31m";
const CARD_FAINT: &str = "\x1b[2m";
const CARD_BOLD: &str = "\x1b[1m";
const CARD_BOLD_OFF: &str = "\x1b[22m";
const CARD_RESET: &str = "\x1b[0m";

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until 'm' (end of ANSI sequence)
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn terminal_width() -> usize {
    crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80).max(40)
}

pub fn card_line(width: usize, content: &str) -> String {
    let visible = UnicodeWidthStr::width(strip_ansi(content).as_str());
    let padding = if width > 2 && visible < width - 2 { width - 2 - visible } else { 0 };
    format!("{}│{}{}{}│{}", CARD_LINE_COLOR, CARD_RESET, content, " ".repeat(padding), CARD_LINE_COLOR)
}

pub fn card_border(width: usize, corner_left: &str, corner_right: &str) -> String {
    format!("{}{}{}{}{}", CARD_LINE_COLOR, corner_left, "─".repeat(width.saturating_sub(2)), corner_right, CARD_RESET)
}

pub fn connect_card_top(ip: &str, host: &str, _port: i32, username: &str, auth_method: &str, width: usize) -> String {
    let mut lines = Vec::new();
    lines.push(card_border(width, "┌", "┐"));
    lines.push(card_line(width, &format!("{}SSH CONNECT{}", CARD_BOLD, CARD_BOLD_OFF)));
    lines.push(card_line(width, ""));
    lines.push(card_line(width, &format!("{}User: {}{}{}", CARD_LABEL, CARD_RESET, username, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Host: {}{}{}", CARD_LABEL, CARD_RESET, host, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}IP:   {}{}{}", CARD_LABEL, CARD_RESET, ip, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Auth: {}{}{}", CARD_LABEL, CARD_RESET, auth_method, CARD_FAINT)));
    lines.push(card_line(width, ""));
    lines.push(card_border(width, "├", "┤"));
    lines.join("\n")
}

pub fn connect_card_status_line(content: &str, width: usize) -> String {
    card_line(width, content)
}

pub fn connect_card_bottom(width: usize) -> String {
    card_border(width, "└", "┘")
}

pub fn connect_success_line(duration: std::time::Duration) -> String {
    let ms = duration.as_millis();
    format!("{}  ✓ Connected in {}ms{}", CARD_GREEN, ms, CARD_RESET)
}

pub fn connect_fail_line(err: &str) -> String {
    format!("{}  ✗ Failed: {}{}", CARD_RED, err, CARD_RESET)
}

pub fn disconnect_card(host: &str, duration: std::time::Duration, ssh_err: Option<&str>, width: usize) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    let duration_str = format!("{}h {}m {}s", hours, mins, secs);

    let status = match ssh_err {
        Some(err) => format!("{}Error: {}{}", CARD_RED, err, CARD_RESET),
        None => format!("{}OK{}", CARD_GREEN, CARD_RESET),
    };

    let mut lines = Vec::new();
    lines.push(card_border(width, "┌", "┐"));
    lines.push(card_line(width, &format!("{}DISCONNECTED{}", CARD_BOLD, CARD_BOLD_OFF)));
    lines.push(card_line(width, &format!("{}Host:   {}{}{}", CARD_LABEL, CARD_RESET, host, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Duration: {}{}{}", CARD_LABEL, CARD_RESET, duration_str, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Status: {}", CARD_LABEL, status)));
    lines.push(card_border(width, "└", "┘"));
    lines.join("\n")
}
