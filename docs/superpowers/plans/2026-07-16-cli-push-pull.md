# CLI Push / Pull Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `minishell push` and `minishell pull` CLI commands with SCP-compatible semantics, recursive directory transfer, progress output, and permission preservation.

**Architecture:** Add recursive transfer and permission helpers to `sftp.rs`, then wire CLI subcommands in `main.rs`. Path resolution (file vs dir, trailing slash semantics, SCP-style conflict detection) lives in the CLI layer; sftp.rs provides low-level recursive walk + per-file callbacks.

**Tech Stack:** rust, ssh2, clap, anyhow

## Global Constraints

- `#[cfg(unix)]` for local permission operations
- Permission failure → warning, not error
- Single-file failure → continue, report count at end
- Progress callback reused from file browser (`&dyn Fn(u64, u64)`) for single-file, new `TransferProgress` struct for recursive

---

### Task 1: Add helpers to sftp.rs

**Files:**
- Modify: `crates/minishell-ssh/src/sftp.rs`

**Interfaces:**
- Produces:
  - `pub fn mkdir_p(sftp: &Sftp, path: &str) -> Result<()>`
  - `pub fn set_perm_remote(sftp: &Sftp, path: &str, mode: u32) -> Result<()>`
  - `pub fn set_perm_local(path: &Path, mode: u32) -> Result<()>`  `#[cfg(unix)]`
  - `pub struct TransferProgress { pub file_name: String, pub bytes_written: u64, pub total_bytes: u64, pub file_index: usize, pub total_files: usize }`
  - `pub fn upload_recursive(sftp: &Sftp, local: &Path, remote: &str, progress: &dyn Fn(&TransferProgress)) -> Result<Vec<String>>` — returns list of errors
  - `pub fn download_recursive(sftp: &Sftp, remote: &str, local: &Path, progress: &dyn Fn(&TransferProgress)) -> Result<Vec<String>>` — returns list of errors

- [ ] **Step 1: Add TransferProgress struct after FileEntry**

```rust
pub struct TransferProgress {
    pub file_name: String,
    pub bytes_written: u64,
    pub total_bytes: u64,
    pub file_index: usize,
    pub total_files: usize,
}
```

- [ ] **Step 2: Add mkdir_p**

```rust
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
        if let Err(_) = sftp.stat(Path::new(&cur)) {
            sftp.mkdir(Path::new(&cur), 0o755)
                .with_context(|| format!("Failed to create remote dir '{}'", cur))?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Add set_perm_remote**

```rust
pub fn set_perm_remote(sftp: &Sftp, path: &str, mode: u32) -> Result<()> {
    let mut fstat = ssh2::FileStat::new();
    fstat.perm = Some(mode);
    sftp.setstat(Path::new(path), fstat)
        .with_context(|| format!("Failed to set permissions on '{}'", path))
}
```

- [ ] **Step 4: Add set_perm_local (cfg(unix))**

```rust
#[cfg(unix)]
pub fn set_perm_local(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("Failed to set local permissions on '{}'", path.display()))
}
```

- [ ] **Step 5: Add upload_recursive**

```rust
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

    // Create remote dir
    if let Err(e) = sftp.mkdir(Path::new(remote), 0o755) {
        errors.push(format!("mkdir {}: {}", remote, e));
        return Ok(errors);
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
            if let Err(e) = super::sftp::upload_file(sftp, &local_child, &remote_child, &cb) {
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
```

- [ ] **Step 6: Add download_recursive**

```rust
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
            if let Err(e) = super::sftp::download_file(sftp, &remote_child, &local_child, &cb) {
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
```

- [ ] **Step 7: Ensure new functions are exported from lib.rs**

Check `crates/minishell-ssh/src/lib.rs` — already has `pub mod sftp;`, so all `pub fn` in sftp.rs are automatically accessible as `minishell_ssh::sftp::*`.

- [ ] **Step 8: cargo check**

```bash
cargo check -p minishell-ssh 2>&1
```
Expected: clean build

- [ ] **Step 9: Commit**

```bash
git add crates/minishell-ssh/src/sftp.rs
git commit -m "feat(ssh): add recursive transfer, mkdir_p, and permission helpers"
```

---

### Task 2: Add Push / Pull CLI subcommands

**Files:**
- Modify: `crates/minishell-cli/src/main.rs`

**Interfaces:**
- Consumes: `minishell_ssh::sftp::{upload_file, download_file, upload_recursive, download_recursive, mkdir_p, TransferProgress}`
- Consumes: `minishell_ssh::create_session`, `minishell_ssh::ConnectConfig`

- [ ] **Step 1: Add Push / Pull variants to Commands enum**

Add to `Commands`:
```rust
/// Upload files to remote machine (SCP-style)
Push {
    /// Target machine (IP/remark/ID)
    query: String,
    /// Local source path
    local: String,
    /// Remote destination path
    remote: String,
    /// Recursive directory transfer
    #[arg(short)]
    recursive: bool,
},

/// Download files from remote machine (SCP-style)
Pull {
    /// Target machine (IP/remark/ID)
    query: String,
    /// Remote source path
    remote: String,
    /// Local destination path
    local: String,
    /// Recursive directory transfer
    #[arg(short)]
    recursive: bool,
},
```

- [ ] **Step 2: Add path resolution helper**

```rust
enum PathType { File, Dir, NotFound }

fn check_remote_type(sftp: &ssh2::Sftp, path: &str) -> PathType {
    match sftp.stat(Path::new(path)) {
        Ok(stat) => {
            if stat.is_dir() { PathType::Dir }
            else { PathType::File }
        }
        Err(_) => PathType::NotFound,
    }
}

fn check_local_type(path: &Path) -> PathType {
    if path.is_dir() { PathType::Dir }
    else if path.exists() { PathType::File }
    else { PathType::NotFound }
}
```

- [ ] **Step 3: Add progress output helper**

```rust
fn print_progress(p: &sftp::TransferProgress, done: bool) {
    let pct = if p.total_bytes > 0 {
        p.bytes_written * 100 / p.total_bytes
    } else { 0 };
    let size_str = format_size(p.bytes_written);
    let total_str = format_size(p.total_bytes);
    if done {
        println!("  {}  100%  {}  {}", p.file_name, total_str, size_str);
    } else {
        print!("\r  {}  {}%  {}/{}", p.file_name, pct, size_str, total_str);
    }
}
```

Reuse `format_size` from existing code (move it to shared location or re-implement in main.rs).

- [ ] **Step 4: Handle Push in run()**

```rust
Some(Commands::Push { query, local, remote, recursive }) => {
    let store = open_db()?;
    let machines = resolve_machine(&store, &query)?;
    let machine = if machines.len() == 1 {
        machines.into_iter().next().unwrap()
    } else {
        minishell_tui::select_machine(machines)?.ok_or_else(|| anyhow::anyhow!("No machine selected"))?
    };

    let config = build_config(&machine);
    let session = minishell_ssh::create_session(&config)?;
    let sftp = session.sftp()?;

    let local_path = Path::new(&local);
    let local_type = check_local_type(local_path);
    let remote_type = check_remote_type(&sftp, &remote);
    let remote_ends_with_slash = remote.ends_with('/');

    // Determine actual remote destination
    let (actual_local, actual_remote) = match (local_type, remote_type, recursive, remote_ends_with_slash) {
        (PathType::File, PathType::Dir, _, _) | (PathType::File, PathType::NotFound, _, true) => {
            // remote is dir or ends with / → place file inside
            let fname = local_path.file_name().unwrap().to_string_lossy();
            (local_path.to_path_buf(), format!("{}/{}", remote.trim_end_matches('/'), fname))
        }
        (PathType::File, _, _, _) => {
            (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
        }
        (PathType::Dir, PathType::File, _, _) => {
            anyhow::bail!("Cannot overwrite remote file '{}' with local directory '{}'", remote, local);
        }
        (PathType::Dir, PathType::Dir, true, _) => {
            // content into existing dir
            (local_path.to_path_buf(), format!("{}/{}", remote.trim_end_matches('/'), local_path.file_name().unwrap().to_string_lossy()))
        }
        (PathType::Dir, _, true, true) => {
            // tailing /: content into dir
            (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
        }
        (PathType::Dir, _, true, false) => {
            // scp creates the named dir
            (local_path.to_path_buf(), remote.trim_end_matches('/').to_string())
        }
        (PathType::Dir, _, false, _) => {
            anyhow::bail!("'{}' is a directory. Use -r to transfer directories.", local);
        }
        (PathType::NotFound, _, _, _) => {
            anyhow::bail!("Local path '{}' not found", local);
        }
    };

    // Execute
    match local_type {
        PathType::File => {
            let total = std::fs::metadata(&actual_local)?.len();
            let fname = actual_local.file_name().unwrap().to_string_lossy().to_string();
            let cb = |written, _total| {
                let p = sftp::TransferProgress {
                    file_name: fname.clone(),
                    bytes_written: written,
                    total_bytes: total,
                    file_index: 1,
                    total_files: 1,
                };
                print_progress(&p, false);
            };
            print_progress(&sftp::TransferProgress { file_name: fname.clone(), bytes_written: 0, total_bytes: total, file_index: 1, total_files: 1 }, false);
            sftp::upload_file(&sftp, &actual_local, &actual_remote, &cb)?;
            let p = sftp::TransferProgress { file_name: fname.clone(), bytes_written: total, total_bytes: total, file_index: 1, total_files: 1 };
            print_progress(&p, true);
            // Preserve permissions
            #[cfg(unix)]
            if let Ok(meta) = std::fs::metadata(&actual_local) {
                let mode = meta.permissions().mode() & 0o777;
                let _ = sftp::set_perm_remote(&sftp, &actual_remote, mode);
            }
            println!("\nTransferred: 1 file, {}", sftp::format_size(total));
        }
        PathType::Dir => {
            let total_before = 0usize;
            let total_after = std::sync::atomic::AtomicUsize::new(0);
            let total_after_ref = &total_after;
            let cb = |p: &sftp::TransferProgress| {
                total_after_ref.store(p.file_index, std::sync::atomic::Ordering::Relaxed);
                print_progress(p, false);
            };
            let errors = sftp::upload_recursive(&sftp, &actual_local, &actual_remote, &cb)?;
            println!();
            let count = total_after.load(std::sync::atomic::Ordering::Relaxed);
            if errors.is_empty() {
                println!("Transferred: {} files", count);
            } else {
                println!("Transferred: {}/{} files ({} errors)", count - errors.len(), count, errors.len());
                for e in &errors {
                    println!("  ✗  {}", e);
                }
            }
        }
        _ => unreachable!()
    }
}
```

Note: For the multi-file count tracking, use `AtomicUsize` inside the callback.
For single-file progress, use `print!("\r...")` and `println!()` on completion.

- [ ] **Step 5: Handle Pull in run()** (mirror Push logic)

Follow same pattern as Push but with reversed direction and `download_file`/`download_recursive`.

```rust
Some(Commands::Pull { query, remote, local, recursive }) => {
    let store = open_db()?;
    let machines = resolve_machine(&store, &query)?;
    let machine = if machines.len() == 1 { machines.into_iter().next().unwrap() }
        else { minishell_tui::select_machine(machines)?.ok_or_else(|| anyhow::anyhow!("No machine selected"))? };

    let config = build_config(&machine);
    let session = minishell_ssh::create_session(&config)?;
    let sftp = session.sftp()?;

    let local_path = Path::new(&local);
    let remote_type = check_remote_type(&sftp, &remote);
    let local_type = check_local_type(local_path);
    let local_ends_with_slash = local.ends_with('/');

    let (actual_remote, actual_local) = match (remote_type, local_type, recursive, local_ends_with_slash) {
        (PathType::File, PathType::Dir, _, _) | (PathType::File, PathType::NotFound, _, true) => {
            let fname = Path::new(&remote).file_name().unwrap().to_string_lossy();
            (remote, local_path.join(fname.as_ref()))
        }
        (PathType::File, _, _, _) => (remote, local_path.to_path_buf()),
        (PathType::Dir, PathType::File, _, _) => {
            anyhow::bail!("Cannot overwrite local file '{}' with remote directory '{}'", local, remote);
        }
        (PathType::Dir, PathType::Dir, true, _) => {
            let dirname = Path::new(&remote).file_name().unwrap().to_string_lossy();
            (remote, local_path.join(dirname.as_ref()))
        }
        (PathType::Dir, _, true, true) => (remote, local_path.to_path_buf()),
        (PathType::Dir, _, true, false) => (remote, local_path.to_path_buf()),
        (PathType::Dir, _, false, _) => {
            anyhow::bail!("'{}' is a remote directory. Use -r to transfer directories.", remote);
        }
        (PathType::NotFound, _, _, _) => {
            anyhow::bail!("Remote path '{}' not found", remote);
        }
    };

    match remote_type {
        PathType::File => {
            let stat = sftp.stat(Path::new(&actual_remote))?;
            let total = stat.size.unwrap_or(0);
            let fname = actual_local.file_name().unwrap().to_string_lossy().to_string();
            let cb = |written, _total| {
                let p = sftp::TransferProgress {
                    file_name: fname.clone(),
                    bytes_written: written,
                    total_bytes: total,
                    file_index: 1,
                    total_files: 1,
                };
                print_progress(&p, false);
            };
            print_progress(&sftp::TransferProgress { file_name: fname.clone(), bytes_written: 0, total_bytes: total, file_index: 1, total_files: 1 }, false);
            sftp::download_file(&sftp, &actual_remote, &actual_local, &cb)?;
            let p = sftp::TransferProgress { file_name: fname, bytes_written: total, total_bytes: total, file_index: 1, total_files: 1 };
            print_progress(&p, true);
            #[cfg(unix)]
            if let Some(perm) = stat.perm {
                let _ = sftp::set_perm_local(&actual_local, perm);
            }
            println!("\nTransferred: 1 file, {}", sftp::format_size(total));
        }
        PathType::Dir => {
            // similar to push recursive but using download_recursive
        }
        _ => unreachable!()
    }
}
```

(Pull dir case mirrors push dir case with `download_recursive` instead of `upload_recursive`.)

- [ ] **Step 6: Add resolve_machine helper**

```rust
fn resolve_machine(store: &Store, query: &str) -> Result<Vec<Machine>> {
    if let Ok(num) = query.parse::<i32>() {
        let machines = store.search("")?;
        if let Some(m) = machines.iter().find(|m| m.num == num) {
            return Ok(vec![m.clone()]);
        }
    }
    let machines = store.search(query)?;
    if machines.is_empty() {
        anyhow::bail!("No machines found matching '{}'", query);
    }
    Ok(machines)
}
```

- [ ] **Step 7: Add build_config helper** (extract from filebrowser or reuse logic)

```rust
fn build_config(machine: &Machine) -> minishell_ssh::ConnectConfig {
    minishell_ssh::ConnectConfig {
        username: machine.username.clone(),
        password: if machine.password == "-" { String::new() } else { machine.password.clone() },
        private_key_path: if machine.private_key_path == "-" { String::new() } else { machine.private_key_path.clone() },
        host: machine.effective_host().to_string(),
        port: machine.port,
        timeout: std::time::Duration::from_secs(10),
        device: machine.device.clone(),
    }
}
```

- [ ] **Step 8: Add format_size to main.rs** (copy from filebrowser.rs or import from sftp)

The existing `format_size` in `filebrowser.rs` is private. Move it to `minishell-ssh/src/sftp.rs` as `pub fn format_size(size: u64) -> String` and reuse.

- [ ] **Step 9: cargo check**

```bash
cargo check 2>&1
```

- [ ] **Step 10: Commit**

```bash
git add crates/minishell-cli/src/main.rs crates/minishell-ssh/src/sftp.rs
git commit -m "feat(cli): add push/pull subcommands for file transfer"
```

---

### Task 3: Integration test

- [ ] **Step 1: Test push single file**

```bash
cargo build --release && ./target/release/minishell push <test-machine> Cargo.toml /tmp/
```
Expected: Shows progress, file appears on remote.

- [ ] **Step 2: Test pull single file**

```bash
./target/release/minishell pull <test-machine> /tmp/Cargo.toml /tmp/test-download/
```
Expected: Shows progress, file appears locally.

- [ ] **Step 3: Test recursive push**

```bash
./target/release/minishell push -r <test-machine> crates/minishell-ssh/src/ /tmp/ssh-test/
```
Expected: Recursive transfer with progress.

- [ ] **Step 4: Test error cases**

```bash
# Push dir without -r → error
./target/release/minishell push <test-machine> crates/ /tmp/

# Push to existing file when source is dir → error
./target/release/minishell push -r <test-machine> crates/ /tmp/Cargo.toml
```
Expected: Clear error messages.

- [ ] **Step 5: Test permission preservation**

```bash
# Create file with specific permissions
echo test > /tmp/permtest && chmod 600 /tmp/permtest
./target/release/minishell push <test-machine> /tmp/permtest /tmp/
# Check remote permissions
./target/release/minishell pull <test-machine> /tmp/permtest /tmp/permtest-back
ls -la /tmp/permtest-back
```
Expected: Permissions preserved (600).
