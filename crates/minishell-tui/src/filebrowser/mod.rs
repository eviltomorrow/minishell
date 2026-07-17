use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
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

pub mod types;
pub mod panel;
pub mod transfer;
pub mod render;

pub use types::{Side, TreeEntry, ClipboardEntry};
pub use panel::PanelState;
pub use transfer::{TransferProgressState, ActionResult, PendingTransfer};
pub use render::{HEADER_FG, ACTIVE_BORDER, INACTIVE_BORDER, DIR_FG, FILE_FG, SELECTED_BG, STATUS_OK, STATUS_ERR, DIM, HINT, render_name_and_path};
use render::{format_size, pad_left, pad_right, truncate_to_width};

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
    confirm_delete: Option<(Side, String)>,
    transfer_confirm: Option<Side>,
    rename_input: Option<String>,
    old_entry_name: String,
    old_entry_path: std::path::PathBuf,
    clipboard: Vec<ClipboardEntry>,
    clipboard_side: Option<Side>,
    clipboard_panel_open: bool,
    clipboard_panel_cursor: usize,
    switch_confirm: Option<Side>,
    transfer_queue: Vec<ClipboardEntry>,
    connecting_dots: u8,
    pending_connect: Option<mpsc::Receiver<Result<ssh2::Session, String>>>,
    connect_start: std::time::Instant,
    visible_rows: usize,
}

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
            old_entry_name: String::new(),
            old_entry_path: std::path::PathBuf::new(),
            clipboard: Vec::new(),
            clipboard_side: None,
            clipboard_panel_open: false,
            clipboard_panel_cursor: 0,
            switch_confirm: None,
            transfer_queue: Vec::new(),
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

        let progress_arc = transfer.progress.clone();

        {
            let p = match progress_arc.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    self.clear_transfer();
                    self.status = "Transfer failed (internal error)".to_string();
                    return;
                }
            };
            self.progress_file_name = p.file_name.clone();
            self.progress_current = p.bytes;
            self.progress_total = p.total;
        }

        match transfer.done_rx.try_recv() {
            Ok(ActionResult::TransferDone(side)) => {
                self.clear_transfer();
                self.refresh_panel(side);
                if self.transfer_queue.is_empty() {
                    self.status = "Transfer complete".to_string();
                } else {
                    self.transfer_queue.remove(0);
                    if self.transfer_queue.is_empty() {
                        self.status = "Transfer complete".to_string();
                    } else {
                        self.process_next_transfer();
                    }
                }
            }
            Ok(ActionResult::Error(e)) => {
                self.clear_transfer();
                self.status = format!("Error: {}", e);
                if !self.transfer_queue.is_empty() {
                    self.transfer_queue.remove(0);
                    if !self.transfer_queue.is_empty() {
                        self.process_next_transfer();
                    }
                }
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
        if self.local.expanded_dirs.is_empty() {
            Self::sync_tree(&mut self.local);
        } else {
            self.rebuild_panel_tree(Side::Local);
        }
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
                if self.remote.expanded_dirs.is_empty() {
                    Self::sync_tree(&mut self.remote);
                } else {
                    self.rebuild_panel_tree(Side::Remote);
                }
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

    fn current_entry(&self) -> Option<FileEntry> {
        let p = self.active_panel();
        if !p.expanded_dirs.is_empty() {
            p.tree_entries.get(p.cursor).map(|te| te.entry.clone())
        } else {
            p.entries.get(p.cursor).cloned()
        }
    }

    fn current_entry_full_path(&self) -> PathBuf {
        let p = self.active_panel();
        if !p.expanded_dirs.is_empty() {
            Self::tree_entry_full_path(p, p.cursor)
        } else {
            let entry = match p.entries.get(p.cursor) {
                Some(e) => e,
                None => return p.current_path.clone(),
            };
            p.current_path.join(&entry.name)
        }
    }

    fn current_entry_for_side(&self, side: Side) -> Option<FileEntry> {
        let p = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        if !p.expanded_dirs.is_empty() {
            p.tree_entries.get(p.cursor).map(|te| te.entry.clone())
        } else {
            p.entries.get(p.cursor).cloned()
        }
    }

    fn entry_full_path_for_side(&self, side: Side) -> PathBuf {
        let p = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        if !p.expanded_dirs.is_empty() {
            Self::tree_entry_full_path(p, p.cursor)
        } else {
            let entry = match p.entries.get(p.cursor) {
                Some(e) => e,
                None => return p.current_path.clone(),
            };
            p.current_path.join(&entry.name)
        }
    }

    fn is_effectively_selected(&self, entry_name: &str, side: Side, cursor: usize) -> bool {
        if self.clipboard.iter().any(|c| c.name == entry_name && c.source_side == side) {
            return true;
        }
        let p = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        if p.expanded_dirs.is_empty() || cursor >= p.tree_entries.len() {
            return false;
        }
        let mut depth = p.tree_entries[cursor].depth;
        if depth == 0 {
            return false;
        }
        for i in (0..cursor).rev() {
            if p.tree_entries[i].depth < depth {
                let parent_name = &p.tree_entries[i].entry.name;
                if self.clipboard.iter().any(|c| c.name == *parent_name && c.source_side == side) {
                    return true;
                }
                if p.tree_entries[i].depth == 0 {
                    break;
                }
                depth = p.tree_entries[i].depth;
            }
        }
        false
    }

    fn yank_toggle(&mut self) {
        let entry = match self.current_entry() {
            Some(e) => e,
            None => return,
        };
        if entry.name == ".." {
            return;
        }
        let full_path = self.current_entry_full_path();
        let side = self.active_side;
        let cursor = self.active_panel().cursor;

        // Directly in clipboard → remove
        if let Some(pos) = self.clipboard.iter().position(|c| c.source_path == full_path) {
            self.clipboard.remove(pos);
            if self.clipboard.is_empty() {
                self.clipboard_side = None;
            }
            self.status = format!("Unmarked: {}", entry.name);
            return;
        }

        // Selected via parent → deselect parent
        if self.is_effectively_selected(&entry.name, side, cursor) {
            let p = self.active_panel();
            let mut depth = p.tree_entries[cursor].depth;
            for i in (0..cursor).rev() {
                if p.tree_entries[i].depth < depth {
                    let parent_name = p.tree_entries[i].entry.name.clone();
                    if let Some(pos) = self.clipboard.iter().position(|c| c.name == parent_name && c.source_side == side) {
                        self.clipboard.remove(pos);
                        if self.clipboard.is_empty() {
                            self.clipboard_side = None;
                        }
                        self.status = format!("Unmarked: {}", parent_name);
                        return;
                    }
                    if p.tree_entries[i].depth == 0 {
                        break;
                    }
                    depth = p.tree_entries[i].depth;
                }
            }
            return;
        }

        // Cross-side check
        if let Some(clip_side) = self.clipboard_side {
            if clip_side != side {
                self.switch_confirm = Some(side);
                self.status = format!(
                    "Switch to {}? Clear selection [Y/N]",
                    side.label()
                );
                return;
            }
        }

        // Add to clipboard
        if self.clipboard_side.is_none() {
            self.clipboard_side = Some(side);
        }
        self.clipboard.push(ClipboardEntry {
            source_path: full_path,
            source_side: side,
            name: entry.name.clone(),
            is_dir: entry.is_dir,
        });
        self.status = format!("✓ Copied: {} ({})", entry.name, side.label());
    }

    fn confirm_switch(&mut self) {
        let side = match self.switch_confirm.take() {
            Some(s) => s,
            None => return,
        };
        let entry = match self.current_entry() {
            Some(e) => e,
            None => return,
        };
        let full_path = self.current_entry_full_path();
        self.clipboard.clear();
        self.clipboard_panel_cursor = 0;
        self.clipboard_side = Some(side);
        self.clipboard.push(ClipboardEntry {
            source_path: full_path,
            source_side: side,
            name: entry.name.clone(),
            is_dir: entry.is_dir,
        });
        self.status = format!("✓ Copied: {} ({})", entry.name, side.label());
    }

    fn open_clipboard_panel(&mut self) {
        if self.clipboard.is_empty() {
            self.status = "No files selected".to_string();
            return;
        }
        self.clipboard_panel_open = true;
        self.clipboard_panel_cursor = 0;
    }

    fn close_clipboard_panel(&mut self) {
        self.clipboard_panel_open = false;
    }

    fn clipboard_panel_next(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.clipboard_panel_cursor = (self.clipboard_panel_cursor + 1) % self.clipboard.len();
    }

    fn clipboard_panel_remove(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        self.clipboard.remove(self.clipboard_panel_cursor);
        if self.clipboard.is_empty() {
            self.clipboard_side = None;
            self.clipboard_panel_open = false;
            self.status = "Selection cleared".to_string();
            return;
        }
        if self.clipboard_panel_cursor >= self.clipboard.len() {
            self.clipboard_panel_cursor = self.clipboard.len() - 1;
        }
    }

    fn paste_from_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            self.status = "No files selected".to_string();
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let clip_side = match self.clipboard_side {
            Some(s) => s,
            None => return,
        };
        if clip_side == self.active_side {
            self.status = format!("Already on {} side", clip_side.label());
            return;
        }
        if clip_side == Side::Remote && self.session.is_none() {
            self.status = "Not connected".to_string();
            return;
        }

        self.transfer_queue = self.clipboard.drain(..).collect();
        self.clipboard_side = None;
        self.clipboard_panel_open = false;
        self.process_next_transfer();
    }

    fn process_next_transfer(&mut self) {
        let entry = match self.transfer_queue.first() {
            Some(e) => e.clone(),
            None => {
                self.status = "Transfer complete".to_string();
                return;
            }
        };

        let source_path = entry.source_path.clone();
        let source_side = entry.source_side;
        let name = entry.name.clone();
        let is_dir = entry.is_dir;

        let dest_path = self.active_panel().current_path.join(&name);
        let dest_str = dest_path.to_string_lossy().to_string();
        let src_str = source_path.to_string_lossy().to_string();

        let (tx, progress) = self.init_transfer(name.clone());

        match source_side {
            Side::Local => {
                if is_dir {
                    self.status = format!("Uploading {}/...", name);
                    self.start_transfer(tx, progress, move |sftp, tx, progress| {
                        let cb = |p: &sftp::TransferProgress| {
                            let mut state = progress.lock().unwrap();
                            state.file_name = p.file_name.clone();
                            state.bytes = p.bytes_written;
                            state.total = p.total_bytes;
                        };
                        let errors = sftp::upload_recursive(&sftp, &source_path, &dest_str, &cb);
                        match errors {
                            Ok(errs) if errs.is_empty() => { let _ = tx.send(ActionResult::TransferDone(Side::Remote)); }
                            Ok(errs) => { let _ = tx.send(ActionResult::Error(errs.join("; "))); }
                            Err(e) => { let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e))); }
                        }
                    });
                } else {
                    let total_size = std::fs::metadata(&source_path).map(|m| m.len()).unwrap_or(0);
                    self.status = format!("Uploading {}...", name);
                    self.progress_total = total_size;
                    self.start_transfer(tx, progress, move |sftp, tx, progress| {
                        let cb = |cur: u64, total: u64| {
                            let mut state = progress.lock().unwrap();
                            state.bytes = cur;
                            state.total = total;
                        };
                        match sftp::upload_file(&sftp, &source_path, &dest_str, &cb) {
                            Ok(()) => { let _ = tx.send(ActionResult::TransferDone(Side::Remote)); }
                            Err(e) => { let _ = tx.send(ActionResult::Error(format!("Upload failed: {}", e))); }
                        }
                    });
                }
            }
            Side::Remote => {
                if is_dir {
                    self.status = format!("Downloading {}/...", name);
                    self.start_transfer(tx, progress, move |sftp, tx, progress| {
                        let _ = std::fs::create_dir_all(&dest_path);
                        let cb = |p: &sftp::TransferProgress| {
                            let mut state = progress.lock().unwrap();
                            state.file_name = p.file_name.clone();
                            state.bytes = p.bytes_written;
                            state.total = p.total_bytes;
                        };
                        let errors = sftp::download_recursive(&sftp, &src_str, &dest_path, &cb);
                        match errors {
                            Ok(errs) if errs.is_empty() => { let _ = tx.send(ActionResult::TransferDone(Side::Local)); }
                            Ok(errs) => { let _ = tx.send(ActionResult::Error(errs.join("; "))); }
                            Err(e) => { let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e))); }
                        }
                    });
                } else {
                    self.status = format!("Downloading {}...", name);
                    self.start_transfer(tx, progress, move |sftp, tx, progress| {
                        let cb = |cur: u64, total: u64| {
                            let mut state = progress.lock().unwrap();
                            state.bytes = cur;
                            state.total = total;
                        };
                        match sftp::download_file(&sftp, &src_str, &dest_path, &cb) {
                            Ok(()) => { let _ = tx.send(ActionResult::TransferDone(Side::Local)); }
                            Err(e) => { let _ = tx.send(ActionResult::Error(format!("Download failed: {}", e))); }
                        }
                    });
                }
            }
        }
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
            Side::Local => {
                for p in new_expanded {
                    if !self.local.expanded_dirs.contains(&p) {
                        self.local.expanded_dirs.push(p);
                    }
                }
            }
            Side::Remote => {
                for p in new_expanded {
                    if !self.remote.expanded_dirs.contains(&p) {
                        self.remote.expanded_dirs.push(p);
                    }
                }
            }
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
                    let dir_name = parent.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
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

    fn collapse_or_navigate_tree(&mut self) {
        let p = self.active_panel();
        if p.expanded_dirs.is_empty() || p.tree_entries.is_empty() {
            self.parent_dir();
            return;
        }

        let te = &p.tree_entries[p.cursor];
        let is_expanded = te.entry.is_dir
            && p.expanded_dirs.iter().any(|d| {
                d.file_name().map(|n| n.to_string_lossy() == te.entry.name).unwrap_or(false)
            });

        if is_expanded {
            let full_path = Self::tree_entry_full_path(p, p.cursor);
            let depth = te.depth;

            if depth == 0 {
                let side = self.active_side;
                match side {
                    Side::Local => self.local.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
                    Side::Remote => self.remote.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
                }
                self.rebuild_panel_tree(side);
            } else {
                let side = self.active_side;
                match side {
                    Side::Local => self.local.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
                    Side::Remote => self.remote.expanded_dirs.retain(|p| !p.starts_with(&full_path)),
                }
                let cursor = self.active_panel().cursor;
                let parent_depth = depth - 1;
                let mut parent_idx = None;
                for i in (0..cursor).rev() {
                    if self.active_panel().tree_entries[i].depth == parent_depth {
                        parent_idx = Some(i);
                        break;
                    }
                }
                if let Some(idx) = parent_idx {
                    self.active_panel_mut().cursor = idx;
                }
                self.rebuild_panel_tree(side);
            }
        } else {
            let depth = te.depth;
            if depth == 0 {
                self.parent_dir();
                return;
            }
            let cursor = p.cursor;
            let parent_depth = depth - 1;
            let mut parent_idx = None;
            for i in (0..cursor).rev() {
                if p.tree_entries[i].depth == parent_depth {
                    parent_idx = Some(i);
                    break;
                }
            }
            if let Some(idx) = parent_idx {
                self.active_panel_mut().cursor = idx;
            } else {
                self.parent_dir();
            }
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
        let entry = match self.current_entry_for_side(Side::Local) {
            Some(e) => e,
            None => return,
        };

        let filename = entry.name.clone();
        let local_path = self.entry_full_path_for_side(Side::Local);
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
        let entry = match self.current_entry_for_side(Side::Remote) {
            Some(e) => e,
            None => return,
        };

        let filename = entry.name.clone();
        let remote_path = self.entry_full_path_for_side(Side::Remote);
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
        let entry = match self.current_entry() {
            Some(e) => e,
            None => return,
        };
        if entry.name == ".." {
            return;
        }
        let full_path = self.current_entry_full_path();
        let filename = entry.name.clone();
        let type_label = if entry.is_dir { "[DIR]" } else { "[FILE]" };
        let (src_path, dst_path) = match side {
            Side::Local => {
                let remote_path = self.remote.current_path.join(&filename);
                (full_path, remote_path)
            }
            Side::Remote => {
                let local_path = self.local.current_path.join(&filename);
                (local_path, full_path)
            }
        };
        let direction_label = match side {
            Side::Local => "Push",
            Side::Remote => "Pull",
        };
        let arrow = match side {
            Side::Local => "->",
            Side::Remote => "<-",
        };
        let (left_path, right_path) = match side {
            Side::Local => (src_path, dst_path),   // src -> dst
            Side::Remote => (dst_path, src_path),   // dst <- src
        };
        self.status = format!(
            "{}:|{}|{}│{}|{}|{}",
            direction_label, type_label, filename, left_path.display(), arrow, right_path.display()
        );
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
        let entry = match self.current_entry() {
            Some(e) => e,
            None => return,
        };
        let full_path = self.current_entry_full_path();
        let side_label = match side {
            Side::Local => "Local",
            Side::Remote => "Remote",
        };
        let type_label = if entry.is_dir { "[DIR]" } else { "[FILE]" };
        self.status = format!(
            "Delete:|{}|{}│{}|{}",
            type_label, entry.name, full_path.display(), side_label
        );
        self.confirm_delete = Some((side, entry.name));
    }

    fn confirm_delete_action(&mut self) {
        let (side, entry_name) = match self.confirm_delete.take() {
            Some(v) => v,
            None => return,
        };

        let (path_str, is_dir) = {
            let panel = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            let entry = match panel.entries.iter().find(|e| e.name == entry_name) {
                Some(e) => e.clone(),
                None => return,
            };
            let p = panel.current_path.join(&entry.name);
            (p.to_string_lossy().to_string(), entry.is_dir)
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
        let entry = match self.current_entry() {
            Some(e) => e,
            None => return,
        };
        self.old_entry_name = entry.name.clone();
        self.old_entry_path = self.current_entry_full_path();
        self.rename_input = Some(entry.name.clone());
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
        let old_path = self.old_entry_path.clone();
        let new_path = old_path.parent().unwrap_or(&old_path).join(&new_name);

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

        if self.switch_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_switch(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.switch_confirm = None;
                    self.status = "Selection unchanged".to_string();
                }
                _ => {}
            }
            return;
        }

        if self.clipboard_panel_open {
            match key.code {
                KeyCode::Char('n') => {
                    self.clipboard_panel_next();
                    return;
                }
                KeyCode::Char('k') => {
                    self.clipboard_panel_remove();
                    return;
                }
                KeyCode::Char('v') | KeyCode::Esc => {
                    self.close_clipboard_panel();
                    return;
                }
                KeyCode::Char('p') => {
                    self.close_clipboard_panel();
                    self.paste_from_clipboard();
                    return;
                }
                KeyCode::Char('y') => {
                    return; // block yank while panel is open
                }
                _ => {} // fall through to normal handler
            }
        }

        if self.pending.is_some() {
            return;
        }

        match key.code {
            KeyCode::Up => self.move_cursor(-1, self.visible_rows),
            KeyCode::Down => self.move_cursor(1, self.visible_rows),
            KeyCode::PageUp => self.cursor_first(),
            KeyCode::PageDown => self.cursor_last(self.visible_rows),
            KeyCode::Right | KeyCode::Enter => self.enter_dir(),
            KeyCode::Left => self.collapse_or_navigate_tree(),
            KeyCode::Esc => self.parent_dir(),
            KeyCode::Tab => self.toggle_side(),
            KeyCode::Char('/') => self.goto_root(),
            KeyCode::Char('~') => self.goto_home(),
            KeyCode::Char('x') => self.start_transfer_confirm(),
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('t') => self.toggle_tree(),
            KeyCode::Char('y') => self.yank_toggle(),
            KeyCode::Char('v') => self.open_clipboard_panel(),
            KeyCode::Char('p') => self.paste_from_clipboard(),
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

        // Split panels
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);

        self.visible_rows = (chunks[1].height as usize).saturating_sub(4).max(1);
        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        // Clipboard panel overlay
        if self.clipboard_panel_open {
            let panel_area = match self.clipboard_side.unwrap_or(self.active_side) {
                Side::Local => panels[0],
                Side::Remote => panels[1],
            };
            self.render_clipboard_panel(f, panel_area);
        }

        // Status + Help bar
        {
            let is_transferring = self.pending.is_some();
            let (status_color, prefix) = if is_transferring {
                (Color::Yellow, "\u{25B6}")
            } else if self.status.starts_with("Error")
                || self.status.starts_with("Delete failed")
                || self.status.starts_with("Upload failed")
                || self.status.starts_with("Download failed")
            {
                (STATUS_ERR, "\u{2717}")
            } else if self.status.starts_with("Transfer complete")
                || self.status.starts_with("Deleted")
                || self.status.starts_with("Renamed")
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
            } else if self.transfer_confirm.is_some() || self.pending.is_some() {
                self.status.clone()
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
                // Format: "Delete:|[DIR]|name│/full/path|side"
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
                    // Append "?" to the last span (path)
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
                // Format: "Push:|[DIR]|name│/src/path|->|/dst/path"
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

            let right_spans: Vec<Span> = if self.confirm_delete.is_some() {
                vec![
                    Span::styled("[Y]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled("es  ", styles::help_style()),
                    Span::styled("[N]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled("o", styles::help_style()),
                ]
            } else if self.transfer_confirm.is_some() {
                vec![
                    Span::styled("[Y]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled("es  ", styles::help_style()),
                    Span::styled("[N]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled("o", styles::help_style()),
                ]
            } else if self.switch_confirm.is_some() {
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
                vec![
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
                    Span::styled("y", styles::key_style()),
                    Span::styled(":copy  ", styles::help_style()),
                    Span::styled("v", styles::key_style()),
                    Span::styled(":view  ", styles::help_style()),
                    Span::styled("p", styles::key_style()),
                    Span::styled(":paste  ", styles::help_style()),
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

            let marker = if is_selected {
                marker_active
            } else {
                " "
            };
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

    fn render_clipboard_panel(&self, f: &mut Frame, area: Rect) {
        let count = self.clipboard.len();

        // Panel size — wider, with room for header
        let panel_width = 42u16.min(area.width.saturating_sub(2));
        let panel_height = (count as u16 + 4).min(area.height).max(5);

        // Bottom-right corner of the panel area
        let x = area.x + area.width.saturating_sub(panel_width);
        let y = area.y + area.height.saturating_sub(panel_height);

        let panel_area = Rect { x, y, width: panel_width, height: panel_height };

        // Outer block with rounded border
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

        // Header row: "  Name ...  Side"
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

        // Header underline
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

        // File list
        let list_start_y = inner.y + 2;
        let visible_count = (inner.height as usize).saturating_sub(2); // header(2)
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

            // Row background for cursor
            if is_cursor {
                f.render_widget(
                    Block::default().style(Style::default().bg(Color::Rgb(30, 50, 70))),
                    Rect { x: inner.x, y: y_pos, width: inner.width, height: 1 },
                );
            }

            // Cursor indicator
            let cursor_mark = if is_cursor { "\u{25B8}" } else { " " };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    cursor_mark,
                    if is_cursor {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ))),
                Rect { x: inner.x, y: y_pos, width: 1, height: 1 },
            );

            // Icon
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    icon,
                    Style::default(),
                ))),
                Rect { x: inner.x + 1, y: y_pos, width: 2, height: 1 },
            );

            // Name
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

            // Side badge [L] / [R]
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
