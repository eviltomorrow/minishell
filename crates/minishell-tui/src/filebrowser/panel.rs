use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;
use super::tree::TreeEntry;

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Local,
    Remote,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Local => "LOCAL",
            Side::Remote => "REMOTE",
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(n: usize) -> Vec<FileEntry> {
        (0..n).map(|i| FileEntry {
            name: format!("file{}", i),
            is_dir: i % 2 == 0,
            size: i as u64 * 100,
            modified: String::new(),
            perm: String::new(),
        }).collect()
    }

    fn make_panel(n: usize) -> PanelState {
        let mut panel = PanelState::new(PathBuf::from("/"));
        panel.entries = make_entries(n);
        panel.tree_entries = panel.entries.iter()
            .map(|e| TreeEntry { entry: e.clone(), depth: 0 })
            .collect();
        panel
    }

    #[test]
    fn test_cursor_move_down() {
        let mut panel = make_panel(5);
        assert_eq!(panel.cursor, 0);
        panel.move_cursor(1, 10);
        assert_eq!(panel.cursor, 1);
    }

    #[test]
    fn test_cursor_move_up() {
        let mut panel = make_panel(5);
        panel.cursor = 3;
        panel.move_cursor(-1, 10);
        assert_eq!(panel.cursor, 2);
    }

    #[test]
    fn test_cursor_bounds() {
        let mut panel = make_panel(3);
        panel.move_cursor(-1, 10);
        assert_eq!(panel.cursor, 0);
        panel.move_cursor(10, 10);
        assert_eq!(panel.cursor, 2);
    }

    #[test]
    fn test_empty_entries() {
        let mut panel = PanelState::new(PathBuf::from("/"));
        panel.move_cursor(1, 10);
        assert_eq!(panel.cursor, 0);
    }

    #[test]
    fn test_cursor_first() {
        let mut panel = make_panel(5);
        panel.cursor = 3;
        panel.cursor_first();
        assert_eq!(panel.cursor, 0);
        assert_eq!(panel.scroll_offset, 0);
    }

    #[test]
    fn test_cursor_last() {
        let mut panel = make_panel(5);
        panel.cursor_last(10);
        assert_eq!(panel.cursor, 4);
    }
}
