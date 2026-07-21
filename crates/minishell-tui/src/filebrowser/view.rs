use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use minishell_utils::{format_size, pad_left, pad_right, truncate_to_width};

use super::types::Side;
use super::render::{render_name_and_path, ACTIVE_BORDER, HEADER_FG, INACTIVE_BORDER, DIR_FG, FILE_FG, SELECTED_BG, STATUS_OK, STATUS_ERR, DIM, HINT};
use super::FileBrowserState;
use crate::styles;

impl FileBrowserState {
    pub fn render(&mut self, f: &mut Frame) {
        let area = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(2),
                Constraint::Length(1),
            ])
            .split(area);

        let header_lines = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[0]);

        let host = self.machine.effective_host();
        let path = self.active_panel().current_path.clone();
        let breadcrumb = path.components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(" > ");

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled("文件浏览器", styles::header_style()),
                Span::styled(format!("  {}@{}:{}", self.machine.username, host, self.machine.port), styles::help_style()),
                Span::styled(format!("  {}", breadcrumb), Style::default().fg(Color::DarkGray)),
            ])),
            header_lines[0],
        );

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(area.width as usize),
                styles::separator_style(),
            ))),
            header_lines[1],
        );

        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);

        self.visible_rows = (chunks[1].height as usize).saturating_sub(4).max(1);
        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        if self.clipboard_panel_open {
            let panel_area = match self.clipboard_side.unwrap_or(self.active_side) {
                Side::Local => panels[0],
                Side::Remote => panels[1],
            };
            self.render_clipboard_panel(f, panel_area);
        }

        self.render_status_bar(f, area, chunks[2]);
    }

    fn render_status_bar(&mut self, f: &mut Frame, area: Rect, status_area: Rect) {
        let is_transferring = self.pending.is_some();
        let (status_color, prefix) = if is_transferring {
            (Color::Yellow, "\u{25B6}")
        } else if self.status.starts_with("错误")
            || self.status.starts_with("删除失败")
            || self.status.starts_with("上传失败")
            || self.status.starts_with("下载失败")
        {
            (STATUS_ERR, "\u{2717}")
        } else if self.status.starts_with("传输完成")
            || self.status.starts_with("已删除")
            || self.status.starts_with("已重命名")
        {
            (STATUS_OK, "\u{2713}")
        } else {
            (HINT, " ")
        };

        let status_text = if self.rename_input.is_some() {
            if let Some(ref input) = self.rename_input {
                format!("{} {}", self.status, input)
            } else {
                self.status.clone()
            }
        } else {
            self.status.clone()
        };

        let mut spans: Vec<Span> = vec![
            Span::styled(prefix, Style::default()),
        ];

        if self.pending.is_some() && self.progress_total > 0 {
            let pct = (self.progress_current * 100 / self.progress_total) as usize;
            let bar_width = 20;
            let filled = pct * bar_width / 100;
            let empty = bar_width - filled;
            spans.push(Span::styled(&self.progress_file_name, Style::default().fg(Color::Yellow)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled("\u{2588}".repeat(filled), Style::default().fg(Color::Cyan)));
            spans.push(Span::styled("\u{2591}".repeat(empty), Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(format!(" {}%", pct), Style::default().fg(Color::Yellow)));
        } else if self.confirm_delete.is_some() {
            let parts: Vec<&str> = status_text.split('|').collect();
            if parts.len() >= 4 {
                spans.push(Span::styled(
                    format!("{} ", parts[0]),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    parts[1],
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
                let name_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
                let sep_style = Style::default().fg(Color::DarkGray);
                let path_style = Style::default().fg(Color::Cyan);
                let mut name_spans = render_name_and_path(parts[2], name_style, sep_style, path_style);
                if let Some(last) = name_spans.last_mut() {
                    let content = last.content.to_mut();
                    content.push('?');
                }
                spans.extend(name_spans);
                spans.push(Span::styled(
                    format!(" ({})", parts[3]),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled(&status_text, Style::default().fg(Color::Red)));
            }
        } else if self.transfer_confirm.is_some() {
            let parts: Vec<&str> = status_text.split('|').collect();
            if parts.len() >= 5 {
                spans.push(Span::styled(
                    format!("{} ", parts[0]),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    parts[1],
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
                let name_spans = render_name_and_path(
                    parts[2],
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                );
                spans.extend(name_spans);
                spans.push(Span::styled(
                    format!(" {} ", parts[3]),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{}?", parts[4]),
                    Style::default().fg(Color::Cyan),
                ));
            } else {
                spans.push(Span::styled(&status_text, Style::default().fg(Color::Yellow)));
            }
        } else if self.switch_confirm.is_some() {
            spans.push(Span::styled(&status_text, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(&status_text, Style::default().fg(status_color)));
        }

        spans.push(Span::styled(" │ ", styles::status_sep_style()));

        let right_spans: Vec<Span> = self.build_help_hints();

        let w = area.width as usize;
        let right_edge_gap = 2usize;
        let left_width: usize = spans.iter().map(|s| s.width()).sum();
        let right_width: usize = right_spans.iter().map(|s| s.width()).sum();
        if left_width + right_width + right_edge_gap < w {
            let padding = w - left_width - right_width - right_edge_gap;
            spans.push(Span::raw(" ".repeat(padding)));
        }
        spans.extend(right_spans);

        f.render_widget(
            Line::from(spans).style(Style::default().bg(Color::Reset)),
            status_area,
        );

        if self.rename_input.is_some() {
            let prefix_width: usize = 3;
            let status_width = UnicodeWidthStr::width(status_text.as_str());
            let cx = status_area.x + prefix_width as u16 + status_width as u16;
            f.set_cursor_position((cx, status_area.y));
        }
    }

    fn build_help_hints(&self) -> Vec<Span<'static>> {
        if self.confirm_delete.is_some() {
            vec![
                Span::styled("[Y]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled("es  ", styles::help_style()),
                Span::styled("[N]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled("o", styles::help_style()),
            ]
        } else if self.transfer_confirm.is_some() || self.switch_confirm.is_some() {
            vec![
                Span::styled("[Y]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("es  ", styles::help_style()),
                Span::styled("[N]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled("o", styles::help_style()),
            ]
        } else if self.rename_input.is_some() {
            vec![
                Span::styled("Enter", styles::key_style()),
                Span::styled(":ok  ", styles::help_style()),
                Span::styled("Esc", styles::key_style()),
                Span::styled(":cancel", styles::help_style()),
            ]
        } else if self.pending.is_some() {
            vec![Span::styled("(transferring...)", styles::help_style())]
        } else if self.clipboard_panel_open {
            vec![
                Span::styled("n", styles::key_style()),
                Span::styled(":next  ", styles::help_style()),
                Span::styled("k", styles::key_style()),
                Span::styled(":remove  ", styles::help_style()),
                Span::styled("p", styles::key_style()),
                Span::styled(":paste  ", styles::help_style()),
                Span::styled("v", styles::key_style()),
                Span::styled(":close", styles::help_style()),
            ]
        } else if !self.active_panel().expanded_dirs.is_empty() {
            let mut hints = vec![
                Span::styled("Tab", styles::key_style()),
                Span::styled(":panel  ", styles::help_style()),
                Span::styled("\u{2191}\u{2193}", styles::key_style()),
                Span::styled(":move  ", styles::help_style()),
                Span::styled("\u{2192}", styles::key_style()),
                Span::styled(":enter  ", styles::help_style()),
                Span::styled("\u{2190}", styles::key_style()),
                Span::styled(":back  ", styles::help_style()),
                Span::styled("y", styles::key_style()),
                Span::styled(":copy  ", styles::help_style()),
                Span::styled("v", styles::key_style()),
                Span::styled(":view  ", styles::help_style()),
            ];
            if self.clipboard.is_empty() {
                hints.push(Span::styled("x", styles::key_style()));
                hints.push(Span::styled(":transfer  ", styles::help_style()));
            }
            hints.extend([
                Span::styled("d", styles::key_style()),
                Span::styled(":del  ", styles::help_style()),
                Span::styled("r", styles::key_style()),
                Span::styled(":rename  ", styles::help_style()),
                Span::styled("t", styles::key_style()),
                Span::styled(":tree  ", styles::help_style()),
                Span::styled("q", styles::key_style()),
                Span::styled(":quit", styles::help_style()),
            ]);
            hints
        } else {
            let mut hints = vec![
                Span::styled("Tab", styles::key_style()),
                Span::styled(":panel  ", styles::help_style()),
                Span::styled("\u{2191}\u{2193}", styles::key_style()),
                Span::styled(":move  ", styles::help_style()),
                Span::styled("\u{2192}", styles::key_style()),
                Span::styled(":enter  ", styles::help_style()),
                Span::styled("\u{2190}", styles::key_style()),
                Span::styled(":back  ", styles::help_style()),
                Span::styled("y", styles::key_style()),
                Span::styled(":copy  ", styles::help_style()),
                Span::styled("v", styles::key_style()),
                Span::styled(":view  ", styles::help_style()),
                Span::styled("p", styles::key_style()),
                Span::styled(":paste  ", styles::help_style()),
            ];
            if self.clipboard.is_empty() {
                hints.push(Span::styled("x", styles::key_style()));
                hints.push(Span::styled(":transfer  ", styles::help_style()));
            }
            hints.extend([
                Span::styled("d", styles::key_style()),
                Span::styled(":del  ", styles::help_style()),
                Span::styled("r", styles::key_style()),
                Span::styled(":rename  ", styles::help_style()),
                Span::styled("t", styles::key_style()),
                Span::styled(":tree  ", styles::help_style()),
                Span::styled("q", styles::key_style()),
                Span::styled(":quit", styles::help_style()),
            ]);
            hints
        }
    }

    fn render_panel(&self, f: &mut Frame, area: Rect, side: Side) {
        let panel = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        let is_active = self.active_side == side && self.pending.is_none();

        let border_style = if is_active {
            Style::default().fg(ACTIVE_BORDER)
        } else {
            Style::default().fg(INACTIVE_BORDER)
        };

        let title_style = if is_active {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };

        let label = format!(" {} ", side.label());
        let title = Line::from(vec![
            Span::styled(label, title_style),
            Span::styled(
                panel.current_path.display().to_string(),
                if is_active {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(DIM)
                },
            ),
        ]);

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(if is_active {
                ratatui::widgets::BorderType::Plain
            } else {
                ratatui::widgets::BorderType::Rounded
            });

        let inner = block.inner(area);
        f.render_widget(block, area);

        let perm_width: usize = 10;
        let size_width: usize = 8;
        let modified_width: usize = 16;
        let gap: usize = 2;
        let overhead: usize = 1 + perm_width + 1 + size_width + gap + modified_width;
        let name_area = (inner.width as usize).saturating_sub(overhead).max(6);

        let header = Line::from(vec![
            Span::styled(
                pad_right("Name", name_area),
                Style::default().fg(HEADER_FG).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                pad_left("Perm", perm_width),
                Style::default().fg(HEADER_FG).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                pad_left("Size", size_width),
                Style::default().fg(HEADER_FG).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                pad_left("Modified", modified_width),
                Style::default().fg(HEADER_FG).add_modifier(Modifier::BOLD),
            ),
        ]);
        f.render_widget(
            header,
            Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 },
        );

        f.render_widget(
            Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(DIM),
            )),
            Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 },
        );

        if panel.tree_entries.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  (empty)",
                    Style::default().fg(DIM),
                ))),
                Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: 1 },
            );
            return;
        }

        let visible = (inner.height.saturating_sub(2)) as usize;
        let selected_active_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
            .bg(SELECTED_BG);
        let selected_inactive_style = Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(70, 70, 90));
        let selected_style = if is_active { selected_active_style } else { selected_inactive_style };
        let dir_style = Style::default().fg(DIR_FG);
        let file_style = Style::default().fg(FILE_FG);
        let dim_style = Style::default().fg(DIM);

        let marker_active = if is_active { "\u{25B8}" } else { " " };

        for i in 0..visible {
            let idx = panel.scroll_offset + i;
            if idx >= panel.tree_entries.len() {
                break;
            }
            let te = &panel.tree_entries[idx];
            let entry = &te.entry;
            let display_depth = te.depth;
            let is_selected = idx == panel.cursor;
            let in_clipboard = self.is_effectively_selected(&entry.name, side, idx);
            let style = if is_selected {
                selected_style
            } else if in_clipboard {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else if entry.is_dir {
                dir_style
            } else {
                file_style
            };

            let marker = if is_selected { marker_active } else { " " };
            let icon = if in_clipboard {
                "[\u{2713}]"
            } else if entry.is_dir {
                "\u{1F4C1}"
            } else {
                "\u{1F4C4}"
            };
            let indent = "  ".repeat(display_depth);
            let short_name = entry.name.rsplit('/').next().unwrap_or(&entry.name);
            let name = format!("{}{}{}{}", marker, indent, icon, short_name);

            let display_name = truncate_to_width(&name, name_area);
            let display_name_padded = pad_right(&display_name, name_area);

            let perm_str = pad_left(&entry.perm, perm_width);

            let size_str = if entry.is_dir {
                pad_left("", size_width)
            } else {
                pad_left(&format_size(entry.size), size_width)
            };

            let modified_str = pad_left(&entry.modified, modified_width);

            let y = inner.y + 2 + i as u16;

            f.render_widget(
                Line::from(vec![
                    Span::styled(display_name_padded, style),
                    Span::raw(" "),
                    Span::styled(&perm_str, if is_selected { selected_style } else { dim_style }),
                    Span::raw(" "),
                    Span::styled(&size_str, if is_selected { selected_style } else { dim_style }),
                    Span::raw("  "),
                    Span::styled(&modified_str, if is_selected { selected_style } else { dim_style }),
                ]),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
        }
    }

    fn render_clipboard_panel(&self, f: &mut Frame, area: Rect) {
        let count = self.clipboard.len();

        let panel_width = 42u16.min(area.width.saturating_sub(2));
        let panel_height = (count as u16 + 4).min(area.height).max(5);

        let x = area.x + area.width.saturating_sub(panel_width);
        let y = area.y + area.height.saturating_sub(panel_height);

        let panel_area = Rect { x, y, width: panel_width, height: panel_height };

        let block = Block::default()
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled("\u{1F4CB}", Style::default()),
                Span::styled(
                    format!(" Selection ({}) ", count),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(panel_area);
        f.render_widget(block, panel_area);

        let col_name_w = inner.width.saturating_sub(7) as usize;

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("  {:<width$}", "Name", width = col_name_w),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Side",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                ),
            ])),
            Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 },
        );

        let mut line_spans = vec![];
        line_spans.push(Span::styled(
            format!("{}\u{2500}{}\u{2500}{}\u{2500}{}",
                "\u{2500}".repeat(1),
                "\u{2500}".repeat(col_name_w.saturating_sub(1)),
                "\u{2500}".repeat(1),
                "\u{2500}".repeat(3),
            ),
            Style::default().fg(Color::Rgb(60, 60, 90)),
        ));
        f.render_widget(
            Paragraph::new(Line::from(line_spans)),
            Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 },
        );

        let list_start_y = inner.y + 2;
        let visible_count = (inner.height as usize).saturating_sub(2);
        let scroll = self.clipboard_panel_cursor
            .saturating_sub(visible_count.saturating_sub(1))
            .max(0);

        for i in 0..visible_count {
            let idx = scroll + i;
            if idx >= self.clipboard.len() {
                break;
            }
            let entry = &self.clipboard[idx];
            let is_cursor = idx == self.clipboard_panel_cursor;

            let icon = if entry.is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };
            let side_label = match entry.source_side {
                Side::Local => "L",
                Side::Remote => "R",
            };

            let y_pos = list_start_y + i as u16;

            if is_cursor {
                f.render_widget(
                    Block::default().style(Style::default().bg(Color::Rgb(30, 50, 70))),
                    Rect { x: inner.x, y: y_pos, width: inner.width, height: 1 },
                );
            }

            let cursor_mark = if is_cursor { "\u{25B8}" } else { " " };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    cursor_mark,
                    if is_cursor { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) },
                ))),
                Rect { x: inner.x, y: y_pos, width: 1, height: 1 },
            );

            f.render_widget(
                Paragraph::new(Line::from(Span::styled(icon, Style::default()))),
                Rect { x: inner.x + 1, y: y_pos, width: 2, height: 1 },
            );

            let name_display = truncate_to_width(&entry.name, col_name_w);
            let name_padded = pad_right(&name_display, col_name_w);
            let name_style = if is_cursor {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(name_padded, name_style))),
                Rect { x: inner.x + 3, y: y_pos, width: col_name_w as u16, height: 1 },
            );

            let side_text = format!("[{}]", side_label);
            let side_style = if is_cursor {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(side_text, side_style))),
                Rect {
                    x: inner.x + inner.width.saturating_sub(4),
                    y: y_pos,
                    width: 4,
                    height: 1,
                },
            );
        }
    }
}
