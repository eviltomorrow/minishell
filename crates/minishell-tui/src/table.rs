use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::buffer::Buffer;
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
pub struct Column {
    pub title: String,
    pub width: usize,
}

pub struct MachineTable {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
    pub cursor: usize,
    pub width: u16,
    pub height: u16,
}

impl MachineTable {
    pub fn new(columns: Vec<Column>) -> Self {
        MachineTable {
            columns,
            rows: Vec::new(),
            cursor: 0,
            width: 0,
            height: 0,
        }
    }

    pub fn set_size(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
    }

    pub fn set_rows(&mut self, rows: Vec<Vec<String>>) {
        self.rows = rows;
        if self.cursor >= self.rows.len() && !self.rows.is_empty() {
            self.cursor = self.rows.len() - 1;
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, n: usize) {
        self.cursor = n.min(self.rows.len().saturating_sub(1));
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor < self.rows.len().saturating_sub(1) {
            self.cursor += 1;
        }
    }

    pub fn goto_top(&mut self) {
        self.cursor = 0;
    }

    pub fn goto_bottom(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = self.rows.len() - 1;
        }
    }

    pub fn rows_count(&self) -> usize {
        self.rows.len()
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, selected_style: Style, normal_style: Style) {
        let header_style = super::styles::header_style();

        // Header
        let mut header_spans = Vec::new();
        for col in &self.columns {
            header_spans.push(Span::styled(pad_right(&col.title, col.width), header_style));
        }
        let header = Line::from(header_spans);
        buf.set_line(area.x, area.y, &header, area.width);

        // Rows
        let visible_height = self.height.saturating_sub(1) as usize;
        let total_rows = self.rows.len();

        let top = if total_rows <= visible_height {
            0
        } else {
            (self.cursor as isize - visible_height as isize / 2).max(0) as usize
        };
        let top = top.min(total_rows.saturating_sub(visible_height));

        for i in 0..visible_height {
            let row_idx = top + i;
            if row_idx >= total_rows {
                break;
            }

            let y = area.y + 1 + i as u16;
            let style = if row_idx == self.cursor {
                selected_style
            } else {
                normal_style
            };

            let mut spans = Vec::new();
            for (j, col) in self.columns.iter().enumerate() {
                let text = self.rows[row_idx].get(j).map(|s| s.as_str()).unwrap_or("");
                spans.push(Span::styled(pad_right(text, col.width), style));
            }
            let line = Line::from(spans);
            buf.set_line(area.x, y, &line, area.width);
        }
    }
}

fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
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

pub fn format_machine_row(m: &minishell_core::Machine, show_secrets: bool) -> Vec<String> {
    let empty = "-".to_string();
    if show_secrets {
        vec![
            format!("{}", m.num),
            m.ip.clone(),
            "".to_string(),
            if m.nat_ip.is_empty() { empty.clone() } else { m.nat_ip.clone() },
            m.username.clone(),
            if m.password.is_empty() { empty.clone() } else { m.password.clone() },
            "".to_string(),
            if m.private_key_path.is_empty() { empty.clone() } else { m.private_key_path.clone() },
            "".to_string(),
            if m.device.is_empty() { empty.clone() } else { m.device.clone() },
            "".to_string(),
            if m.remark.is_empty() { empty.clone() } else { m.remark.clone() },
        ]
    } else {
        vec![
            format!("{}", m.num),
            m.ip.clone(),
            "".to_string(),
            if m.nat_ip.is_empty() { empty.clone() } else { m.nat_ip.clone() },
            m.username.clone(),
            if m.device.is_empty() { empty.clone() } else { m.device.clone() },
            if m.remark.is_empty() { empty.clone() } else { m.remark.clone() },
        ]
    }
}

pub fn default_columns() -> Vec<Column> {
    vec![
        Column { title: "#".into(),       width: 4  },
        Column { title: "IP".into(),      width: 15 },
        Column { title: "".into(),        width: 2  },
        Column { title: "NAT".into(),     width: 12 },
        Column { title: "User".into(),    width: 10 },
        Column { title: "Device".into(),  width: 10 },
        Column { title: "Remark".into(),  width: 20 },
    ]
}

pub fn secrets_columns() -> Vec<Column> {
    vec![
        Column { title: "#".into(),       width: 4  },
        Column { title: "IP".into(),      width: 15 },
        Column { title: "".into(),        width: 2  },
        Column { title: "NAT".into(),     width: 12 },
        Column { title: "User".into(),    width: 10 },
        Column { title: "Password".into(), width: 10 },
        Column { title: "".into(),        width: 1  },
        Column { title: "Key".into(),     width: 16 },
        Column { title: "".into(),        width: 1  },
        Column { title: "Device".into(),  width: 10 },
        Column { title: "".into(),        width: 1  },
        Column { title: "Remark".into(),  width: 16 },
    ]
}

const COL_GAP: usize = 2;

pub fn auto_column_widths(columns: &[Column], rows: &[Vec<String>]) -> Vec<Column> {
    columns.iter().enumerate().map(|(i, col)| {
        if col.title.is_empty() {
            return col.clone();
        }
        let title_w = UnicodeWidthStr::width(col.title.as_str());
        let max_data_w = rows.iter()
            .filter_map(|row| row.get(i))
            .map(|val| UnicodeWidthStr::width(val.as_str()))
            .max()
            .unwrap_or(0);
        let w = title_w.max(max_data_w).max(3) + COL_GAP;
        Column { title: col.title.clone(), width: w }
    }).collect()
}
