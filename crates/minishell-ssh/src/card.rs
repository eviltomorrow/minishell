use unicode_width::UnicodeWidthStr;

const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for next in chars.by_ref() {
                if next == 'm' { break; }
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

fn line(width: usize) -> String {
    format!("{}{}{}", DIM, "─".repeat(width), RESET)
}

fn info_line(width: usize, content: &str) -> String {
    let visible = UnicodeWidthStr::width(strip_ansi(content).as_str());
    let padding = width.saturating_sub(visible).saturating_sub(2);
    format!(" {}{}{}", content, " ".repeat(padding), RESET)
}

pub fn connect_card_top(_ip: &str, host: &str, _port: i32, username: &str, auth_method: &str, width: usize) -> (String, usize) {
    let title = format!("{}SSH CONNECT{}", BOLD, RESET);
    let info = format!("{}{}@{} ({})", DIM, username, host, auth_method);

    let mut lines = Vec::new();
    lines.push(line(width));
    lines.push(info_line(width, &title));
    lines.push(info_line(width, &info));
    lines.push(line(width));
    (lines.join("\n"), width)
}

pub fn connect_card_status_line(content: &str, width: usize) -> String {
    info_line(width, content)
}

pub fn connect_card_bottom(width: usize) -> String {
    line(width)
}

pub fn connect_success_line(duration: std::time::Duration, width: usize) -> String {
    let total_secs = duration.as_secs();
    let ms = duration.subsec_millis();
    let duration_str = if total_secs >= 86400 {
        format!("{}d {}h {}m {}s", total_secs / 86400, (total_secs % 86400) / 3600, (total_secs % 3600) / 60, total_secs % 60)
    } else if total_secs >= 3600 {
        format!("{}h {}m {}s", total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60)
    } else if total_secs >= 60 {
        format!("{}m {}s", total_secs / 60, total_secs % 60)
    } else if total_secs > 0 {
        format!("{}.{}s", total_secs, ms / 100)
    } else {
        format!("{}ms", duration.as_millis())
    };
    info_line(width, &format!("{}✓ Connected{} in {}{}{}", GREEN, DIM, RESET, duration_str, RESET))
}

pub fn connect_fail_line(err: &str, width: usize) -> String {
    info_line(width, &format!("{}✗ {}{}", RED, err, RESET))
}

pub fn disconnect_card(host: &str, duration: std::time::Duration, ssh_err: Option<&str>, width: usize) -> String {
    let total_secs = duration.as_secs();
    let duration_str = if total_secs >= 86400 {
        format!("{}d {}h {}m {}s", total_secs / 86400, (total_secs % 86400) / 3600, (total_secs % 3600) / 60, total_secs % 60)
    } else if total_secs >= 3600 {
        format!("{}h {}m {}s", total_secs / 3600, (total_secs % 3600) / 60, total_secs % 60)
    } else if total_secs >= 60 {
        format!("{}m {}s", total_secs / 60, total_secs % 60)
    } else {
        format!("{}s", total_secs)
    };

    let status = match ssh_err {
        Some(err) => format!("{}{}{}", RED, err, RESET),
        None => format!("{}OK{}", GREEN, RESET),
    };

    let title = format!("{}DISCONNECTED{}", BOLD, RESET);
    let info = format!("{}{} │ {} │ {}", DIM, host, duration_str, status);

    let mut lines = Vec::new();
    lines.push(line(width));
    lines.push(info_line(width, &title));
    lines.push(info_line(width, &info));
    lines.push(line(width));
    lines.join("\n")
}
