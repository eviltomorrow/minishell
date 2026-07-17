use minishell_ssh::sftp::FileEntry;

pub struct TreeEntry {
    pub entry: FileEntry,
    pub depth: usize,
}
