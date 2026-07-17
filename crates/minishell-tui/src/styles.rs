use ratatui::style::{Color, Modifier, Style};

pub fn header_style() -> Style {
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
}

pub fn main_border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn help_style() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn search_style() -> Style {
    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
}

pub fn form_box_style() -> Style {
    Style::default().fg(Color::Cyan)
}

pub fn delete_box_style() -> Style {
    Style::default().fg(Color::Red)
}

pub fn form_field_style() -> Style {
    Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD)
}

pub fn separator_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn key_style() -> Style {
    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
}

pub fn status_style() -> Style {
    Style::default().fg(Color::White)
}

pub fn status_label_style() -> Style {
    Style::default().fg(Color::Cyan)
}

pub fn status_sep_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn status_key_style() -> Style {
    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
}

pub fn status_desc_style() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn selected_style() -> Style {
    Style::default().fg(Color::White).add_modifier(Modifier::BOLD).bg(Color::Blue)
}

pub fn zebra_row_style() -> Style {
    Style::default().bg(Color::Rgb(30, 30, 40))
}
