use ratatui::style::Style;
use ratatui::text::Span;
pub use minishell_utils::{format_size, pad_left, pad_right, truncate_to_width};

pub const HEADER_FG: ratatui::style::Color = ratatui::style::Color::Cyan;
pub const ACTIVE_BORDER: ratatui::style::Color = ratatui::style::Color::Cyan;
pub const INACTIVE_BORDER: ratatui::style::Color = ratatui::style::Color::DarkGray;
pub const DIR_FG: ratatui::style::Color = ratatui::style::Color::Yellow;
pub const FILE_FG: ratatui::style::Color = ratatui::style::Color::White;
pub const SELECTED_BG: ratatui::style::Color = ratatui::style::Color::Blue;
pub const STATUS_OK: ratatui::style::Color = ratatui::style::Color::Green;
pub const STATUS_ERR: ratatui::style::Color = ratatui::style::Color::Red;
pub const DIM: ratatui::style::Color = ratatui::style::Color::DarkGray;
pub const HINT: ratatui::style::Color = ratatui::style::Color::Gray;

pub fn render_name_and_path<'a>(text: &'a str, name_style: Style, sep_style: Style, path_style: Style) -> Vec<Span<'a>> {
    const PIPE_SEP: char = '\u{2502}';
    match text.find(PIPE_SEP) {
        Some(p) => {
            let name = &text[..p];
            let path = &text[p + PIPE_SEP.len_utf8()..];
            vec![
                Span::styled(format!(" {} ", name), name_style),
                Span::styled(format!("{} ", PIPE_SEP), sep_style),
                Span::styled(path, path_style),
            ]
        }
        None => vec![Span::styled(format!(" {} ", text), name_style)],
    }
}

pub fn format_entry_size(size: u64, is_dir: bool) -> String {
    if is_dir {
        pad_left("", 8)
    } else {
        pad_left(&format_size(size), 8)
    }
}

pub fn truncate_name(name: &str, max_width: usize) -> String {
    truncate_to_width(name, max_width)
}
