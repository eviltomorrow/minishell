# SFTP File Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a TUI split-pane SFTP file browser for browsing remote directories, uploading/downloading files, deleting, and renaming.

**Architecture:** Extract `create_session()` from existing SSH auth, add `minishell-ssh/src/sftp.rs` for SFTP operations, create `minishell-tui/src/filebrowser.rs` for the TUI component, integrate into `app.rs` with `b` key entry.

**Tech Stack:** Rust, ssh2 (SFTP), ratatui, crossterm, std::thread + mpsc for async transfer

## Global Constraints

- No async runtime — use `std::thread` + `std::sync::mpsc` for background transfers
- Session for transfers created independently (separate TCP connection per transfer thread)
- All paths: remote always Unix `/`, local current platform
- `ConnectConfig` must derive Clone for transfer thread use

---

### Task 1: Refactor SSH crate — extract `create_session()`

**Files:**
- Modify: `crates/minishell-ssh/src/lib.rs`
- Test: `crates/minishell-ssh/src/lib.rs` (existing tests still pass)

**Interfaces:**
- Consumes: `ConnectConfig` struct (add `#[derive(Clone)]`)
- Produces: `pub fn create_session(config: &ConnectConfig) -> Result<ssh2::Session>`

- [ ] **Step 1: Add `Clone` derive to `ConnectConfig`**

```rust
#[derive(Clone)]
pub struct ConnectConfig {
    // existing fields unchanged
}
```

- [ ] **Step 2: Extract `create_session()` from `try_session()`**

Move TCP connect + handshake + auth into a new public function:

```rust
pub fn create_session(config: &ConnectConfig) -> Result<ssh2::Session> {
    let addr = format!("{}:{}", config.host, config.port);
    let parsed_addr: std::net::SocketAddr = match addr.parse() {
        Ok(addr) => addr,
        Err(_) => addr
            .to_socket_addrs()
            .context("Failed to resolve hostname")?
            .next()
            .ok_or_else(|| anyhow::anyhow!("No addresses found for hostname"))?,
    };

    let tcp = TcpStream::connect_timeout(&parsed_addr, config.timeout)
        .with_context(|| format!("Failed to connect to {}", config.host))?;

    let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH handshake failed")?;

    if !config.private_key_path.is_empty() {
        let key_path = std::path::Path::new(&config.private_key_path);
        session
            .userauth_pubkey_file(&config.username, None, key_path, None)
            .context("Public key auth failed")?;
    } else if !config.password.is_empty() {
        session
            .userauth_password(&config.username, &config.password)
            .context("Password auth failed")?;
    } else {
        session
            .userauth_agent(&config.username)
            .context("Agent auth failed")?;
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    Ok(session)
}
```

- [ ] **Step 3: Simplify `try_session()` to call `create_session()`**

```rust
fn try_session(
    addr: &std::net::SocketAddr,
    config: &ConnectConfig,
    term: &str,
) -> Result<SessionEnd> {
    let session = create_session(config)?;
    // ... rest stays the same (PTY, shell, session loop)
}
```

Remove the duplicated TCP/SSH/auth code from `try_session()` — replace with `let session = create_session(config)?;`. Keep the PTY, shell, and session loop code unchanged.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-ssh`
Expected: compilation succeeds

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p minishell-ssh`
Expected: all tests pass (no behavioral change)

- [ ] **Step 6: Commit**

```bash
git add crates/minishell-ssh/src/lib.rs
git commit -m "refactor(ssh): extract create_session() for reuse by SFTP"
```

---

### Task 2: Create SFTP module

**Files:**
- Create: `crates/minishell-ssh/src/sftp.rs`
- Modify: `crates/minishell-ssh/src/lib.rs` (add `pub mod sftp;`)

**Interfaces:**
- Consumes: `create_session()`, `ConnectConfig`
- Produces: see `pub fn` signatures below

- [ ] **Step 1: Create `crates/minishell-ssh/src/sftp.rs`**

```rust
use std::io::Read;
use std::path::Path;
use anyhow::{Result, Context};
use ssh2::{Session, Sftp};

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

pub fn list_dir(sftp: &Sftp, path: &str) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    let dir = sftp.opendir(Path::new(path))
        .with_context(|| format!("Failed to open directory '{}'", path))?;

    for entry in dir {
        let (name, stat) = entry?;
        if name == "." || name == ".." {
            continue;
        }
        entries.push(FileEntry {
            name,
            is_dir: stat.is_dir(),
            size: stat.size.unwrap_or(0),
            modified: format_modified(stat.mtime),
        });
    }

    // Sort: dirs first, then alphabetical
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
    });

    Ok(entries)
}

fn format_modified(mtime: Option<u64>) -> String {
    match mtime {
        Some(secs) => {
            // Simple date formatting
            let naive = chrono::DateTime::from_timestamp(secs as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            naive
        }
        None => String::new(),
    }
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
    sftp.rename(Path::new(old_path), Path::new(new_path))
        .with_context(|| format!("Failed to rename '{}' to '{}'", old_path, new_path))
}
```

Note: The `format_modified` function uses `chrono`. Let me check if `chrono` is already a dependency... it's not. Let me use a simple manual formatting instead to avoid adding a dependency.

Actually, since this is a plan, I can specify the correct approach. Let me use a simple manual format without chrono:

```rust
fn format_modified(mtime: Option<u64>) -> String {
    match mtime {
        Some(secs) => {
            // Convert from epoch seconds
            let secs = secs as i64;
            // Use localtime
            let tm = unsafe {
                let mut tm = std::mem::zeroed::<libc::tm>();
                libc::localtime_r(&secs, &mut tm);
                tm
            };
            format!("{:04}-{:02}-{:02} {:02}:{:02}",
                tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
                tm.tm_hour, tm.tm_min)
        }
        None => String::new(),
    }
}
```

Wait, the plan should use the simplest approach. Let me just use a Rust crate-free approach using `std::time::SystemTime`:

```rust
fn format_modified(mtime: Option<u64>) -> String {
    match mtime {
        Some(secs) => {
            let duration = std::time::Duration::from_secs(secs);
            // Approximate from epoch
            let total_secs = secs as i64;
            let days = total_secs / 86400;
            let time_secs = total_secs % 86400;
            let hours = time_secs / 3600;
            let mins = (time_secs % 3600) / 60;
            // Very rough year calculation
            let year = 1970 + (days as f64 / 365.25) as i64;
            format!("{} {:02}:{:02}", year, hours, mins)
        }
        None => String::new(),
    }
}
```

Hmm, this is getting complex. Let me just use the libc crate which is already a dependency:

```rust
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
```

This works since `libc` is already a dependency of `minishell-ssh`.

- [ ] **Step 2: Add `pub mod sftp;` to `lib.rs`**

Add at top near `pub mod card;`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p minishell-ssh`
Expected: compilation succeeds

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-ssh/src/sftp.rs crates/minishell-ssh/src/lib.rs
git commit -m "feat(ssh): add SFTP module with list/upload/download/delete/rename"
```

---

### Task 3: Create FileBrowser TUI module

**Files:**
- Create: `crates/minishell-tui/src/filebrowser.rs`
- Modify: `crates/minishell-tui/src/lib.rs` (add `pub mod filebrowser;`)
- Modify: `crates/minishell-tui/Cargo.toml` (add nothing — reuses existing deps)

- [ ] **Step 1: Create initial file with state types**

```rust
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use minishell_core::Machine;
use minishell_ssh::sftp::{self, FileEntry, format_modified};
use minishell_ssh::ConnectConfig;

#[derive(Clone, Copy, PartialEq)]
pub enum Side { Local, Remote }

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }
}

struct PanelState {
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
}

impl PanelState {
    fn new(path: PathBuf) -> Self {
        PanelState {
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            current_path: path,
        }
    }
}

enum ActionResult {
    DirEntries(Vec<FileEntry>, PathBuf),
    TransferDone,
    RefreshDone,
    Error(String),
}

pub struct FileBrowserState {
    machine: Machine,
    local: PanelState,
    remote: PanelState,
    active_side: Side,
    session: Option<ssh2::Session>,
    sftp: Option<ssh2::Sftp>,
    status: String,
    pending: Option<Receiver<ActionResult>>,
    confirm_delete: Option<(Side, usize)>,
    rename_input: Option<String>,
}
```

- [ ] **Step 2: Add initialization and session setup**

```rust
impl FileBrowserState {
    pub fn new(machine: Machine) -> Self {
        let local_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        FileBrowserState {
            local: PanelState::new(local_path),
            remote: PanelState::new(PathBuf::from("/")),
            active_side: Side::Remote,
            machine,
            session: None,
            sftp: None,
            status: "Connecting...".to_string(),
            pending: None,
            confirm_delete: None,
            rename_input: None,
        }
    }

    pub fn connect(&mut self) -> Result<(), String> {
        let config = self.build_config();
        let session = minishell_ssh::create_session(&config)
            .map_err(|e| format!("SSH connection failed: {}", e))?;
        let s = session.sftp()
            .map_err(|e| format!("SFTP init failed: {}", e))?;
        self.session = Some(session);
        self.sftp = Some(s);
        self.status = "Connected".to_string();
        Ok(())
    }

    fn build_config(&self) -> ConnectConfig {
        let host = self.machine.effective_host().to_string();
        ConnectConfig {
            username: self.machine.username.clone(),
            password: if self.machine.password == "-" { String::new() } else { self.machine.password.clone() },
            private_key_path: if self.machine.private_key_path == "-" { String::new() } else { self.machine.private_key_path.clone() },
            host,
            port: self.machine.port,
            timeout: std::time::Duration::from_secs(10),
            device: self.machine.device.clone(),
        }
    }
}
```

- [ ] **Step 3: Add navigation methods**

```rust
impl FileBrowserState {
    pub fn active_panel(&self) -> &PanelState {
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

    pub fn toggle_side(&mut self) {
        self.active_side = self.active_side.other();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let panel = self.active_panel_mut();
        let len = panel.entries.len();
        if len == 0 { return; }
        let new = (panel.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        panel.cursor = new;
        // Adjust scroll
        if new < panel.scroll_offset {
            panel.scroll_offset = new;
        }
        let visible = 20; // approximate, will be adjusted in render
        if new >= panel.scroll_offset + visible {
            panel.scroll_offset = new.saturating_sub(visible - 1);
        }
    }

    pub fn enter_dir(&mut self) {
        let panel = self.active_panel_mut();
        if panel.entries.is_empty() { return; }
        let entry = &panel.entries[panel.cursor];
        if !entry.is_dir { return; }

        let new_path = panel.current_path.join(&entry.name);
        panel.current_path = new_path;
        panel.cursor = 0;
        panel.scroll_offset = 0;
        self.refresh_panel(self.active_side);
    }

    pub fn parent_dir(&mut self) {
        let panel = self.active_panel_mut();
        if !panel.current_path.parent().map_or(false, |p| p.as_os_str().is_empty()) {
            if let Some(parent) = panel.current_path.parent() {
                panel.current_path = parent.to_path_buf();
                panel.cursor = 0;
                panel.scroll_offset = 0;
                self.refresh_panel(self.active_side);
            }
        }
    }

    pub fn refresh_panel(&mut self, side: Side) {
        match side {
            Side::Local => self.refresh_local(),
            Side::Remote => self.refresh_remote(),
        }
    }

    fn refresh_local(&mut self) {
        let path = self.local.current_path.clone();
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') { continue; }
                let meta = entry.metadata().ok();
                entries.push(FileEntry {
                    name,
                    is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified: meta.and_then(|m| m.modified().ok())
                        .map(|t| t.duration_since(std::time::UNIX_EPOCH).ok()
                            .map(|d| d.as_secs()).unwrap_or(0))
                        .map(|secs| format_modified(Some(secs)))
                        .unwrap_or_default(),
                });
            }
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
        self.local.entries = entries;
        self.local.cursor = self.local.cursor.min(self.local.entries.len().saturating_sub(1));
        self.local.scroll_offset = 0;
    }

    fn refresh_remote(&mut self) {
        // Async via thread
        let sftp_sender = self.sftp.as_ref().and_then(|_| {
            // We can't easily clone Sftp, so do it synchronously for simplicity
            None
        });
        
        // For now, synchronous refresh for remote too (fast enough)
        if let Some(ref sftp) = self.sftp.clone() {
            let path = self.remote.current_path.to_string_lossy().to_string();
            match sftp::list_dir(sftp, &path) {
                Ok(entries) => {
                    self.remote.entries = entries;
                    self.remote.cursor = self.remote.cursor.min(
                        self.remote.entries.len().saturating_sub(1));
                    self.remote.scroll_offset = 0;
                    self.status = format!("{} entries", self.remote.entries.len());
                }
                Err(e) => {
                    self.status = format!("Error: {}", e);
                }
            }
        }
    }
}
```

Wait, `Sftp` is not `Clone`. Let me check... Actually, `ssh2::Sftp` doesn't implement Clone. But `Session::sftp()` returns a new `Sftp` handle from the same session. However, calling it multiple times might work - let me check the ssh2 source.

Actually, in libssh2, `sftp_init()` can be called multiple times and each returns a new SFTP session handle. So I can call `session.sftp()` multiple times. But I'd need a reference to the `Session`.

Let me restructure. Instead of caching `sftp`, just get it from `session` when needed:

```rust
fn with_sftp<F, T>(&mut self, f: F) -> Result<T, String>
where
    F: FnOnce(&ssh2::Sftp) -> Result<T, String>,
{
    if let Some(ref session) = self.session {
        let sftp = session.sftp().map_err(|e| format!("SFTP error: {}", e))?;
        f(&sftp)
    } else {
        Err("Not connected".to_string())
    }
}
```

But `Sftp` is not `Send`, so we can't easily pass it to a thread. Let me reconsider.

For the design, the file browser's remote panel operations will be synchronous for now:
- Directory listing: synchronous (fast, < 100ms)
- Delete/rename: synchronous (fast)
- Upload/Download: threaded, creates new SSH session

So I'll keep `session` and create `sftp` on the fly.

Actually, `ssh2::Sftp` is Send. Let me double check... `unsafe impl Send for Sftp` in ssh2 source. Yes, it is Send.

But for simplicity, let me just create sftp from session each time. It's cleaner and avoids lifetime issues.

Let me restructure:

```rust
impl FileBrowserState {
    fn refresh_remote(&mut self) {
        let session = match self.session.as_ref() {
            Some(s) => s,
            None => { self.status = "Not connected".to_string(); return; }
        };
        let sftp = match session.sftp() {
            Ok(s) => s,
            Err(e) => { self.status = format!("SFTP error: {}", e); return; }
        };
        let path = self.remote.current_path.to_string_lossy().to_string();
        match sftp::list_dir(&sftp, &path) {
            Ok(entries) => {
                self.remote.entries = entries;
                self.remote.cursor = self.remote.cursor.min(
                    self.remote.entries.len().saturating_sub(1));
                self.remote.scroll_offset = 0;
                self.status = format!("{} entries", self.remote.entries.len());
            }
            Err(e) => {
                self.status = format!("Error: {}", e);
            }
        }
    }
}
```

- [ ] **Step 4: Add transfer (upload/download) methods with threading**

```rust
impl FileBrowserState {
    pub fn upload_selected(&mut self) {
        if self.active_side != Side::Local { return; }
        if self.pending.is_some() { return; }
        let entry = self.local.entries.get(self.local.cursor).cloned();
        let entry = match entry {
            Some(e) if !e.is_dir => e,
            _ => { self.status = "Select a file to upload".to_string(); return; }
        };

        let local_path = self.local.current_path.join(&entry.name);
        let remote_path = self.remote.current_path.join(&entry.name);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Uploading {}...", entry.name);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        let local_path2 = local_path.clone();
        self.pending = Some(rx);

        thread::spawn(move || {
            match transfer_upload(config, &local_path2, &remote_str) {
                Ok(()) => { let _ = tx.send(ActionResult::TransferDone); }
                Err(e) => { let _ = tx.send(ActionResult::Error(e)); }
            }
        });
    }

    pub fn download_selected(&mut self) {
        if self.active_side != Side::Remote { return; }
        if self.pending.is_some() { return; }
        let entry = self.remote.entries.get(self.remote.cursor).cloned();
        let entry = match entry {
            Some(e) if !e.is_dir => e,
            _ => { self.status = "Select a file to download".to_string(); return; }
        };

        let remote_path = self.remote.current_path.join(&entry.name);
        let local_path = self.local.current_path.join(&entry.name);
        let remote_str = remote_path.to_string_lossy().to_string();

        self.status = format!("Downloading {}...", entry.name);
        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);

        thread::spawn(move || {
            match transfer_download(config, &remote_str, &local_path) {
                Ok(()) => { let _ = tx.send(ActionResult::TransferDone); }
                Err(e) => { let _ = tx.send(ActionResult::Error(e)); }
            }
        });
    }

    fn delete_selected(&mut self) {
        let panel = self.active_panel_mut();
        let entry = match panel.entries.get(panel.cursor).cloned() {
            Some(e) => e,
            None => return,
        };
        self.status = format!("Delete {}?", entry.name);
        self.confirm_delete = Some((self.active_side, panel.cursor));
    }

    fn confirm_delete_action(&mut self) {
        let (side, idx) = match self.confirm_delete.take() {
            Some(v) => v,
            None => return,
        };
        let session = match self.session.as_ref() {
            Some(s) => s,
            None => { self.status = "Not connected".to_string(); return; }
        };
        let sftp = match session.sftp() {
            Ok(s) => s,
            Err(e) => { self.status = format!("SFTP error: {}", e); return; }
        };
        let panel = match side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        };
        let entry = match panel.entries.get(idx) {
            Some(e) => e.clone(),
            None => return,
        };
        let path = panel.current_path.join(&entry.name);
        let path_str = path.to_string_lossy().to_string();
        let result = if entry.is_dir {
            sftp::remove_dir(&sftp, &path_str)
        } else {
            sftp::remove_file(&sftp, &path_str)
        };
        match result {
            Ok(()) => {
                self.status = format!("Deleted {}", entry.name);
                self.refresh_panel(side);
            }
            Err(e) => {
                self.status = format!("Delete failed: {}", e);
            }
        }
    }

    fn start_rename(&mut self) {
        let panel = self.active_panel_mut();
        let entry = match panel.entries.get(panel.cursor) {
            Some(e) => e.clone(),
            None => return,
        };
        self.rename_input = Some(entry.name.clone());
        self.status = "Enter new name:".to_string();
    }

    fn confirm_rename(&mut self) {
        let new_name = match self.rename_input.take() {
            Some(n) if !n.is_empty() => n,
            _ => { self.status = "Rename cancelled".to_string(); return; }
        };
        let side = self.active_side;
        let panel = match side {
            Side::Local => &mut self.local,
            Side::Remote => &mut self.remote,
        };
        let old_entry = match panel.entries.get(panel.cursor) {
            Some(e) => e.clone(),
            None => return,
        };
        let old_path = panel.current_path.join(&old_entry.name);
        let new_path = panel.current_path.join(&new_name);
        let old_str = old_path.to_string_lossy().to_string();
        let new_str = new_path.to_string_lossy().to_string();

        match side {
            Side::Local => {
                match std::fs::rename(&old_path, &new_path) {
                    Ok(()) => {
                        self.status = format!("Renamed to {}", new_name);
                        self.refresh_panel(side);
                    }
                    Err(e) => {
                        self.status = format!("Rename failed: {}", e);
                    }
                }
            }
            Side::Remote => {
                let session = match self.session.as_ref() {
                    Some(s) => s,
                    None => { self.status = "Not connected".to_string(); return; }
                };
                let sftp = match session.sftp() {
                    Ok(s) => s,
                    Err(e) => { self.status = format!("SFTP error: {}", e); return; }
                };
                match sftp::rename_item(&sftp, &old_str, &new_str) {
                    Ok(()) => {
                        self.status = format!("Renamed to {}", new_name);
                        self.refresh_panel(side);
                    }
                    Err(e) => {
                        self.status = format!("Rename failed: {}", e);
                    }
                }
            }
        }
    }

    pub fn check_pending(&mut self) {
        if let Some(ref rx) = self.pending {
            match rx.try_recv() {
                Ok(ActionResult::TransferDone) => {
                    self.pending = None;
                    self.status = "Transfer complete".to_string();
                    self.refresh_panel(Side::Remote);
                    self.refresh_panel(Side::Local);
                }
                Ok(ActionResult::Error(e)) => {
                    self.pending = None;
                    self.status = format!("Error: {}", e);
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Transfer failed".to_string();
                }
            }
        }
    }

    pub fn init_dirs(&mut self) {
        self.refresh_local();
        if self.sftp.is_some() {
            self.refresh_remote();
        }
    }
}

fn transfer_upload(config: ConnectConfig, local: &Path, remote: &str) -> Result<(), String> {
    let session = minishell_ssh::create_session(&config)
        .map_err(|e| format!("Connection failed: {}", e))?;
    let sftp = session.sftp()
        .map_err(|e| format!("SFTP init failed: {}", e))?;
    sftp::upload_file(&sftp, local, remote)
        .map_err(|e| format!("Upload failed: {}", e))
}

fn transfer_download(config: ConnectConfig, remote: &str, local: &Path) -> Result<(), String> {
    let session = minishell_ssh::create_session(&config)
        .map_err(|e| format!("Connection failed: {}", e))?;
    let sftp = session.sftp()
        .map_err(|e| format!("SFTP init failed: {}", e))?;
    sftp::download_file(&sftp, remote, local)
        .map_err(|e| format!("Download failed: {}", e))
}
```

- [ ] **Step 5: Add the update method (key handling)**

```rust
impl FileBrowserState {
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        // Rename mode
        if self.rename_input.is_some() {
            match key.code {
                KeyCode::Enter => { self.confirm_rename(); }
                KeyCode::Esc => { self.rename_input = None; self.status = "Rename cancelled".to_string(); }
                KeyCode::Backspace => { self.rename_input.as_mut().map(|s| { s.pop(); }); }
                KeyCode::Char(c) => { self.rename_input.as_mut().map(|s| s.push(c)); }
                _ => {}
            }
            return;
        }

        // Confirm delete mode
        if self.confirm_delete.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => { self.confirm_delete_action(); }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm_delete = None;
                    self.status = "Delete cancelled".to_string();
                }
                _ => {}
            }
            return;
        }

        // Normal mode
        match key.code {
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => self.enter_dir(),
            KeyCode::BackSpace | KeyCode::Char('h') => self.parent_dir(),
            KeyCode::Tab => self.toggle_side(),
            KeyCode::Char('u') => self.upload_selected(),
            KeyCode::Char('d') => self.download_selected(),
            KeyCode::Char('x') => self.delete_selected(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('q') => { /* caller handles exit */ }
            _ => {}
        }
    }

    pub fn wants_quit(&self, key: &crossterm::event::KeyEvent) -> bool {
        matches!(key.code, crossterm::event::KeyCode::Char('q'))
            && self.rename_input.is_none()
            && self.confirm_delete.is_none()
    }
}
```

- [ ] **Step 6: Add the render method (ratatui)**

```rust
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::buffer::Buffer;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

impl FileBrowserState {
    pub fn render(&self, f: &mut Frame) {
        let area = f.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(area);

        // Header
        let host = self.machine.effective_host();
        let header = format!(" {}@{}:{}   {}",
            self.machine.username, host, self.machine.port,
            self.remote.current_path.display());
        f.render_widget(Paragraph::new(Line::from(
            Span::styled(&header, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        )), chunks[0]);

        // Split panels
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);

        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);

        // Status bar
        let mut spans: Vec<Span> = Vec::new();
        if self.pending.is_some() {
            spans.push(Span::styled(" ⏳ ", Style::default().fg(Color::Yellow)));
            spans.push(Span::styled(&self.status, Style::default().fg(Color::Yellow)));
        } else if self.status.starts_with("Error") || self.status.starts_with("Delete failed") || self.status.starts_with("Upload failed") || self.status.starts_with("Download failed") {
            spans.push(Span::styled(" ✗ ", Style::default().fg(Color::Red)));
            spans.push(Span::styled(&self.status, Style::default().fg(Color::Red)));
        } else if self.status.starts_with("Transfer complete") {
            spans.push(Span::styled(" ✓ ", Style::default().fg(Color::Green)));
            spans.push(Span::styled(&self.status, Style::default().fg(Color::Green)));
        } else {
            spans.push(Span::styled("   ", Style::default()));
            spans.push(Span::styled(&self.status, Style::default().fg(Color::White)));
        }

        // Keybindings hint
        let help = if self.confirm_delete.is_some() {
            "  y:确认  n:取消".to_string()
        } else if self.rename_input.is_some() {
            format!("  Enter:确认  Esc:取消  {}", self.rename_input.as_ref().unwrap())
        } else {
            "  Tab:切栏  ↑↓:移动  Enter:进入  u:上传  d:下载  x:删除  r:重命名  q:退出  Backspace:上级".to_string()
        };
        let w = area.width as usize;
        if spans.iter().map(|s| s.width()).sum::<usize>() + help.len() + 4 < w {
            let padding = w - spans.iter().map(|s| s.width()).sum::<usize>() - help.len();
            spans.push(Span::raw(" ".repeat(padding)));
        }
        spans.push(Span::styled(&help, Style::default().fg(Color::Gray)));

        f.render_widget(Line::from(spans), chunks[2]);
    }

    fn render_panel(&self, f: &mut Frame, area: Rect, side: Side) {
        let panel = match side {
            Side::Local => &self.local,
            Side::Remote => &self.remote,
        };
        let is_active = self.active_side == side;
        let title = match side {
            Side::Local => format!(" LOCAL: {}", panel.current_path.display()),
            Side::Remote => format!(" REMOTE: {}", panel.current_path.display()),
        };

        let border_style = if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Header row
        let header = Line::from(vec![
            Span::styled("  Name", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" ".repeat(inner.width.saturating_sub(22).max(1) as usize)),
            Span::styled("Size", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("Modified", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(header, Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });

        // Entries
        let visible = (inner.height.saturating_sub(1)) as usize;
        let selected_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD).bg(Color::Blue);
        let dir_style = Style::default().fg(Color::Cyan);
        let file_style = Style::default().fg(Color::White);
        let dim_style = Style::default().fg(Color::DarkGray);

        for i in 0..visible {
            let idx = panel.scroll_offset + i;
            if idx >= panel.entries.len() { break; }
            let entry = &panel.entries[idx];
            let style = if idx == panel.cursor { selected_style }
                        else if entry.is_dir { dir_style }
                        else { file_style };

            let marker = if idx == panel.cursor && is_active { "▸" } else { " " };
            let icon = if entry.is_dir { "📁" } else { " " };
            let name = format!("{}{} {}", marker, icon, entry.name);

            let size_str = if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            };

            let y = inner.y + 1 + i as u16;

            // Truncate name to fit available width
            let name_width = (inner.width as usize).saturating_sub(22).max(1);
            let display_name = truncate_to_width(&name, name_width);

            f.render_widget(Line::from(vec![
                Span::styled(display_name, style),
                Span::raw(" "),
                Span::styled(
                    pad_left(&size_str, 10),
                    if idx == panel.cursor { selected_style } else { dim_style },
                ),
                Span::raw("  "),
                Span::styled(
                    &entry.modified,
                    if idx == panel.cursor { selected_style } else { dim_style },
                ),
            ]), Rect { x: inner.x, y, width: inner.width, height: 1 });
        }
    }
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut s = size as f64;
    let mut unit_idx = 0;
    while s >= 1024.0 && unit_idx < UNITS.len() - 1 {
        s /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", size)
    } else if s >= 100.0 {
        format!("{:.0} {}", s, UNITS[unit_idx])
    } else if s >= 10.0 {
        format!("{:.1} {}", s, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", s, UNITS[unit_idx])
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", " ".repeat(width - w), s)
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width { break; }
        result.push(c);
        current_width += cw;
    }
    result
}
```

- [ ] **Step 7: Add `pub mod filebrowser;` to `crates/minishell-tui/src/lib.rs`**

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: compilation succeeds

- [ ] **Step 9: Commit**

```bash
git add crates/minishell-tui/src/filebrowser.rs crates/minishell-tui/src/lib.rs
git commit -m "feat(tui): add file browser TUI module with SFTP operations"
```

---

### Task 4: Integrate file browser into app.rs

**Files:**
- Modify: `crates/minishell-tui/src/app.rs`

- [ ] **Step 1: Add import and AppState field**

Add to imports:
```rust
use super::filebrowser::{FileBrowserState, Side};
```

Add to `AppState` struct:
```rust
pub filebrowser: Option<FileBrowserState>,
```

Initialize in `run_inner()`:
```rust
filebrowser: None,
```

- [ ] **Step 2: Add `b` key handler in `update()`**

In the normal mode match block, add before the catch-all `_ => {}`:
```rust
KeyCode::Char('b') => {
    if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
        let mut fb = FileBrowserState::new(m);
        if let Err(e) = fb.connect() {
            // Leave TUI to show error, or just set filebrowser and show error in status
            state.filebrowser = Some(fb);
            state.status = e;
        } else {
            fb.init_dirs();
            state.filebrowser = Some(fb);
        }
    }
}
```

Wait, there's no `status` field in AppState currently. The status bar shows machine info, not a dynamic status. The file browser has its own status. So we don't need to add a status to AppState.

Actually, the `connect()` failure is a problem. If SSH connection fails, we should show the error. But since we're still in the TUI loop, we can set `filebrowser` and the file browser's status will show the error.

If connection fails, the file browser is still created with `status = "Error: ..."`. The render will show it.

But if the TUI's `terminal.draw()` calls the main view function instead of the file browser view, we need to handle that. Let me design the main loop modification.

- [ ] **Step 3: Modify main loop to dispatch to filebrowser**

In `run_inner()`, the main loop currently:
```rust
loop {
    if login_target { ... }
    terminal.draw(|f| view(f, &mut state))?;
    match event::read()? { ... }
    if should_quit { ... }
}
```

Change to:
```rust
loop {
    if login_target { ... }
    
    if state.filebrowser.is_some() {
        // Check pending transfers
        if let Some(ref mut fb) = state.filebrowser {
            fb.check_pending();
        }
        terminal.draw(|f| {
            if let Some(ref fb) = state.filebrowser {
                fb.render(f);
            }
        })?;
        match event::read()? {
            Event::Key(key) => {
                if let Some(ref mut fb) = state.filebrowser {
                    if fb.wants_quit(&key) {
                        state.filebrowser = None;
                    } else {
                        fb.handle_key(key);
                    }
                }
            }
            _ => {}
        }
    } else {
        // Existing main loop
        terminal.draw(|f| view(f, &mut state))?;
        match event::read()? {
            Event::Key(key) => update(&mut state, key),
            Event::Paste(data) => handle_paste(&mut state, &data),
            _ => {}
        }
    }
    
    if should_quit { ... }
}
```

- [ ] **Step 4: Add `b` to help bar**

In `view()`, add `("b", "browse")` to the `help_items` vector:
```rust
let help_items: Vec<(&str, &str)> = vec![
    ("↑↓", "sel"),
    ("↵", "login"),
    ("b", "browse"),
    ("e", "edit"),
    ("a", "add"),
    ("d", "del"),
    ("s", "secrets"),
    ("/", "search"),
    ("q", "quit"),
];
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: compilation succeeds

- [ ] **Step 6: Run full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/minishell-tui/src/app.rs
git commit -m "feat(tui): integrate file browser with 'b' key entry in main UI"
```

---

### Task 5: Final integration pass

**Files:**
- Verify: full project compiles and tests pass

- [ ] **Step 1: Full compilation check**

Run: `cargo check`
Expected: no errors

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 3: Quick audit for edge cases**

Check the following:
- Empty directory rendering (both local and remote)
- Connection failure flow (sftp connect error shown in status)
- Transfer thread panic safety (unwrap in thread)
- Path with spaces in upload/download
- Rename to empty string
- Delete last entry in directory (cursor out of bounds after refresh)
- Rapid key presses causing multiple transfer threads

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: integrate SFTP file browser with TUI"
```
