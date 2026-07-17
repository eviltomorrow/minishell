use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;

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
        todo!()
    }
    
    pub fn move_cursor(&mut self, delta: isize, visible_rows: usize) {
        todo!()
    }
    
    pub fn cursor_first(&mut self) {
        todo!()
    }
    
    pub fn cursor_last(&mut self, visible_rows: usize) {
        todo!()
    }
}
