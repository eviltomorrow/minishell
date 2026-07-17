use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;
use super::types::TreeEntry;

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
}
