use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::thread;
use minishell_core::Machine;
use minishell_ssh::sftp::{self, FileEntry, format_modified, format_perm};
use minishell_ssh::ConnectConfig;

pub mod types;
pub mod panel;
pub mod transfer;
pub mod render;
pub mod input;
pub mod view;

pub use types::{Side, TreeEntry, ClipboardEntry};
pub use panel::PanelState;
pub use transfer::{TransferProgressState, ActionResult, PendingTransfer};
pub use render::{HEADER_FG, ACTIVE_BORDER, INACTIVE_BORDER, DIR_FG, FILE_FG, SELECTED_BG, STATUS_OK, STATUS_ERR, DIM, HINT, render_name_and_path};

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
            status: "连接中".to_string(),
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
        if !self.status.starts_with("错误") && !self.status.starts_with("连接中") {
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

    fn cancel_transfer(&mut self) {
        if let Some(ref t) = self.pending {
            t.cancel();
        }
        self.status = "取消中...".to_string();
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
                    if !self.status.starts_with("错误") {
                        let p = self.active_panel();
                        self.status = format!("{} entries", p.entries.len());
                    }
                }
                Ok(Err(e)) => {
                    self.pending_connect = None;
                    self.status = format!("错误: {}", e);
                    self.init_dirs();
                }
                Err(TryRecvError::Empty) => {
                    if self.connect_start.elapsed() > std::time::Duration::from_secs(5) {
                        self.pending_connect = None;
                        self.status = "连接超时（5秒）".to_string();
                        self.init_dirs();
                    } else {
                        self.connecting_dots = (self.connecting_dots + 1) % 4;
                        let dots = ".".repeat(self.connecting_dots as usize);
                        self.status = format!("连接中{}", dots);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending_connect = None;
                    self.status = "错误: SSH 连接失败".to_string();
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
                    self.status = "传输失败（内部错误）".to_string();
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
                    self.status = "传输完成".to_string();
                } else {
                    self.transfer_queue.remove(0);
                    if self.transfer_queue.is_empty() {
                        self.status = "传输完成".to_string();
                    } else {
                        self.process_next_transfer();
                    }
                }
            }
            Ok(ActionResult::Error(e)) => {
                self.clear_transfer();
                self.status = format!("错误: {}", e);
                if !self.transfer_queue.is_empty() {
                    self.transfer_queue.remove(0);
                    if !self.transfer_queue.is_empty() {
                        self.process_next_transfer();
                    }
                }
            }
            Ok(ActionResult::Aborted) => {
                self.clear_transfer();
                self.transfer_queue.clear();
                self.status = "传输已取消".to_string();
                self.init_dirs();
            }
            Err(TryRecvError::Empty) => {
                if self.progress_total > 0 {
                    let pct = self.progress_current * 100 / self.progress_total;
                    self.status = format!("{} {}%", self.progress_file_name, pct);
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.clear_transfer();
                self.status = "传输失败".to_string();
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
                self.status = "未连接".to_string();
                return;
            }
        };
        let sftp = match session.sftp() {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("SFTP 错误: {}", e);
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
                self.status = format!("错误: {}", e);
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
                    "切换到 {}？清空选择 [Y/N]",
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
            self.status = "无文件选中".to_string();
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
            self.status = "选择已清空".to_string();
            return;
        }
        if self.clipboard_panel_cursor >= self.clipboard.len() {
            self.clipboard_panel_cursor = self.clipboard.len() - 1;
        }
    }

    fn paste_from_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            self.status = "无文件选中".to_string();
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
            self.status = format!("已在 {} 侧", clip_side.label());
            return;
        }
        if clip_side == Side::Remote && self.session.is_none() {
            self.status = "未连接".to_string();
            return;
        }

        self.transfer_queue = self.clipboard.drain(..).collect();
        self.clipboard_side = None;
        self.clipboard_panel_open = false;
        self.process_next_transfer();
    }

    fn start_file_transfer(&mut self, name: String, source_path: PathBuf, dest_path: PathBuf, is_dir: bool, direction: Side) {
        let (tx, progress, cancel) = self.init_transfer(name.clone());
        let src_str = source_path.to_string_lossy().to_string();
        let dest_str = dest_path.to_string_lossy().to_string();
        let done_side = direction.other();

        let verb = match direction {
            Side::Local => "Uploading",
            Side::Remote => "Downloading",
        };
        if is_dir {
            self.status = format!("{} {}/...", verb, name);
        } else {
            self.status = format!("{} {}...", verb, name);
            if direction == Side::Local {
                self.progress_total = std::fs::metadata(&source_path).map(|m| m.len()).unwrap_or(0);
            }
        }

        self.start_transfer(tx, progress, cancel, move |sftp, tx, progress, cancel| {
            if is_dir && direction == Side::Remote {
                let _ = std::fs::create_dir_all(&dest_path);
            }
            let result = match (direction, is_dir) {
                (Side::Local, true) => {
                    let cb = |p: &sftp::TransferProgress| {
                        let mut state = progress.lock().unwrap();
                        state.file_name = p.file_name.clone();
                        state.bytes = p.bytes_written;
                        state.total = p.total_bytes;
                    };
                    sftp::upload_recursive(&sftp, &source_path, &dest_str, &cb, &cancel)
                        .map_err(|e| format!("上传失败: {}", e))
                        .and_then(|errs| if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) })
                }
                (Side::Local, false) => {
                    let cb = |cur: u64, total: u64| {
                        let mut state = progress.lock().unwrap();
                        state.bytes = cur;
                        state.total = total;
                    };
                    sftp::upload_file(&sftp, &source_path, &dest_str, &cb, &cancel)
                        .map_err(|e| format!("上传失败: {}", e))
                }
                (Side::Remote, true) => {
                    let cb = |p: &sftp::TransferProgress| {
                        let mut state = progress.lock().unwrap();
                        state.file_name = p.file_name.clone();
                        state.bytes = p.bytes_written;
                        state.total = p.total_bytes;
                    };
                    sftp::download_recursive(&sftp, &src_str, &dest_path, &cb, &cancel)
                        .map_err(|e| format!("下载失败: {}", e))
                        .and_then(|errs| if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) })
                }
                (Side::Remote, false) => {
                    let cb = |cur: u64, total: u64| {
                        let mut state = progress.lock().unwrap();
                        state.bytes = cur;
                        state.total = total;
                    };
                    sftp::download_file(&sftp, &src_str, &dest_path, &cb, &cancel)
                        .map_err(|e| format!("下载失败: {}", e))
                }
            };
            match result {
                Ok(()) => { let _ = tx.send(ActionResult::TransferDone(done_side)); }
                Err(e) => { let _ = tx.send(ActionResult::Error(e)); }
            }
        });
    }

    fn process_next_transfer(&mut self) {
        let entry = match self.transfer_queue.first() {
            Some(e) => e.clone(),
            None => {
                self.status = "传输完成".to_string();
                return;
            }
        };

        let name = entry.name.clone();
        let source_path = entry.source_path.clone();
        let is_dir = entry.is_dir;
        let dest_path = self.active_panel().current_path.join(&name);
        let direction = entry.source_side;

        self.start_file_transfer(name, source_path, dest_path, is_dir, direction);
    }

    pub fn toggle_tree(&mut self) {
        if self.active_side == Side::Remote && self.session.is_none() {
            self.status = "未连接".to_string();
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
        cancel: Arc<AtomicBool>,
        f: impl FnOnce(ssh2::Sftp, mpsc::Sender<ActionResult>, Arc<Mutex<TransferProgressState>>, Arc<AtomicBool>) + Send + 'static,
    ) {
        let config = self.build_config();
        thread::spawn(move || {
            let session = match minishell_ssh::create_session(&config) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("连接失败: {}", e)));
                    return;
                }
            };
            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(ActionResult::Error(format!("SFTP 初始化失败: {}", e)));
                    return;
                }
            };
            f(sftp, tx, progress, cancel);
        });
    }

    fn init_transfer(&mut self, filename: String) -> (mpsc::Sender<ActionResult>, Arc<Mutex<TransferProgressState>>, Arc<AtomicBool>) {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(Mutex::new(TransferProgressState {
            file_name: filename.clone(),
            bytes: 0,
            total: 0,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        self.pending = Some(PendingTransfer { progress: progress.clone(), done_rx: rx, cancel: cancel.clone() });
        self.progress_file_name = filename;
        self.progress_current = 0;
        self.progress_total = 0;
        (tx, progress, cancel)
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
        let is_dir = entry.is_dir;
        self.start_file_transfer(filename, local_path, remote_path, is_dir, Side::Local);
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
        let is_dir = entry.is_dir;
        self.start_file_transfer(filename, remote_path, local_path, is_dir, Side::Remote);
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
                    self.status = "未连接".to_string();
                    return;
                }
            };
            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    self.status = format!("SFTP 错误: {}", e);
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
                self.status = format!("删除失败: {}", e);
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
        self.status = "输入新名称:".to_string();
    }

    fn confirm_rename(&mut self) {
        let new_name = match self.rename_input.take() {
            Some(n) if !n.is_empty() => n,
            _ => {
                self.status = "重命名已取消".to_string();
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
                        self.status = "未连接".to_string();
                        return;
                    }
                };
                let sftp = match session.sftp() {
                    Ok(s) => s,
                    Err(e) => {
                        self.status = format!("SFTP 错误: {}", e);
                        return;
                    }
                };
                sftp::rename_item(&sftp, &old_path.to_string_lossy(), &new_path.to_string_lossy())
                    .map_err(|e| e.to_string())
            }
        };

        match result {
            Ok(()) => {
                self.status = format!("已重命名为 {}", new_name);
                self.refresh_panel(side);
            }
            Err(e) => {
                self.status = format!("重命名失败: {}", e);
            }
        }
    }
}
