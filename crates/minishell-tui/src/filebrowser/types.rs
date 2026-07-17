use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;

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

pub struct TreeEntry {
    pub entry: FileEntry,
    pub depth: usize,
}

#[derive(Clone)]
pub struct ClipboardEntry {
    pub source_path: PathBuf,
    pub source_side: Side,
    pub name: String,
    pub is_dir: bool,
}
