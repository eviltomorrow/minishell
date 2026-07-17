use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;
use super::tree::TreeEntry;

pub struct PanelState {
    pub entries: Vec<FileEntry>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub current_path: PathBuf,
    pub prev_dir_name: Option<String>,
    pub tree_entries: Vec<TreeEntry>,
    pub expanded_dirs: Vec<PathBuf>,
}

impl PanelState {
    pub fn new(path: PathBuf) -> Self {
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
    
    pub fn move_cursor(&mut self, delta: isize, visible_rows: usize) {
        let len = self.tree_entries.len();
        if len == 0 {
            return;
        }
        let new = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        self.cursor = new;
        if new < self.scroll_offset {
            self.scroll_offset = new;
        } else if new >= self.scroll_offset + visible_rows {
            self.scroll_offset = new + 1 - visible_rows;
        }
    }
    
    pub fn cursor_first(&mut self) {
        if !self.tree_entries.is_empty() {
            self.cursor = 0;
            self.scroll_offset = 0;
        }
    }
    
    pub fn cursor_last(&mut self, visible_rows: usize) {
        let len = self.tree_entries.len();
        if len > 0 {
            self.cursor = len - 1;
            if self.cursor >= self.scroll_offset + visible_rows {
                self.scroll_offset = len.saturating_sub(visible_rows);
            }
        }
    }
}
