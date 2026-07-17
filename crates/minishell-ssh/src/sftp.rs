use std::io::{Read, Write};
use std::path::Path;
use anyhow::{Result, Context};
pub use ssh2::Sftp;
pub use minishell_utils::format_size;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
    pub perm: String,
}

#[derive(Clone)]
pub struct TransferProgress {
    pub file_name: String,
    pub bytes_written: u64,
    pub total_bytes: u64,
    pub file_index: usize,
    pub total_files: usize,
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
                perm: format_perm(stat.perm, stat.is_dir()),
            }
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    Ok(entries)
}

pub fn upload_file(
    sftp: &Sftp,
    local_path: &Path,
    remote_path: &str,
    progress: &dyn Fn(u64, u64),
) -> Result<()> {
    let mut local_file = std::fs::File::open(local_path)
        .with_context(|| format!("Failed to open local file '{}'", local_path.display()))?;
    let total = local_file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut remote_file = sftp.create(Path::new(remote_path))
        .with_context(|| format!("Failed to create remote file '{}'", remote_path))?;

    let mut buf = [0u8; 65536];
    let mut written = 0u64;
    loop {
        let n = local_file.read(&mut buf)?;
        if n == 0 { break; }
        remote_file.write_all(&buf[..n])?;
        written += n as u64;
        progress(written, total);
    }
    remote_file.close()?;
    progress(total, total);
    Ok(())
}

pub fn download_file(
    sftp: &Sftp,
    remote_path: &str,
    local_path: &Path,
    progress: &dyn Fn(u64, u64),
) -> Result<()> {
    let stat = sftp.stat(Path::new(remote_path))
        .with_context(|| format!("Failed to stat remote file '{}'", remote_path))?;
    let total = stat.size.unwrap_or(0);

    let mut remote_file = sftp.open(Path::new(remote_path))
        .with_context(|| format!("Failed to open remote file '{}'", remote_path))?;

    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent dir '{}'", parent.display()))?;
    }

    let _ = std::fs::remove_file(local_path);
    let mut local_file = std::fs::File::create(local_path)
        .with_context(|| format!("Failed to create local file '{}'", local_path.display()))?;

    let mut buf = [0u8; 65536];
    let mut written = 0u64;
    loop {
        let n = remote_file.read(&mut buf)?;
        if n == 0 { break; }
        local_file.write_all(&buf[..n])?;
        written += n as u64;
        progress(written, total);
    }
    progress(total, total);
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

pub fn remove_recursive(sftp: &Sftp, path: &str) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    let entries = match sftp.readdir(Path::new(path)) {
        Ok(e) => e,
        Err(e) => { errors.push(format!("readdir {}: {}", path, e)); return Ok(errors); }
    };
    for (entry_path, stat) in entries {
        let name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if name == "." || name == ".." { continue; }
        let full = entry_path.to_string_lossy().to_string();
        if stat.is_dir() {
            let child_errors = remove_recursive(sftp, &full)?;
            errors.extend(child_errors);
        } else if stat.is_file() {
            if let Err(e) = sftp.unlink(Path::new(&full)) {
                errors.push(format!("unlink {}: {}", full, e));
            }
        }
    }
    if errors.is_empty() {
        if let Err(e) = sftp.rmdir(Path::new(path)) {
            errors.push(format!("rmdir {}: {}", path, e));
        }
    }
    Ok(errors)
}

pub fn rename_item(sftp: &Sftp, old_path: &str, new_path: &str) -> Result<()> {
    sftp.rename(Path::new(old_path), Path::new(new_path), None)
        .with_context(|| format!("Failed to rename '{}' to '{}'", old_path, new_path))
}

pub fn mkdir_p(sftp: &Sftp, path: &str) -> Result<()> {
    let mut parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if path.starts_with('/') {
        parts.insert(0, "/");
    }
    let mut cur = String::new();
    for part in parts {
        if part.is_empty() { continue; }
        if cur.is_empty() || cur == "/" {
            cur = format!("/{}", part.trim_start_matches('/'));
        } else {
            cur = format!("{}/{}", cur, part);
        }
        if sftp.stat(Path::new(&cur)).is_err() {
            sftp.mkdir(Path::new(&cur), 0o755)
                .with_context(|| format!("Failed to create remote dir '{}'", cur))?;
        }
    }
    Ok(())
}

pub fn set_perm_remote(sftp: &Sftp, path: &str, mode: u32) -> Result<()> {
    let fstat = ssh2::FileStat {
        size: None,
        uid: None,
        gid: None,
        perm: Some(mode),
        mtime: None,
        atime: None,
    };
    sftp.setstat(Path::new(path), fstat)
        .with_context(|| format!("Failed to set permissions on '{}'", path))
}

#[cfg(unix)]
pub fn set_perm_local(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set local permissions on '{}'", path.display()))
}

pub fn upload_recursive(
    sftp: &Sftp,
    local: &Path,
    remote: &str,
    progress: &dyn Fn(&TransferProgress),
) -> Result<Vec<String>> {
    let mut errors = Vec::new();

    let local_meta = match local.metadata() {
        Ok(m) => m,
        Err(e) => { return Err(e.into()); }
    };

    let remote_path = Path::new(remote);
    match sftp.stat(remote_path) {
        Ok(stat) if stat.is_dir() => {}
        _ => {
            if let Err(e) = sftp.mkdir(remote_path, 0o755) {
                errors.push(format!("mkdir {}: {}", remote, e));
                return Ok(errors);
            }
        }
    }
    #[cfg(unix)]
    if let Err(e) = set_perm_remote(sftp, remote, local_meta.permissions().mode() & 0o777) {
        errors.push(format!("perm {}: {}", remote, e));
    }

    let entries = match std::fs::read_dir(local) {
        Ok(d) => d,
        Err(e) => { errors.push(format!("read {}: {}", local.display(), e)); return Ok(errors); }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => { errors.push(format!("entry: {}", e)); continue; }
        };
        let entry_name = entry.file_name().to_string_lossy().to_string();
        let local_child = entry.path();
        let remote_child = format!("{}/{}", remote.trim_end_matches('/'), entry_name);

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let child_errors = upload_recursive(sftp, &local_child, &remote_child, progress)?;
            errors.extend(child_errors);
        } else {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => { errors.push(format!("meta {}: {}", entry_name, e)); continue; }
            };
            let total = meta.len();
            let fname = entry_name.clone();
            let idx = errors.len() + 1;
            let cb = |written: u64, _total: u64| {
                progress(&TransferProgress {
                    file_name: fname.clone(),
                    bytes_written: written,
                    total_bytes: total,
                    file_index: idx,
                    total_files: 0,
                });
            };
            if let Err(e) = upload_file(sftp, &local_child, &remote_child, &cb) {
                errors.push(format!("{}: {}", entry_name, e));
                continue;
            }
            #[cfg(unix)]
            if let Err(e) = set_perm_remote(sftp, &remote_child, meta.permissions().mode() & 0o777) {
                errors.push(format!("perm {}: {}", entry_name, e));
            }
        }
    }
    Ok(errors)
}

pub fn download_recursive(
    sftp: &Sftp,
    remote: &str,
    local: &Path,
    progress: &dyn Fn(&TransferProgress),
) -> Result<Vec<String>> {
    let mut errors = Vec::new();

    let entries = match sftp.readdir(Path::new(remote)) {
        Ok(e) => e,
        Err(e) => { errors.push(format!("readdir {}: {}", remote, e)); return Ok(errors); }
    };

    for (path, stat) in entries {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if name == "." || name == ".." { continue; }

        let local_child = local.join(&name);
        let remote_child = path.to_string_lossy().to_string();

        if stat.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&local_child) {
                errors.push(format!("mkdir {}: {}", local_child.display(), e));
                continue;
            }
            if let Some(perm) = stat.perm {
                #[cfg(unix)]
                if let Err(e) = set_perm_local(&local_child, perm) {
                    errors.push(format!("perm {}: {}", name, e));
                }
            }
            let child_errors = download_recursive(sftp, &remote_child, &local_child, progress)?;
            errors.extend(child_errors);
        } else if stat.is_file() {
            let total = stat.size.unwrap_or(0);
            let fname = name.clone();
            let idx = errors.len() + 1;
            let cb = |written: u64, _total: u64| {
                progress(&TransferProgress {
                    file_name: fname.clone(),
                    bytes_written: written,
                    total_bytes: total,
                    file_index: idx,
                    total_files: 0,
                });
            };
            if let Some(parent) = local_child.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(format!("mkdir {}: {}", parent.display(), e));
                    continue;
                }
            }
            if let Err(e) = download_file(sftp, &remote_child, &local_child, &cb) {
                errors.push(format!("{}: {}", name, e));
                continue;
            }
            if let Some(perm) = stat.perm {
                #[cfg(unix)]
                if let Err(e) = set_perm_local(&local_child, perm) {
                    errors.push(format!("perm {}: {}", name, e));
                }
            }
        }
    }
    Ok(errors)
}

pub fn format_perm(perm: Option<u32>, is_dir: bool) -> String {
    match perm {
        Some(mode) => {
            let t = if is_dir { 'd' } else { '-' };
            let r = |bit: u32, c: char| if mode & bit != 0 { c } else { '-' };
            format!("{}{}{}{}{}{}{}{}{}{}",
                t,
                r(0o400, 'r'), r(0o200, 'w'), r(0o100, 'x'),
                r(0o040, 'r'), r(0o020, 'w'), r(0o010, 'x'),
                r(0o004, 'r'), r(0o002, 'w'), r(0o001, 'x'),
            )
        }
        None => String::new(),
    }
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
