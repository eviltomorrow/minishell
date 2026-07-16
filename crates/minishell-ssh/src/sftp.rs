use std::io::{Read, Write};
use std::path::Path;
use anyhow::{Result, Context};
use ssh2::Sftp;

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

pub fn list_dir(sftp: &Sftp, path: &str) -> Result<Vec<FileEntry>> {
    let entries_raw = sftp.readdir(Path::new(path))
        .with_context(|| format!("Failed to read directory '{}'", path))?;

    let mut entries: Vec<FileEntry> = entries_raw.into_iter()
        .filter(|(name, _)| name != "." && name != "..")
        .map(|(name, stat)| {
            let name = name.to_string_lossy().to_string();
            FileEntry {
                name,
                is_dir: stat.is_dir(),
                size: stat.size.unwrap_or(0),
                modified: format_modified(stat.mtime),
            }
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(entries)
}

pub fn upload_file(sftp: &Sftp, local_path: &Path, remote_path: &str) -> Result<()> {
    let mut local_file = std::fs::File::open(local_path)
        .with_context(|| format!("Failed to open local file '{}'", local_path.display()))?;

    let mut remote_file = sftp.create(Path::new(remote_path))
        .with_context(|| format!("Failed to create remote file '{}'", remote_path))?;

    let mut buf = [0u8; 65536];
    loop {
        let n = local_file.read(&mut buf)?;
        if n == 0 { break; }
        remote_file.write_all(&buf[..n])?;
    }
    remote_file.close()?;
    Ok(())
}

pub fn download_file(sftp: &Sftp, remote_path: &str, local_path: &Path) -> Result<()> {
    let mut remote_file = sftp.open(Path::new(remote_path))
        .with_context(|| format!("Failed to open remote file '{}'", remote_path))?;

    let mut local_file = std::fs::File::create(local_path)
        .with_context(|| format!("Failed to create local file '{}'", local_path.display()))?;

    let mut buf = [0u8; 65536];
    loop {
        let n = remote_file.read(&mut buf)?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n])?;
    }
    Ok(())
}

pub fn remove_file(sftp: &Sftp, path: &str) -> Result<()> {
    sftp.unlink(Path::new(path))
        .with_context(|| format!("Failed to delete file '{}'", path))
}

pub fn remove_dir(sftp: &Sftp, path: &str) -> Result<()> {
    sftp.rmdir(Path::new(path))
        .with_context(|| format!("Failed to remove directory '{}'", path))
}

pub fn rename_item(sftp: &Sftp, old_path: &str, new_path: &str) -> Result<()> {
    sftp.rename(Path::new(old_path), Path::new(new_path), None)
        .with_context(|| format!("Failed to rename '{}' to '{}'", old_path, new_path))
}

pub fn format_modified(mtime: Option<u64>) -> String {
    match mtime {
        Some(secs) => {
            let ts = secs as i64;
            unsafe {
                let mut tm: libc::tm = std::mem::zeroed();
                libc::localtime_r(&ts, &mut tm);
                format!("{:04}-{:02}-{:02} {:02}:{:02}",
                    tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
                    tm.tm_hour, tm.tm_min)
            }
        }
        None => String::new(),
    }
}
