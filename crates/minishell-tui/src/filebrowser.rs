use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
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
use crate::styles;

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

    fn label(self) -> &'static str {
        match self {
            Side::Local => "LOCAL",
            Side::Remote => "REMOTE",
        }
    }
}

struct PanelState {
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
    prev_dir_name: Option<String>,
}

impl PanelState {
    fn new(path: PathBuf) -> Self {
        PanelState {
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            current_path: path,
            prev_dir_name: None,
        }
    }
}

enum ActionResult {
    TransferDone(Side),
    Error(String),
}

struct PendingTransfer {
    progress: Arc<Mutex<(u64, u64)>>,
    done_rx: Receiver<ActionResult>,
}

pub struct FileBrowserState {
    machine: Machine,
    local: PanelState,
    remote: PanelState,
    active_side: Side,
    session: Option<ssh2::Session>,
    status: String,
    pending: Option<PendingTransfer>,
    progress_current: u64,
    progress_total: u64,
    confirm_delete: Option<(Side, usize)>,
    rename_input: Option<String>,
}

const HEADER_FG: Color = Color::Cyan;
const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::DarkGray;
const DIR_FG: Color = Color::Yellow;
const FILE_FG: Color = Color::White;
const SELECTED_BG: Color = Color::Blue;
const STATUS_OK: Color = Color::Green;
const STATUS_ERR: Color = Color::Red;
const STATUS_BUSY: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;
const HINT: Color = Color::Gray;

impl FileBrowserState {
    pub fn new(machine: Machine) -> Self {
        let local_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        FileBrowserState {
            local: PanelState::new(local_path),
            remote: PanelState::new(PathBuf::from("/")),
            active_side: Side::Local,
            machine,
            session: None,
            status: "Connecting...".to_string(),
            pending: None,
            progress_current: 0,
            progress_total: 0,
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
        let p = self.active_panel();
        self.status = format!("{} entries", p.entries.len());
    }

    pub fn check_pending(&mut self) {
        let transfer = match self.pending.as_ref() {
            Some(t) => t,
            None => return,
        };

        // Update progress from shared state
        {
            let p = transfer.progress.lock().unwrap();
            self.progress_current = p.0;
            self.progress_total = p.1;
        }

        // Check completion
        match transfer.done_rx.try_recv() {
            Ok(ActionResult::TransferDone(side)) => {
                self.pending = None;
                self.progress_current = 0;
                self.progress_total = 0;
                self.status = "Transfer complete".to_string();
                self.refresh_panel(side);
            }
            Ok(ActionResult::Error(e)) => {
                self.pending = None;
                self.progress_current = 0;
                self.progress_total = 0;
                self.status = format!("Error: {}", e);
            }
            Err(TryRecvError::Empty) => {
                if self.progress_total > 0 {
                    let pct = self.progress_current * 100 / self.progress_total;
                    self.status = format!("Transferring... {}%", pct);
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.progress_current = 0;
                self.progress_total = 0;
                self.status = "Transfer failed".to_string();
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
        self.local.cursor = self.local
            .cursor
            .min(self.local.entries.len().saturating_sub(1));
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
        let p = self.active_panel();
        self.status = format!("{} entries", p.entries.len());
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
        let (new_path, dir_name) = {
            let p = self.active_panel();
            if p.entries.is_empty() {
                return;
            }
            let entry = &p.entries[p.cursor];
            if !entry.is_dir {
                return;
            }
            let dir_name = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
            (p.current_path.join(&entry.name), dir_name)
        };
        {
            let p = self.active_panel_mut();
            p.prev_dir_name = Some(dir_name);
            p.current_path = new_path;
            p.cursor = 0;
            p.scroll_offset = 0;
        }
        self.refresh_panel(self.active_side);
    }

    fn parent_dir(&mut self) {
        let (parent, prev_name) = {
            let p = self.active_panel();
            if p.current_path.parent().map_or(true, |p| p.as_os_str().is_empty()) {
                return;
            }
            let parent = p.current_path.parent().map(|p| p.to_path_buf());
            (parent, p.prev_dir_name.clone())
        };
        if let Some(path) = parent {
            {
                let p = self.active_panel_mut();
                p.current_path = path;
                p.cursor = 0;
                p.scroll_offset = 0;
            }
            self.refresh_panel(self.active_side);
            if let Some(ref name) = prev_name {
                let p = self.active_panel_mut();
                if let Some(pos) = p.entries.iter().position(|e| e.name == *name || e.name.ends_with(&format!("/{}", name))) {
                    p.cursor = pos;
                }
            }
        }
    }

    fn upload_selected(&mut self) {
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

        let total_size = entry.size;
        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        let local_path = self.local.current_path.join(filename);
        let remote_path = self.remote.current_path.join(filename);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Uploading {}...", filename);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Mutex::new((0u64, total_size)));
        let progress_clone = progress.clone();

        self.pending = Some(PendingTransfer {
            progress,
            done_rx: rx,
        });
        self.progress_current = 0;
        self.progress_total = total_size;

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
            let cb = |cur, total| {
                let mut p = progress_clone.lock().unwrap();
                *p = (cur, total);
            };
            match sftp::upload_file(&sftp, &local_path, &remote_str, &cb) {
                Ok(()) => {
                    let _ = tx.send(ActionResult::TransferDone(Side::Remote));
                }
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e)));
                }
            }
        });
    }

    fn download_selected(&mut self) {
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

        let total_size = entry.size;
        let remote_path = self.remote.current_path.join(&entry.name);
        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        let local_path = self.local.current_path.join(filename);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Downloading {}...", entry.name);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Mutex::new((0u64, total_size)));
        let progress_clone = progress.clone();

        self.pending = Some(PendingTransfer {
            progress,
            done_rx: rx,
        });
        self.progress_current = 0;
        self.progress_total = total_size;

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
            let cb = |cur, total| {
                let mut p = progress_clone.lock().unwrap();
                *p = (cur, total);
            };
            match sftp::download_file(&sftp, &remote_str, &local_path, &cb) {
                Ok(()) => {
                    let _ = tx.send(ActionResult::TransferDone(Side::Local));
                }
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e)));
                }
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
            sftp::remove_file(&sftp, &path_str)
                .or_else(|_| sftp::remove_dir(&sftp, &path_str))
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
            Side::Local => std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string()),
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
                sftp::rename_item(&sftp, &old_path.to_string_lossy(), &new_path.to_string_lossy())
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
            KeyCode::Right | KeyCode::Enter => self.enter_dir(),
            KeyCode::Left | KeyCode::Esc => self.parent_dir(),
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
            .constraints([
                Constraint::Length(2),                       // header (text + separator)
                Constraint::Min(2),                          // panels
                Constraint::Length(1),                       // status + help
            ])
            .split(area);

        // Header
        let header_lines = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[0]);

        let host = self.machine.effective_host();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled("文件浏览器", styles::header_style()),
                Span::styled(format!("  {}@{}:{}", self.machine.username, host, self.machine.port), styles::help_style()),
                Span::styled(format!("  {}", self.active_panel().current_path.display()), Style::default().fg(Color::DarkGray)),
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

        // Split panels
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);

        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        // Status + Help bar
        {
            let (status_color, prefix) = if self.pending.is_some() {
                (STATUS_BUSY, " \u{23F3} ")
            } else if self.status.starts_with("Error")
                || self.status.starts_with("Delete failed")
                || self.status.starts_with("Upload failed")
                || self.status.starts_with("Download failed")
            {
                (STATUS_ERR, " \u{2717} ")
            } else if self.status.starts_with("Transfer complete")
                || self.status.starts_with("Deleted")
                || self.status.starts_with("Renamed")
            {
                (STATUS_OK, " \u{2713} ")
            } else {
                (HINT, "   ")
            };

            let mut spans: Vec<Span> = vec![
                Span::styled(prefix, Style::default()),
                Span::styled(&self.status, Style::default().fg(status_color)),
                Span::styled(" │ ", styles::status_sep_style()),
            ];

            let right_spans: Vec<Span> = if self.confirm_delete.is_some() {
                vec![
                    Span::styled("y", styles::key_style()),
                    Span::styled(":yes  ", styles::help_style()),
                    Span::styled("n", styles::key_style()),
                    Span::styled(":no", styles::help_style()),
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
            } else {
                vec![
                    Span::styled("Tab", styles::key_style()),
                    Span::styled(":panel  ", styles::help_style()),
                    Span::styled("\u{2191}\u{2193}", styles::key_style()),
                    Span::styled(":move  ", styles::help_style()),
                    Span::styled("\u{2192}", styles::key_style()),
                    Span::styled(":enter  ", styles::help_style()),
                    Span::styled("\u{2190}", styles::key_style()),
                    Span::styled(":back  ", styles::help_style()),
                    Span::styled("u", styles::key_style()),
                    Span::styled(":upload  ", styles::help_style()),
                    Span::styled("d", styles::key_style()),
                    Span::styled(":download  ", styles::help_style()),
                    Span::styled("x", styles::key_style()),
                    Span::styled(":del  ", styles::help_style()),
                    Span::styled("r", styles::key_style()),
                    Span::styled(":rename  ", styles::help_style()),
                    Span::styled("q", styles::key_style()),
                    Span::styled(":quit", styles::help_style()),
                ]
            };

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
                chunks[2],
            );
        }
    }

    fn render_panel(&self, f: &mut Frame, area: Rect, side: Side) {
        let panel = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        let is_active = self.active_side == side;
        let title = format!(
            " {} {} ",
            side.label(),
            panel.current_path.display()
        );

        let border_style = if is_active {
            Style::default().fg(ACTIVE_BORDER)
        } else {
            Style::default().fg(INACTIVE_BORDER)
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Column widths
        let size_width: usize = 8;
        let modified_width: usize = 16;
        let gap: usize = 2;
        let overhead: usize = 1 + size_width + gap + modified_width;
        let name_area = (inner.width as usize).saturating_sub(overhead).max(6);

        // Column header
        let header = Line::from(vec![
            Span::styled(
                pad_right("Name", name_area),
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
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        // Separator under header
        f.render_widget(
            Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(DIM),
            )),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 1,
            },
        );

        // File entries
        let visible = (inner.height.saturating_sub(2)) as usize;
        let selected_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
            .bg(SELECTED_BG);
        let dir_style = Style::default().fg(DIR_FG);
        let file_style = Style::default().fg(FILE_FG);
        let dim_style = Style::default().fg(DIM);

        let marker_active = if is_active { "\u{25B8}" } else { " " };

        for i in 0..visible {
            let idx = panel.scroll_offset + i;
            if idx >= panel.entries.len() {
                break;
            }
            let entry = &panel.entries[idx];
            let is_selected = idx == panel.cursor;
            let style = if is_selected {
                selected_style
            } else if entry.is_dir {
                dir_style
            } else {
                file_style
            };

            let marker = if is_selected {
                marker_active
            } else {
                " "
            };
            let icon = if entry.is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };
            let short_name = entry.name.rsplit('/').next().unwrap_or(&entry.name);
            let name = format!("{}{} {}", marker, icon, short_name);

            let display_name = truncate_to_width(&name, name_area);
            let display_name_padded = pad_right(&display_name, name_area);

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
                    Span::styled(
                        &size_str,
                        if is_selected { selected_style } else { dim_style },
                    ),
                    Span::raw("  "),
                    Span::styled(
                        &modified_str,
                        if is_selected { selected_style } else { dim_style },
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
