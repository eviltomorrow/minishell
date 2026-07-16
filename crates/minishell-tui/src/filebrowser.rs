use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use minishell_core::Machine;
use minishell_ssh::sftp::{self, FileEntry, format_modified};
use minishell_ssh::ConnectConfig;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Local,
    Remote,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }
}

struct PanelState {
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
}

impl PanelState {
    fn new(path: PathBuf) -> Self {
        PanelState {
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            current_path: path,
        }
    }
}

enum ActionResult {
    TransferDone,
    Error(String),
}

pub struct FileBrowserState {
    machine: Machine,
    local: PanelState,
    remote: PanelState,
    active_side: Side,
    session: Option<ssh2::Session>,
    status: String,
    pending: Option<Receiver<ActionResult>>,
    confirm_delete: Option<(Side, usize)>,
    rename_input: Option<String>,
}

impl FileBrowserState {
    pub fn new(machine: Machine) -> Self {
        let local_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        FileBrowserState {
            local: PanelState::new(local_path),
            remote: PanelState::new(PathBuf::from("/")),
            active_side: Side::Remote,
            machine,
            session: None,
            status: "Connecting...".to_string(),
            pending: None,
            confirm_delete: None,
            rename_input: None,
        }
    }

    pub fn connect(&mut self) -> Result<(), String> {
        let config = self.build_config();
        let session = minishell_ssh::create_session(&config)
            .map_err(|e| format!("SSH connection failed: {}", e))?;
        self.session = Some(session);
        self.status = "Connected".to_string();
        Ok(())
    }

    fn build_config(&self) -> ConnectConfig {
        let host = self.machine.effective_host().to_string();
        ConnectConfig {
            username: self.machine.username.clone(),
            password: if self.machine.password == "-" {
                String::new()
            } else {
                self.machine.password.clone()
            },
            private_key_path: if self.machine.private_key_path == "-" {
                String::new()
            } else {
                self.machine.private_key_path.clone()
            },
            host,
            port: self.machine.port,
            timeout: std::time::Duration::from_secs(10),
            device: self.machine.device.clone(),
        }
    }

    fn active_panel(&self) -> &PanelState {
        match self.active_side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        }
    }

    fn active_panel_mut(&mut self) -> &mut PanelState {
        match self.active_side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        }
    }

    pub fn init_dirs(&mut self) {
        self.refresh_local();
        if self.session.is_some() {
            self.refresh_remote();
        }
    }

    pub fn check_pending(&mut self) {
        if let Some(ref rx) = self.pending {
            match rx.try_recv() {
                Ok(ActionResult::TransferDone) => {
                    self.pending = None;
                    self.status = "Transfer complete".to_string();
                    self.refresh_panel(Side::Remote);
                    self.refresh_panel(Side::Local);
                }
                Ok(ActionResult::Error(e)) => {
                    self.pending = None;
                    self.status = format!("Error: {}", e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Transfer failed".to_string();
                }
            }
        }
    }

    fn refresh_local(&mut self) {
        let path = self.local.current_path.clone();
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let meta = entry.metadata().ok();
                let modified = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| format_modified(Some(d.as_secs())))
                    .unwrap_or_default();
                entries.push(FileEntry {
                    name,
                    is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified,
                });
            }
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
        self.local.entries = entries;
        self.local.cursor = self.local.cursor.min(self.local.entries.len().saturating_sub(1));
        self.local.scroll_offset = 0;
    }

    fn refresh_remote(&mut self) {
        let session = match self.session.as_ref() {
            Some(s) => s,
            None => {
                self.status = "Not connected".to_string();
                return;
            }
        };
        let sftp = match session.sftp() {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("SFTP error: {}", e);
                return;
            }
        };
        let path = self.remote.current_path.to_string_lossy().to_string();
        match sftp::list_dir(&sftp, &path) {
            Ok(entries) => {
                self.remote.entries = entries;
                self.remote.cursor = self.remote
                    .cursor
                    .min(self.remote.entries.len().saturating_sub(1));
                self.remote.scroll_offset = 0;
                self.status = format!("{} entries", self.remote.entries.len());
            }
            Err(e) => {
                self.status = format!("Error: {}", e);
            }
        }
    }

    fn refresh_panel(&mut self, side: Side) {
        match side {
            Side::Local => self.refresh_local(),
            Side::Remote => self.refresh_remote(),
        }
    }

    pub fn toggle_side(&mut self) {
        self.active_side = self.active_side.other();
    }

    fn move_cursor(&mut self, delta: isize) {
        let panel = self.active_panel_mut();
        let len = panel.entries.len();
        if len == 0 {
            return;
        }
        let new = (panel.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        panel.cursor = new;
        if new < panel.scroll_offset {
            panel.scroll_offset = new;
        }
    }

    fn enter_dir(&mut self) {
        let new_path = {
            let p = self.active_panel();
            if p.entries.is_empty() {
                return;
            }
            let entry = &p.entries[p.cursor];
            if !entry.is_dir {
                return;
            }
            p.current_path.join(&entry.name)
        };
        {
            let p = self.active_panel_mut();
            p.current_path = new_path;
            p.cursor = 0;
            p.scroll_offset = 0;
        }
        self.refresh_panel(self.active_side);
    }

    fn parent_dir(&mut self) {
        let parent = {
            let p = self.active_panel();
            if p.current_path.parent().map_or(true, |p| p.as_os_str().is_empty()) {
                return;
            }
            p.current_path.parent().map(|p| p.to_path_buf())
        };
        if let Some(path) = parent {
            {
                let p = self.active_panel_mut();
                p.current_path = path;
                p.cursor = 0;
                p.scroll_offset = 0;
            }
            self.refresh_panel(self.active_side);
        }
    }

    fn upload_selected(&mut self) {
        if self.active_side != Side::Local {
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let entry = self.local.entries.get(self.local.cursor).cloned();
        let entry = match entry {
            Some(e) if !e.is_dir => e,
            _ => {
                self.status = "Select a file to upload".to_string();
                return;
            }
        };

        let local_path = self.local.current_path.join(&entry.name);
        let remote_path = self.remote.current_path.join(&entry.name);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Uploading {}...", entry.name);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);

        thread::spawn(move || {
            let session = match minishell_ssh::create_session(&config) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("Connection failed: {}", e)));
                    return;
                }
            };
            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("SFTP init failed: {}", e)));
                    return;
                }
            };
            match sftp::upload_file(&sftp, &local_path, &remote_str) {
                Ok(()) => { let _ = tx.send(ActionResult::TransferDone); }
                Err(e) => { let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e))); }
            }
        });
    }

    fn download_selected(&mut self) {
        if self.active_side != Side::Remote {
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let entry = self.remote.entries.get(self.remote.cursor).cloned();
        let entry = match entry {
            Some(e) if !e.is_dir => e,
            _ => {
                self.status = "Select a file to download".to_string();
                return;
            }
        };

        let remote_path = self.remote.current_path.join(&entry.name);
        let local_path = self.local.current_path.join(&entry.name);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Downloading {}...", entry.name);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);

        thread::spawn(move || {
            let session = match minishell_ssh::create_session(&config) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("Connection failed: {}", e)));
                    return;
                }
            };
            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("SFTP init failed: {}", e)));
                    return;
                }
            };
            match sftp::download_file(&sftp, &remote_str, &local_path) {
                Ok(()) => { let _ = tx.send(ActionResult::TransferDone); }
                Err(e) => { let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e))); }
            }
        });
    }

    fn start_delete(&mut self) {
        let side = self.active_side;
        let (entry_name, cursor) = {
            let p = self.active_panel();
            let cursor = p.cursor;
            match p.entries.get(cursor).cloned() {
                Some(e) => (e.name.clone(), cursor),
                None => return,
            }
        };
        self.status = format!("Delete {}?", entry_name);
        self.confirm_delete = Some((side, cursor));
    }

    fn confirm_delete_action(&mut self) {
        let (side, idx) = match self.confirm_delete.take() {
            Some(v) => v,
            None => return,
        };

        let (path_str, entry_name) = {
            let panel = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            let entry = match panel.entries.get(idx) {
                Some(e) => e.clone(),
                None => return,
            };
            let p = panel.current_path.join(&entry.name);
            (p.to_string_lossy().to_string(), entry.name.clone())
        };

        let result = if side == Side::Local {
            let path = std::path::Path::new(&path_str);
            if path.is_dir() {
                std::fs::remove_dir(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())
            }
        } else {
            let session = match self.session.as_ref() {
                Some(s) => s,
                None => {
                    self.status = "Not connected".to_string();
                    return;
                }
            };
            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    self.status = format!("SFTP error: {}", e);
                    return;
                }
            };
            sftp::remove_file(&sftp, &path_str).or_else(|_| sftp::remove_dir(&sftp, &path_str))
                .map_err(|e| e.to_string())
        };

        match result {
            Ok(()) => {
                self.status = format!("Deleted {}", entry_name);
                self.refresh_panel(side);
            }
            Err(e) => {
                self.status = format!("Delete failed: {}", e);
            }
        }
    }

    fn start_rename(&mut self) {
        let entry_name = {
            let p = self.active_panel();
            match p.entries.get(p.cursor) {
                Some(e) => e.clone(),
                None => return,
            }
        };
        self.rename_input = Some(entry_name.name.clone());
        self.status = "Enter new name:".to_string();
    }

    fn confirm_rename(&mut self) {
        let new_name = match self.rename_input.take() {
            Some(n) if !n.is_empty() => n,
            _ => {
                self.status = "Rename cancelled".to_string();
                return;
            }
        };
        let side = self.active_side;
        let (old_path, new_path) = {
            let panel = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            let old_entry = match panel.entries.get(panel.cursor) {
                Some(e) => e.clone(),
                None => return,
            };
            (
                panel.current_path.join(&old_entry.name),
                panel.current_path.join(&new_name),
            )
        };

        let result = match side {
            Side::Local => std::fs::rename(&old_path, &new_path)
                .map_err(|e| e.to_string()),
            Side::Remote => {
                let session = match self.session.as_ref() {
                    Some(s) => s,
                    None => {
                        self.status = "Not connected".to_string();
                        return;
                    }
                };
                let sftp = match session.sftp() {
                    Ok(s) => s,
                    Err(e) => {
                        self.status = format!("SFTP error: {}", e);
                        return;
                    }
                };
                sftp::rename_item(
                    &sftp,
                    &old_path.to_string_lossy(),
                    &new_path.to_string_lossy(),
                )
                .map_err(|e| e.to_string())
            }
        };

        match result {
            Ok(()) => {
                self.status = format!("Renamed to {}", new_name);
                self.refresh_panel(side);
            }
            Err(e) => {
                self.status = format!("Rename failed: {}", e);
            }
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        if self.rename_input.is_some() {
            match key.code {
                KeyCode::Enter => self.confirm_rename(),
                KeyCode::Esc => {
                    self.rename_input = None;
                    self.status = "Rename cancelled".to_string();
                }
                KeyCode::Backspace => {
                    if let Some(ref mut s) = self.rename_input {
                        s.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut s) = self.rename_input {
                        s.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        if self.confirm_delete.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_delete_action(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm_delete = None;
                    self.status = "Delete cancelled".to_string();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => self.enter_dir(),
            KeyCode::Backspace | KeyCode::Char('h') => self.parent_dir(),
            KeyCode::Tab => self.toggle_side(),
            KeyCode::Char('u') => self.upload_selected(),
            KeyCode::Char('d') => self.download_selected(),
            KeyCode::Char('x') => self.start_delete(),
            KeyCode::Char('r') => self.start_rename(),
            _ => {}
        }
    }

    pub fn wants_quit(&self, key: &crossterm::event::KeyEvent) -> bool {
        matches!(key.code, crossterm::event::KeyCode::Char('q'))
            && self.rename_input.is_none()
            && self.confirm_delete.is_none()
    }

    pub fn render(&self, f: &mut Frame) {
        let area = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)])
            .split(area);

        // Header
        let host = self.machine.effective_host();
        let header = format!(
            " {}@{}:{}   {}",
            self.machine.username,
            host,
            self.machine.port,
            self.remote.current_path.display()
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &header,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))),
            chunks[0],
        );

        // Split panels
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);

        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        // Status bar
        let (status_color, prefix) = if self.pending.is_some() {
            (Color::Yellow, " ⏳ ")
        } else if self.status.starts_with("Error")
            || self.status.starts_with("Delete failed")
            || self.status.starts_with("Upload failed")
            || self.status.starts_with("Download failed")
        {
            (Color::Red, " ✗ ")
        } else if self.status.starts_with("Transfer complete")
            || self.status.starts_with("Deleted")
            || self.status.starts_with("Renamed")
        {
            (Color::Green, " ✓ ")
        } else {
            (Color::White, "   ")
        };

        let mut spans: Vec<Span> = vec![
            Span::styled(prefix, Style::default()),
            Span::styled(&self.status, Style::default().fg(status_color)),
        ];

        let help = if self.confirm_delete.is_some() {
            "  y:确认  n:取消".to_string()
        } else if self.rename_input.is_some() {
            format!(
                "  Enter:确认  Esc:取消  {}▌",
                self.rename_input.as_ref().unwrap()
            )
        } else {
            "  Tab:切栏  ↑↓:移动  Enter:进入  u:上传  d:下载  x:删除  r:重命名  q:退出  Backspace:上级".to_string()
        };
        let w = area.width as usize;
        let spans_width: usize = spans.iter().map(|s| s.width()).sum();
        if spans_width + help.len() + 4 < w {
            let padding = w - spans_width - help.len();
            spans.push(Span::raw(" ".repeat(padding)));
        }
        spans.push(Span::styled(&help, Style::default().fg(Color::Gray)));

        f.render_widget(Line::from(spans), chunks[2]);
    }

    fn render_panel(&self, f: &mut Frame, area: Rect, side: Side) {
        let panel = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        let is_active = self.active_side == side;
        let title = format!(
            " {} {}",
            if side == Side::Local { "LOCAL" } else { "REMOTE" },
            panel.current_path.display()
        );

        let border_style = if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Header row
        let header = Line::from(vec![
            Span::styled("  Name", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" ".repeat(inner.width.saturating_sub(22).max(1) as usize)),
            Span::styled("Size", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("Modified", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(
            header,
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        // Entries
        let visible = (inner.height.saturating_sub(1)) as usize;
        let selected_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
            .bg(Color::Blue);
        let dir_style = Style::default().fg(Color::Cyan);
        let file_style = Style::default().fg(Color::White);
        let dim_style = Style::default().fg(Color::DarkGray);

        for i in 0..visible {
            let idx = panel.scroll_offset + i;
            if idx >= panel.entries.len() {
                break;
            }
            let entry = &panel.entries[idx];
            let style = if idx == panel.cursor {
                selected_style
            } else if entry.is_dir {
                dir_style
            } else {
                file_style
            };

            let marker = if idx == panel.cursor && is_active {
                "▸"
            } else {
                " "
            };
            let icon = if entry.is_dir { "📁" } else { " " };
            let name = format!("{}{} {}", marker, icon, entry.name);

            let size_str = if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            };

            let y = inner.y + 1 + i as u16;
            let name_width = (inner.width as usize).saturating_sub(22).max(1);
            let display_name = truncate_to_width(&name, name_width);

            f.render_widget(
                Line::from(vec![
                    Span::styled(display_name, style),
                    Span::raw(" "),
                    Span::styled(
                        pad_left(&size_str, 10),
                        if idx == panel.cursor {
                            selected_style
                        } else {
                            dim_style
                        },
                    ),
                    Span::raw("  "),
                    Span::styled(
                        &entry.modified,
                        if idx == panel.cursor {
                            selected_style
                        } else {
                            dim_style
                        },
                    ),
                ]),
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }
}

fn format_size(size: u64) -> String {
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

fn pad_left(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", " ".repeat(width - w), s)
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
