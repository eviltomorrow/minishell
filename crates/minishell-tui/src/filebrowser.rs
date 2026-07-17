use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use minishell_core::Machine;
use minishell_ssh::sftp::{self, FileEntry, format_modified, format_perm};
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

struct TreeEntry {
    entry: FileEntry,
    depth: usize,
}

struct PanelState {
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
    prev_dir_name: Option<String>,
    tree_entries: Vec<TreeEntry>,
    expanded_dirs: Vec<PathBuf>,
}

impl PanelState {
    fn new(path: PathBuf) -> Self {
        PanelState {
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            current_path: path,
            prev_dir_name: None,
            tree_entries: Vec::new(),
            expanded_dirs: Vec::new(),
        }
    }
}

struct TransferProgressState {
    file_name: String,
    bytes: u64,
    total: u64,
}

enum ActionResult {
    TransferDone(Side),
    Error(String),
}

struct PendingTransfer {
    progress: Arc<Mutex<TransferProgressState>>,
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
    progress_file_name: String,
    progress_current: u64,
    progress_total: u64,
    confirm_delete: Option<(Side, usize)>,
    transfer_confirm: Option<Side>,
    rename_input: Option<String>,
    connecting_dots: u8,
    pending_connect: Option<mpsc::Receiver<Result<ssh2::Session, String>>>,
    connect_start: std::time::Instant,
    visible_rows: usize,
}

const HEADER_FG: Color = Color::Cyan;
const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::DarkGray;
const DIR_FG: Color = Color::Yellow;
const FILE_FG: Color = Color::White;
const SELECTED_BG: Color = Color::Blue;
const STATUS_OK: Color = Color::Green;
const STATUS_ERR: Color = Color::Red;
const DIM: Color = Color::DarkGray;
const HINT: Color = Color::Gray;

impl FileBrowserState {
    pub fn new(machine: Machine) -> Self {
        let local_path = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
        let mut fb = FileBrowserState {
            local: PanelState::new(local_path),
            remote: PanelState::new(PathBuf::from("/")),
            active_side: Side::Local,
            machine,
            session: None,
            status: "Connecting".to_string(),
            pending: None,
            progress_file_name: String::new(),
            progress_current: 0,
            progress_total: 0,
            confirm_delete: None,
            transfer_confirm: None,
            rename_input: None,
            connecting_dots: 0,
            pending_connect: None,
            connect_start: std::time::Instant::now(),
            visible_rows: 20,
        };
        let config = fb.build_config();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = minishell_ssh::create_session(&config)
                .map_err(|e| format!("SSH connection failed: {}", e));
            let _ = tx.send(result);
        });
        fb.pending_connect = Some(rx);
        fb.init_dirs();
        fb
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
            timeout: std::time::Duration::from_secs(5),
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
        if !self.status.starts_with("Error") && !self.status.starts_with("Connecting") {
            let p = self.active_panel();
            self.status = format!("{} entries", p.entries.len());
        }
    }

    fn clear_transfer(&mut self) {
        self.pending = None;
        self.progress_file_name.clear();
        self.progress_current = 0;
        self.progress_total = 0;
    }

    pub fn check_pending(&mut self) {
        if let Some(rx) = &self.pending_connect {
            match rx.try_recv() {
                Ok(Ok(session)) => {
                    self.pending_connect = None;
                    self.session = Some(session);
                    let home = if self.machine.username == "root" {
                        PathBuf::from("/root")
                    } else {
                        PathBuf::from(format!("/home/{}", self.machine.username))
                    };
                    self.remote.current_path = home;
                    self.refresh_remote();
                    if !self.status.starts_with("Error") {
                        let p = self.active_panel();
                        self.status = format!("{} entries", p.entries.len());
                    }
                }
                Ok(Err(e)) => {
                    self.pending_connect = None;
                    self.status = format!("Error: {}", e);
                    self.init_dirs();
                }
                Err(TryRecvError::Empty) => {
                    if self.connect_start.elapsed() > std::time::Duration::from_secs(5) {
                        self.pending_connect = None;
                        self.status = "Error: Connection timed out (5s)".to_string();
                        self.init_dirs();
                    } else {
                        self.connecting_dots = (self.connecting_dots + 1) % 4;
                        let dots = ".".repeat(self.connecting_dots as usize);
                        self.status = format!("Connecting{}", dots);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending_connect = None;
                    self.status = "Error: SSH connection failed".to_string();
                    self.init_dirs();
                }
            }
            return;
        }

        let transfer = match self.pending.as_ref() {
            Some(t) => t,
            None => return,
        };

        {
            let p = transfer.progress.lock().unwrap();
            self.progress_file_name = p.file_name.clone();
            self.progress_current = p.bytes;
            self.progress_total = p.total;
        }

        match transfer.done_rx.try_recv() {
            Ok(ActionResult::TransferDone(side)) => {
                self.clear_transfer();
                self.status = "Transfer complete".to_string();
                self.refresh_panel(side);
            }
            Ok(ActionResult::Error(e)) => {
                self.clear_transfer();
                self.status = format!("Error: {}", e);
            }
            Err(TryRecvError::Empty) => {
                if self.progress_total > 0 {
                    let pct = self.progress_current * 100 / self.progress_total;
                    self.status = format!("{} {}%", self.progress_file_name, pct);
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.clear_transfer();
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
                let perm = meta.as_ref().map(|m| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        format_perm(Some(m.permissions().mode() & 0o777), m.is_dir())
                    }
                    #[cfg(not(unix))]
                    { String::new() }
                }).unwrap_or_default();
                entries.push(FileEntry {
                    name,
                    is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified,
                    perm,
                });
            }
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
        self.local.entries = entries;
        self.local.cursor = self.local
            .cursor
            .min(self.local.entries.len().saturating_sub(1));
        self.local.scroll_offset = 0;
        Self::sync_tree(&mut self.local);
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
                Self::sync_tree(&mut self.remote);
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

    fn children_of_local(&self, path: &Path) -> Vec<FileEntry> {
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let meta = entry.metadata().ok();
                let modified = meta.as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| format_modified(Some(d.as_secs())))
                    .unwrap_or_default();
                let perm = meta.as_ref().map(|m| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        format_perm(Some(m.permissions().mode() & 0o777), m.is_dir())
                    }
                    #[cfg(not(unix))]
                    { String::new() }
                }).unwrap_or_default();
                entries.push(FileEntry {
                    name,
                    is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified,
                    perm,
                });
            }
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
        entries
    }

    fn children_of_remote(&self, path: &str) -> Vec<FileEntry> {
        let session = match self.session.as_ref() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let sftp = match session.sftp() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        sftp::list_dir(&sftp, path).unwrap_or_default()
    }

    fn rebuild_panel_tree(&mut self, side: Side) {
        let entries;
        let expanded_dirs;
        let current_path;
        match side {
            Side::Local => {
                entries = self.local.entries.clone();
                expanded_dirs = self.local.expanded_dirs.clone();
                current_path = self.local.current_path.clone();
            }
            Side::Remote => {
                entries = self.remote.entries.clone();
                expanded_dirs = self.remote.expanded_dirs.clone();
                current_path = self.remote.current_path.clone();
            }
        }

        let mut new_entries = Vec::new();
        self.append_tree_entries(&mut new_entries, &entries, &current_path, &expanded_dirs, 0, side);

        let apply_tree = |panel: &mut PanelState, entries: Vec<TreeEntry>| {
            let saved = panel.tree_entries.get(panel.cursor).map(|te| (te.entry.name.clone(), te.depth));
            panel.tree_entries = entries;
            if let Some((ref name, depth)) = saved {
                if let Some(pos) = panel.tree_entries.iter().position(|te| te.entry.name == *name && te.depth == depth) {
                    panel.cursor = pos;
                    return;
                }
            }
            panel.cursor = panel.cursor.min(panel.tree_entries.len().saturating_sub(1));
        };

        match side {
            Side::Local => apply_tree(&mut self.local, new_entries),
            Side::Remote => apply_tree(&mut self.remote, new_entries),
        }
    }

    fn append_tree_entries(
        &mut self,
        result: &mut Vec<TreeEntry>,
        entries: &[FileEntry],
        base_path: &Path,
        expanded_dirs: &[PathBuf],
        depth: usize,
        side: Side,
    ) {
        for entry in entries {
            result.push(TreeEntry { entry: entry.clone(), depth });
            if !entry.is_dir { continue; }
            let dir_path = base_path.join(&entry.name);
            if !expanded_dirs.contains(&dir_path) { continue; }
            let children = if side == Side::Remote {
                self.children_of_remote(&dir_path.to_string_lossy())
            } else {
                self.children_of_local(&dir_path)
            };
            self.append_tree_entries(result, &children, &dir_path, expanded_dirs, depth + 1, side);
        }
    }

    fn sync_tree(panel: &mut PanelState) {
        panel.tree_entries = panel.entries.iter()
            .map(|e| TreeEntry { entry: e.clone(), depth: 0 })
            .collect();
    }

    pub fn toggle_side(&mut self) {
        self.active_side = self.active_side.other();
        let p = self.active_panel();
        let count = if p.expanded_dirs.is_empty() { p.entries.len() } else { p.tree_entries.len() };
        self.status = format!("{} entries", count);
    }

    fn tree_entry_full_path(panel: &PanelState, cursor: usize) -> PathBuf {
        let te = &panel.tree_entries[cursor];
        if te.depth == 0 {
            return panel.current_path.join(&te.entry.name);
        }
        let mut ancestors: Vec<&str> = Vec::new();
        let mut need = te.depth;
        for i in (0..cursor).rev() {
            let prev = &panel.tree_entries[i];
            if prev.depth == need - 1 {
                ancestors.push(&prev.entry.name);
                need = prev.depth;
                if need == 0 {
                    break;
                }
            }
        }
        ancestors.reverse();
        let mut path = panel.current_path.clone();
        for a in ancestors {
            path = path.join(a);
        }
        path.join(&te.entry.name)
    }

    pub fn toggle_tree(&mut self) {
        if self.active_side == Side::Remote && self.session.is_none() {
            self.status = "Not connected".to_string();
            return;
        }

        let (full_path, is_dir, side) = {
            let p = self.active_panel();
            if p.tree_entries.is_empty() {
                return;
            }
            let entry = &p.tree_entries[p.cursor];
            if !entry.entry.is_dir {
                return;
            }
            let path = Self::tree_entry_full_path(p, p.cursor);
            (path, entry.entry.is_dir, self.active_side)
        };

        if !is_dir {
            return;
        }

        let was_expanded = match side {
            Side::Local => self.local.expanded_dirs.contains(&full_path),
            Side::Remote => self.remote.expanded_dirs.contains(&full_path),
        };

        if was_expanded {
            // Collapse: remove this path and all descendants from expanded_dirs
            match side {
                Side::Local => self.local.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
                Side::Remote => self.remote.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
            }
            self.rebuild_panel_tree(side);
            let count = match side {
                Side::Local => self.local.tree_entries.len(),
                Side::Remote => self.remote.tree_entries.len(),
            };
            self.status = format!("{} entries", count);
            return;
        }

        let children = if side == Side::Remote {
            self.children_of_remote(&full_path.to_string_lossy())
        } else {
            self.children_of_local(&full_path)
        };
        if children.is_empty() {
            self.status = format!("{} (empty)",
                full_path.file_name().map(|s| s.to_string_lossy()).unwrap_or_default());
            return;
        }

        let depth = {
            let p = self.active_panel();
            p.tree_entries[p.cursor].depth
        };

        let mut new_expanded = Vec::new();
        {
            let p = self.active_panel();
            let mut need = depth;
            for i in (0..p.cursor).rev() {
                let prev = &p.tree_entries[i];
                if need > 0 && prev.depth == need - 1 {
                    new_expanded.push(Self::tree_entry_full_path(p, i));
                    need = prev.depth;
                    if need == 0 { break; }
                }
            }
        }
        new_expanded.reverse();
        new_expanded.push(full_path);

        match side {
            Side::Local => self.local.expanded_dirs = new_expanded,
            Side::Remote => self.remote.expanded_dirs = new_expanded,
        }
        self.rebuild_panel_tree(side);
        let count = match side {
            Side::Local => self.local.tree_entries.len(),
            Side::Remote => self.remote.tree_entries.len(),
        };
        self.status = format!("{} entries", count);
    }

    fn move_cursor(&mut self, delta: isize, visible_rows: usize) {
        let panel = self.active_panel_mut();
        let len = panel.tree_entries.len();
        if len == 0 {
            return;
        }
        let new = (panel.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        panel.cursor = new;
        if new < panel.scroll_offset {
            panel.scroll_offset = new;
        } else if new >= panel.scroll_offset + visible_rows {
            panel.scroll_offset = new + 1 - visible_rows;
        }
    }

    fn cursor_first(&mut self) {
        let panel = self.active_panel_mut();
        if !panel.tree_entries.is_empty() {
            panel.cursor = 0;
            panel.scroll_offset = 0;
        }
    }

    fn cursor_last(&mut self, visible_rows: usize) {
        let panel = self.active_panel_mut();
        let len = panel.tree_entries.len();
        if len > 0 {
            panel.cursor = len - 1;
            if panel.cursor >= panel.scroll_offset + visible_rows {
                panel.scroll_offset = len.saturating_sub(visible_rows);
            }
        }
    }

    fn enter_dir(&mut self) {
        let (new_path, dir_name) = {
            let p = self.active_panel();
            if p.tree_entries.is_empty() {
                return;
            }
            let entry = &p.tree_entries[p.cursor].entry;
            if !entry.is_dir {
                let file_path = Self::tree_entry_full_path(p, p.cursor);
                if let Some(parent) = file_path.parent() {
                    if parent == p.current_path {
                        let size_str = sftp::format_size(entry.size);
                        self.status = format!("{} ({})", entry.name, size_str);
                        return;
                    }
                    let dir_name = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
                    (parent.to_path_buf(), dir_name)
                } else {
                    return;
                }
            } else {
                let dir_name = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
                let new_path = Self::tree_entry_full_path(p, p.cursor);
                (new_path, dir_name)
            }
        };
        {
            let p = self.active_panel_mut();
            p.expanded_dirs.clear();
            p.prev_dir_name = Some(dir_name);
            p.current_path = new_path;
            p.cursor = 0;
            p.scroll_offset = 0;
        }
        self.refresh_panel(self.active_side);
    }

    fn goto_root(&mut self) {
        {
            let panel = self.active_panel_mut();
            panel.expanded_dirs.clear();
            panel.current_path = PathBuf::from("/");
            panel.cursor = 0;
            panel.scroll_offset = 0;
        }
        self.refresh_panel(self.active_side);
    }

    fn goto_home(&mut self) {
        {
            let panel = self.active_panel_mut();
            panel.expanded_dirs.clear();
        }
        if self.active_side == Side::Remote {
            let home = if self.machine.username == "root" {
                PathBuf::from("/root")
            } else {
                PathBuf::from(format!("/home/{}", self.machine.username))
            };
            let panel = self.active_panel_mut();
            panel.current_path = home;
            panel.cursor = 0;
            panel.scroll_offset = 0;
            self.refresh_remote();
        } else {
            let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
            let panel = self.active_panel_mut();
            panel.current_path = home;
            panel.cursor = 0;
            panel.scroll_offset = 0;
            self.refresh_local();
        }
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
                p.expanded_dirs.clear();
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

    fn start_transfer(
        &self,
        tx: mpsc::Sender<ActionResult>,
        progress: Arc<Mutex<TransferProgressState>>,
        f: impl FnOnce(ssh2::Sftp, mpsc::Sender<ActionResult>, Arc<Mutex<TransferProgressState>>) + Send + 'static,
    ) {
        let config = self.build_config();
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
            f(sftp, tx, progress);
        });
    }

    fn init_transfer(&mut self, filename: String) -> (mpsc::Sender<ActionResult>, Arc<Mutex<TransferProgressState>>) {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Mutex::new(TransferProgressState {
            file_name: filename.clone(),
            bytes: 0,
            total: 0,
        }));
        self.pending = Some(PendingTransfer { progress: progress.clone(), done_rx: rx });
        self.progress_file_name = filename;
        self.progress_current = 0;
        self.progress_total = 0;
        (tx, progress)
    }

    fn upload_selected(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let entry = match self.local.entries.get(self.local.cursor).cloned() {
            Some(e) => e,
            None => return,
        };

        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
        let local_path = self.local.current_path.join(&filename);
        let remote_path = self.remote.current_path.join(&filename);
        let remote_str = remote_path.to_string_lossy().to_string();
        let is_dir = entry.is_dir;

        let (tx, progress) = self.init_transfer(filename.clone());

        if is_dir {
            self.status = format!("Uploading {}/...", filename);
            self.start_transfer(tx, progress, move |sftp, tx, progress| {
                let cb = |p: &sftp::TransferProgress| {
                    let mut state = progress.lock().unwrap();
                    state.file_name = p.file_name.clone();
                    state.bytes = p.bytes_written;
                    state.total = p.total_bytes;
                };
                let errors = sftp::upload_recursive(&sftp, &local_path, &remote_str, &cb);
                match errors {
                    Ok(errs) if errs.is_empty() => { let _ = tx.send(ActionResult::TransferDone(Side::Remote)); }
                    Ok(errs) => { let _ = tx.send(ActionResult::Error(errs.join("; "))); }
                    Err(e) => { let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e))); }
                }
            });
        } else {
            let total_size = entry.size;
            self.status = format!("Uploading {}...", filename);
            self.progress_total = total_size;
            self.start_transfer(tx, progress, move |sftp, tx, progress| {
                let cb = |cur: u64, total: u64| {
                    let mut state = progress.lock().unwrap();
                    state.bytes = cur;
                    state.total = total;
                };
                match sftp::upload_file(&sftp, &local_path, &remote_str, &cb) {
                    Ok(()) => { let _ = tx.send(ActionResult::TransferDone(Side::Remote)); }
                    Err(e) => { let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e))); }
                }
            });
        }
    }

    fn download_selected(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let entry = match self.remote.entries.get(self.remote.cursor).cloned() {
            Some(e) => e,
            None => return,
        };

        let remote_path = self.remote.current_path.join(&entry.name);
        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
        let local_path = self.local.current_path.join(&filename);
        let remote_str = remote_path.to_string_lossy().to_string();
        let is_dir = entry.is_dir;

        let (tx, progress) = self.init_transfer(filename.clone());

        if is_dir {
            self.status = format!("Downloading {}/...", filename);
            self.start_transfer(tx, progress, move |sftp, tx, progress| {
                let _ = std::fs::create_dir_all(&local_path);
                let cb = |p: &sftp::TransferProgress| {
                    let mut state = progress.lock().unwrap();
                    state.file_name = p.file_name.clone();
                    state.bytes = p.bytes_written;
                    state.total = p.total_bytes;
                };
                let errors = sftp::download_recursive(&sftp, &remote_str, &local_path, &cb);
                match errors {
                    Ok(errs) if errs.is_empty() => { let _ = tx.send(ActionResult::TransferDone(Side::Local)); }
                    Ok(errs) => { let _ = tx.send(ActionResult::Error(errs.join("; "))); }
                    Err(e) => { let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e))); }
                }
            });
        } else {
            let total_size = entry.size;
            self.status = format!("Downloading {}...", filename);
            self.progress_total = total_size;
            self.start_transfer(tx, progress, move |sftp, tx, progress| {
                let cb = |cur: u64, total: u64| {
                    let mut state = progress.lock().unwrap();
                    state.bytes = cur;
                    state.total = total;
                };
                match sftp::download_file(&sftp, &remote_str, &local_path, &cb) {
                    Ok(()) => { let _ = tx.send(ActionResult::TransferDone(Side::Local)); }
                    Err(e) => { let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e))); }
                }
            });
        }
    }

    fn start_transfer_confirm(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let side = self.active_side;
        let panel = self.active_panel();
        let cursor = panel.cursor;
        let entry = match panel.entries.get(cursor).cloned() {
            Some(e) => e,
            None => return,
        };
        if entry.name == ".." {
            return;
        }
        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
        let type_label = if entry.is_dir { "[DIR]" } else { "[FILE]" };
        let direction = match side {
            Side::Local => "\u{2192} remote",
            Side::Remote => "\u{2190} local",
        };
        self.status = format!("Transfer {} {} {}?", type_label, filename, direction);
        self.transfer_confirm = Some(side);
    }

    fn confirm_transfer(&mut self) {
        let side = self.transfer_confirm.take().unwrap_or(self.active_side);
        match side {
            Side::Local => self.upload_selected(),
            Side::Remote => self.download_selected(),
        }
    }

    fn start_delete(&mut self) {
        let side = self.active_side;
        let (entry, cursor) = {
            let p = self.active_panel();
            let cursor = p.cursor;
            match p.entries.get(cursor).cloned() {
                Some(e) => (e, cursor),
                None => return,
            }
        };
        let panel = self.active_panel();
        let full_path = panel.current_path.join(&entry.name);
        let side_label = match side {
            Side::Local => "Local",
            Side::Remote => "Remote",
        };
        let type_label = if entry.is_dir { "[DIR]" } else { "[FILE]" };
        let size_label = if entry.is_dir {
            String::new()
        } else {
            format!(" ({})", sftp::format_size(entry.size))
        };
        self.status = format!(
            "Delete {} {}?  {}:{}{}",
            type_label, entry.name, side_label, full_path.display(), size_label
        );
        self.confirm_delete = Some((side, cursor));
    }

    fn confirm_delete_action(&mut self) {
        let (side, idx) = match self.confirm_delete.take() {
            Some(v) => v,
            None => return,
        };

        let (path_str, entry_name, is_dir) = {
            let panel = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            let entry = match panel.entries.get(idx) {
                Some(e) => e.clone(),
                None => return,
            };
            let p = panel.current_path.join(&entry.name);
            (p.to_string_lossy().to_string(), entry.name.clone(), entry.is_dir)
        };

        let result = if side == Side::Local {
            let path = std::path::Path::new(&path_str);
            if is_dir {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
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
            if is_dir {
                match sftp::remove_recursive(&sftp, &path_str) {
                    Ok(errs) if errs.is_empty() => Ok(()),
                    Ok(errs) => Err(errs.join("; ")),
                    Err(e) => Err(e.to_string()),
                }
            } else {
                sftp::remove_file(&sftp, &path_str).map_err(|e| e.to_string())
            }
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
        let short = entry_name.name.rsplit('/').next().unwrap_or(&entry_name.name).to_string();
        self.rename_input = Some(short);
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

        if self.transfer_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_transfer(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.transfer_confirm = None;
                    self.status = "Transfer cancelled".to_string();
                }
                _ => {}
            }
            return;
        }

        if self.pending.is_some() {
            return;
        }

        if !self.active_panel().expanded_dirs.is_empty() {
            match key.code {
                KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Char('r') => {
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Up => self.move_cursor(-1, self.visible_rows),
            KeyCode::Down => self.move_cursor(1, self.visible_rows),
            KeyCode::PageUp => self.cursor_first(),
            KeyCode::PageDown => self.cursor_last(self.visible_rows),
            KeyCode::Right | KeyCode::Enter => self.enter_dir(),
            KeyCode::Left | KeyCode::Esc => self.parent_dir(),
            KeyCode::Tab => self.toggle_side(),
            KeyCode::Char('/') => self.goto_root(),
            KeyCode::Char('~') => self.goto_home(),
            KeyCode::Char('x') => self.start_transfer_confirm(),
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('t') => self.toggle_tree(),
            _ => {}
        }
    }

    pub fn render(&mut self, f: &mut Frame) {
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

        self.visible_rows = (chunks[1].height as usize).saturating_sub(4).max(1);
        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        // Status + Help bar
        {
            let is_transferring = self.pending.is_some();
            let (status_color, prefix) = if is_transferring {
                (Color::Yellow, " \u{25B6} ")
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

            let status_text = if self.rename_input.is_some() {
                if let Some(ref input) = self.rename_input {
                    format!("{} {}", self.status, input)
                } else {
                    self.status.clone()
                }
            } else if self.transfer_confirm.is_some() {
                self.status.clone()
            } else if self.pending.is_some() && self.progress_total > 0 {
                let pct = (self.progress_current * 100 / self.progress_total) as usize;
                let bar_width = 20;
                let filled = pct * bar_width / 100;
                let empty = bar_width - filled;
                let bar: String = "\u{2588}".repeat(filled) + &"\u{2591}".repeat(empty);
                format!("{} {} {}%", self.progress_file_name, bar, pct)
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
                if status_text.contains("[DIR]") {
                    spans.push(Span::styled("Delete [DIR]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                    if let Some(pos) = status_text.find("?") {
                        let name = &status_text[12..pos];
                        let rest = &status_text[pos..];
                        spans.push(Span::styled(format!(" {}?", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
                        spans.push(Span::styled(rest, Style::default().fg(Color::DarkGray)));
                    }
                } else {
                    spans.push(Span::styled("Delete [FILE]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                    if let Some(pos) = status_text.find("?") {
                        let name = &status_text[13..pos];
                        let rest = &status_text[pos..];
                        spans.push(Span::styled(format!(" {}?", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
                        spans.push(Span::styled(rest, Style::default().fg(Color::DarkGray)));
                    }
                }
            } else if self.transfer_confirm.is_some() {
                if let Some(type_end) = status_text.find(']') {
                    let type_part = &status_text[..=type_end];
                    let rest = &status_text[type_end + 1..];
                    spans.push(Span::styled(type_part, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
                    if let Some(dir_pos) = rest.find('\u{2192}').or_else(|| rest.find('\u{2190}')) {
                        let name = &rest[..dir_pos];
                        let direction = &rest[dir_pos..];
                        spans.push(Span::styled(name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
                        spans.push(Span::styled(direction, Style::default().fg(Color::Green)));
                    } else {
                        spans.push(Span::styled(rest, Style::default().fg(Color::White)));
                    }
                } else {
                    spans.push(Span::styled(&status_text, Style::default().fg(Color::Cyan)));
                }
            } else {
                spans.push(Span::styled(&status_text, Style::default().fg(status_color)));
            }

            spans.push(Span::styled(" │ ", styles::status_sep_style()));

            let right_spans: Vec<Span> = if self.confirm_delete.is_some() {
                vec![
                    Span::styled("[Y]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled("es  ", styles::help_style()),
                    Span::styled("[N]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled("o", styles::help_style()),
                ]
            } else if self.transfer_confirm.is_some() {
                vec![
                    Span::styled("[Y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
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
            } else if !self.active_panel().expanded_dirs.is_empty() {
                vec![
                    Span::styled("Tab", styles::key_style()),
                    Span::styled(":panel  ", styles::help_style()),
                    Span::styled("\u{2191}\u{2193}", styles::key_style()),
                    Span::styled(":move  ", styles::help_style()),
                    Span::styled("\u{2192}", styles::key_style()),
                    Span::styled(":enter  ", styles::help_style()),
                    Span::styled("\u{2190}", styles::key_style()),
                    Span::styled(":back  ", styles::help_style()),
                    Span::styled("t", styles::key_style()),
                    Span::styled(":tree  ", styles::help_style()),
                    Span::styled("q", styles::key_style()),
                    Span::styled(":quit", styles::help_style()),
                ]
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
                    Span::styled("x", styles::key_style()),
                    Span::styled(":transfer  ", styles::help_style()),
                    Span::styled("d", styles::key_style()),
                    Span::styled(":del  ", styles::help_style()),
                    Span::styled("r", styles::key_style()),
                    Span::styled(":rename  ", styles::help_style()),
                    Span::styled("t", styles::key_style()),
                    Span::styled(":tree  ", styles::help_style()),
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

            if self.rename_input.is_some() {
                let prefix_width: usize = 3;
                let status_width = UnicodeWidthStr::width(status_text.as_str());
                let cx = chunks[2].x + prefix_width as u16 + status_width as u16;
                f.set_cursor_position((cx, chunks[2].y));
            }
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

        // Direction marker based on transfer state
        let is_uploading = self.pending.is_some() && self.active_side == Side::Local;
        let is_downloading = self.pending.is_some() && self.active_side == Side::Remote;
        let is_upload_confirm = self.transfer_confirm == Some(Side::Local);
        let is_download_confirm = self.transfer_confirm == Some(Side::Remote);
        let transferring = is_uploading || is_downloading || is_upload_confirm || is_download_confirm;

        let arrow_color = if is_uploading || is_downloading {
            Color::Yellow
        } else if is_upload_confirm || is_download_confirm {
            Color::Green
        } else {
            Color::White
        };

        let (prefix, suffix) = if transferring {
            let is_source = match side {
                Side::Local => is_uploading || is_upload_confirm,
                Side::Remote => is_downloading || is_download_confirm,
            };
            if is_source {
                // Source panel: label →
                ("", " → ")
            } else {
                // Destination panel: → label
                ("→ ", "")
            }
        } else {
            ("", " ")
        };

        let label = format!("{}{}{}", prefix, side.label(), suffix);
        let title = Line::from(vec![
            Span::styled(
                label,
                if transferring {
                    Style::default().fg(arrow_color).add_modifier(Modifier::BOLD)
                } else {
                    title_style
                },
            ),
            Span::styled(
                panel.current_path.display().to_string(),
                if is_active || transferring {
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
                ratatui::widgets::BorderType::Double
            } else {
                ratatui::widgets::BorderType::Rounded
            });

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Column widths
        let perm_width: usize = 10;
        let size_width: usize = 8;
        let modified_width: usize = 16;
        let gap: usize = 2;
        let overhead: usize = 1 + perm_width + 1 + size_width + gap + modified_width;
        let name_area = (inner.width as usize).saturating_sub(overhead).max(6);

        // Column header
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

        // Empty directory hint
        if panel.tree_entries.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  (empty)",
                    Style::default().fg(DIM),
                ))),
                Rect {
                    x: inner.x,
                    y: inner.y + 2,
                    width: inner.width,
                    height: 1,
                },
            );
            return;
        }

        // File entries
        let visible = (inner.height.saturating_sub(2)) as usize;
        let selected_active_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
            .bg(SELECTED_BG);
        let selected_inactive_style = Style::default()
            .fg(Color::White)
            .bg(Color::Rgb(70, 70, 90));
        let selected_style = if is_active {
            selected_active_style
        } else {
            selected_inactive_style
        };
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
                    Span::styled(
                        &perm_str,
                        if is_selected { selected_style } else { dim_style },
                    ),
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
