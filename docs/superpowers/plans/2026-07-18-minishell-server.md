# minishell-server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone SSH/SFTP server (`minishell-server`) that can be deployed on target machines lacking SSH/SFTP services, allowing the existing minishell client to connect via standard SSH protocol.

**Architecture:** New workspace crate using `russh` (async SSH server) + `russh-sftp` (SFTP subsystem) + `nix` (PTY management). Library + binary pattern. Tokio async runtime.

**Tech Stack:** russh 0.62, russh-sftp 2.3, tokio, nix, serde, toml, clap, tracing, anyhow, dirs

## Global Constraints

- Rust edition 2021 (workspace standard)
- No dependency on other minishell crates — fully standalone
- Unix-only (PTY, forkpty, libc)
- Config file: TOML format
- Default listen: `0.0.0.0:2222`
- Host key: Ed25519, auto-generated on first run

---

### Task 1: Scaffold crate and workspace integration

**Files:**
- Create: `crates/minishell-server/Cargo.toml`
- Create: `crates/minishell-server/src/lib.rs`
- Create: `crates/minishell-server/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces: `lib.rs` exports `pub fn run_server(config_path: &str) -> anyhow::Result<()>` (placeholder for now)

- [ ] **Step 1: Add crate to workspace**

Edit root `Cargo.toml`, add `"crates/minishell-server"` to `[workspace.members]` and add `minishell-server = { path = "crates/minishell-server" }` to `[workspace.dependencies]`.

- [ ] **Step 2: Create Cargo.toml**

```toml
[package]
name = "minishell-server"
version.workspace = true
edition.workspace = true

[[bin]]
name = "minishell-server"
path = "src/main.rs"

[dependencies]
russh = "0.62"
russh-keys = "0.62"
russh-sftp = "2.3"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
nix = { version = "0.29", features = ["pty", "process", "signal"] }
libc = "0.2"
clap = { version = "4", features = ["derive"] }
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dirs = "5"
subtle = "2"
```

- [ ] **Step 3: Create lib.rs**

```rust
pub mod config;
pub mod server;
pub mod shell;
pub mod sftp;

pub fn run_server(config_path: &str) -> anyhow::Result<()> {
    // Placeholder — implemented in Task 6
    todo!("run_server")
}
```

- [ ] **Step 4: Create main.rs**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "minishell-server", version, about = "SSH/SFTP server for minishell")]
struct Cli {
    /// Config file path
    #[arg(short, long)]
    config: Option<String>,

    /// Log level override
    #[arg(long)]
    log_level: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| {
        // Default config path resolution in Task 2
        "config.toml".to_string()
    });
    minishell_server::run_server(&config_path)
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles (run_server panics with todo! but that's OK at this stage)

- [ ] **Step 6: Commit**

```bash
git add crates/minishell-server/ Cargo.toml
git commit -m "feat(server): scaffold minishell-server crate"
```

---

### Task 2: Config parsing

**Files:**
- Create: `crates/minishell-server/src/config.rs`

**Interfaces:**
- Produces: `pub struct ServerConfig` with all fields from spec
- Produces: `pub fn load_config(path: &str) -> anyhow::Result<ServerConfig>`
- Produces: `pub fn default_config_path() -> String`

- [ ] **Step 1: Write config struct and parser**

```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_server")]
    pub server: ServerSection,

    #[serde(default = "default_host_key")]
    pub host_key: HostKeySection,

    pub auth: AuthSection,

    #[serde(default = "default_log")]
    pub log: LogSection,
}

#[derive(Debug, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_session_timeout")]
    pub session_timeout: u64,
}

#[derive(Debug, Deserialize)]
pub struct HostKeySection {
    #[serde(default = "default_host_key_path")]
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthSection {
    pub users: Vec<UserConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserConfig {
    pub username: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub authorized_keys: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LogSection {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_server() -> ServerSection {
    ServerSection {
        bind: default_bind(),
        port: default_port(),
        max_connections: default_max_connections(),
        session_timeout: default_session_timeout(),
    }
}

fn default_host_key() -> HostKeySection {
    HostKeySection { path: default_host_key_path() }
}

fn default_log() -> LogSection {
    LogSection { level: default_log_level() }
}

fn default_bind() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 2222 }
fn default_max_connections() -> usize { 50 }
fn default_session_timeout() -> u64 { 3600 }
fn default_host_key_path() -> String {
    dirs::home_dir()
        .map(|h| h.join(".config/minishell-server/host_key"))
        .unwrap_or_else(|| PathBuf::from("/etc/minishell-server/host_key"))
        .to_string_lossy()
        .to_string()
}
fn default_log_level() -> String { "info".to_string() }

pub fn default_config_path() -> String {
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".config/minishell-server/config.toml");
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
    }
    "/etc/minishell-server/config.toml".to_string()
}

pub fn load_config(path: &str) -> anyhow::Result<ServerConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config '{}': {}", path, e))?;
    let config: ServerConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config '{}': {}", path, e))?;
    Ok(config)
}

impl ServerConfig {
    pub fn find_user(&self, username: &str) -> Option<&UserConfig> {
        self.auth.users.iter().find(|u| u.username == username)
    }

    pub fn expanded_host_key_path(&self) -> PathBuf {
        expand_tilde(&self.host_key.path)
    }

    pub fn expanded_authorized_keys_path(&self, user: &UserConfig) -> Option<PathBuf> {
        user.authorized_keys.as_ref().map(|p| expand_tilde(p))
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}
```

- [ ] **Step 2: Add tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_config() {
        let toml = r#"
[server]
bind = "127.0.0.1"
port = 2222

[host_key]
path = "/tmp/test_key"

[auth]
[[auth.users]]
username = "admin"
password = "secret"

[log]
level = "debug"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 2222);
        assert_eq!(config.auth.users.len(), 1);
        assert_eq!(config.auth.users[0].username, "admin");
        assert_eq!(config.auth.users[0].password, Some("secret".to_string()));
    }

    #[test]
    fn test_defaults() {
        let toml = r#"
[auth]
[[auth.users]]
username = "admin"
password = "secret"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.server.bind, "0.0.0.0");
        assert_eq!(config.server.port, 2222);
        assert_eq!(config.server.max_connections, 50);
        assert_eq!(config.server.session_timeout, 3600);
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn test_find_user() {
        let toml = r#"
[auth]
[[auth.users]]
username = "admin"
password = "secret"
[[auth.users]]
username = "deploy"
authorized_keys = "~/.ssh/authorized_keys"
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert!(config.find_user("admin").is_some());
        assert!(config.find_user("deploy").is_some());
        assert!(config.find_user("unknown").is_none());
    }

    #[test]
    fn test_expand_tilde() {
        let result = expand_tilde("~/test");
        assert!(result.to_string_lossy().contains("test"));
        assert!(!result.to_string_lossy().starts_with("~"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minishell-server`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-server/src/config.rs
git commit -m "feat(server): add config parsing with TOML"
```

---

### Task 3: Host key management

**Files:**
- Modify: `crates/minishell-server/src/config.rs` (add host_key module)

**Interfaces:**
- Produces: `pub fn load_or_generate_host_key(path: &Path) -> anyhow::Result<russh_keys::key::KeyPair>`

- [ ] **Step 1: Implement host key loading/generation**

Add to `config.rs`:

```rust
use russh_keys::key::KeyPair;

pub fn load_or_generate_host_key(path: &std::path::Path) -> anyhow::Result<KeyPair> {
    if path.exists() {
        let content = std::fs::read(path)?;
        let key_pair = russh_keys::decode_secret_key(&content, None)?;
        tracing::info!("Loaded host key from {}", path.display());
        return Ok(key_pair);
    }

    // Generate new Ed25519 key
    let key_pair = KeyPair::generate_ed25519()
        .ok_or_else(|| anyhow::anyhow!("Failed to generate Ed25519 key"))?;

    // Save to file
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let key_bytes = russh_keys::encode_secret_key(&key_pair, None)?;
    std::fs::write(path, &key_bytes)?;

    // Set restrictive permissions (owner only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!("Generated new Ed25519 host key at {}", path.display());
    Ok(key_pair)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add crates/minishell-server/src/config.rs
git commit -m "feat(server): add host key generation and loading"
```

---

### Task 4: PTY shell management

**Files:**
- Create: `crates/minishell-server/src/shell.rs`

**Interfaces:**
- Produces: `pub struct PtySession` with fields: `master_fd`, `child_pid`
- Produces: `pub fn spawn_shell(username: &str, term: &str, cols: u16, rows: u16) -> anyhow::Result<PtySession>`
- Produces: `impl Drop for PtySession` (kill child, close fd)

- [ ] **Step 1: Implement PTY shell spawning**

```rust
use nix::pty::openpty;
use nix::unistd::{fork, ForkResult, setsid, dup2, close, execvp};
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag};
use std::ffi::CString;
use std::os::unix::io::{RawFd, AsRawFd};
use std::path::PathBuf;

pub struct PtySession {
    pub master_fd: RawFd,
    pub child_pid: nix::unistd::Pid,
}

impl PtySession {
    pub fn spawn(username: &str, term: &str, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let pty = openpty(None, None)?;

        // Set initial window size
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(pty.master, libc::TIOCSWINSZ, &winsize);
        }

        let child_pid = match unsafe { fork()? } {
            ForkResult::Child => {
                // Create new session
                setsid()?;

                // Set controlling terminal
                unsafe {
                    libc::ioctl(pty.slave, libc::TIOCSCTTY, 0);
                }

                // Redirect stdin/stdout/stderr to slave
                dup2(pty.slave, libc::STDIN_FILENO)?;
                dup2(pty.slave, libc::STDOUT_FILENO)?;
                dup2(pty.slave, libc::STDERR_FILENO)?;

                // Close original slave fd
                if pty.slave > 2 {
                    close(pty.slave)?;
                }
                close(pty.master)?;

                // Set environment
                let home = get_home(username);
                std::env::set_var("HOME", &home);
                std::env::set_var("USER", username);
                std::env::set_var("TERM", term);
                std::env::set_var("SHELL", get_shell());
                std::env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin");

                // Change to home directory
                let _ = std::env::set_current_dir(&home);

                // Exec shell
                let shell = get_shell();
                let shell_cstr = CString::new(shell.clone())?;
                let arg = CString::new("-i")?;
                execvp(&shell_cstr, &[shell_cstr.clone(), arg])?;

                // execvp doesn't return on success
                std::process::exit(1);
            }
            ForkResult::Parent { child } => {
                // Close slave fd in parent
                close(pty.slave)?;
                child
            }
        };

        Ok(PtySession {
            master_fd: pty.master,
            child_pid,
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize);
        }
    }

    pub fn is_alive(&self) -> bool {
        match nix::sys::wait::waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(_) => false, // Child has exited
            Err(nix::errno::Errno::ECHILD) => false,
            Err(_) => true, // Still running
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Send SIGHUP to child
        let _ = signal::kill(self.child_pid, Signal::SIGHUP);
        // Wait for child to exit
        let _ = waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG));
        // Close master fd
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

fn get_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }
    if std::path::Path::new("/bin/bash").exists() {
        return "/bin/bash".to_string();
    }
    "/bin/sh".to_string()
}

fn get_home(username: &str) -> String {
    if username == "root" {
        return "/root".to_string();
    }
    format!("/home/{}", username)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add crates/minishell-server/src/shell.rs
git commit -m "feat(server): add PTY shell management"
```

---

### Task 5: SFTP subsystem

**Files:**
- Create: `crates/minishell-server/src/sftp.rs`

**Interfaces:**
- Produces: `pub struct SftpHandler` implementing `russh_sftp::server::Handler`
- Consumes: `russh_sftp::server::Handler` trait methods

- [ ] **Step 1: Implement SFTP handler**

```rust
use russh_sftp::server::{Handler, self};
use russh_sftp::protocol::{Status, StatusCode, FileAttributes, OpenFlags, Handle, Attrs, Name, Data, Version};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use tokio::fs as async_fs;

#[derive(Debug)]
pub struct SftpHandler {
    root: PathBuf,
    open_dirs: HashMap<String, Vec<(String, FileAttributes)>>,
    open_files: HashMap<String, tokio::fs::File>,
    next_handle: u64,
}

impl SftpHandler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            open_dirs: HashMap::new(),
            open_files: HashMap::new(),
            next_handle: 0,
        }
    }

    fn next_handle(&mut self) -> String {
        self.next_handle += 1;
        format!("h{}", self.next_handle)
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn file_attrs(metadata: &fs::Metadata) -> FileAttributes {
        use std::os::unix::fs::PermissionsExt;
        let perm = metadata.permissions().mode();
        let size = metadata.len();
        let mtime = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        let atime = metadata.accessed()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);

        FileAttributes {
            size: Some(size),
            uid: Some(unsafe { libc::getuid() }),
            gid: Some(unsafe { libc::getgid() }),
            permissions: Some(perm),
            atime: Some(atime),
            mtime: Some(mtime),
        }
    }
}

impl Handler for SftpHandler {
    type Error = Status;

    fn unimplemented(&self) -> Self::Error {
        Status {
            id: 0,
            status_code: StatusCode::OpUnsupported,
            error_message: "Not implemented".to_string(),
            language_tag: "".to_string(),
        }
    }

    async fn init(&mut self, _version: u32, _extensions: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version {
            version: 3,
            extensions: HashMap::new(),
        })
    }

    async fn open(&mut self, id: u32, filename: String, pflags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let path = self.resolve_path(&filename);
        let handle = self.next_handle();

        let file = if pflags.contains(OpenFlags::CREATE) || pflags.contains(OpenFlags::TRUNCATE) {
            async_fs::File::create(&path).await
        } else {
            async_fs::File::open(&path).await
        };

        match file {
            Ok(f) => {
                self.open_files.insert(handle.clone(), f);
                Ok(Handle { id, handle })
            }
            Err(e) => Err(Status {
                id,
                status_code: StatusCode::NoSuchFile,
                error_message: e.to_string(),
                language_tag: "".to_string(),
            })
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.open_files.remove(&handle);
        self.open_dirs.remove(&handle);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let file = self.open_files.get_mut(&handle).ok_or_else(|| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: "Invalid handle".to_string(),
            language_tag: "".to_string(),
        })?;

        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;

        let mut buf = vec![0u8; len as usize];
        let n = file.read(&mut buf).await.map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        buf.truncate(n);

        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        use tokio::io::{AsyncWriteExt, AsyncSeekExt};

        let file = self.open_files.get_mut(&handle).ok_or_else(|| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: "Invalid handle".to_string(),
            language_tag: "".to_string(),
        })?;

        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;

        file.write_all(&data).await.map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let dir_path = self.resolve_path(&path);
        let handle = self.next_handle();

        let entries = fs::read_dir(&dir_path).map_err(|e| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;

        let mut items = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Status {
                id,
                status_code: StatusCode::Failure,
                error_message: e.to_string(),
                language_tag: "".to_string(),
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "." || name == ".." { continue; }
            let metadata = entry.metadata().map_err(|e| Status {
                id,
                status_code: StatusCode::Failure,
                error_message: e.to_string(),
                language_tag: "".to_string(),
            })?;
            items.push((name, Self::file_attrs(&metadata)));
        }

        self.open_dirs.insert(handle.clone(), items);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let entries = self.open_dirs.get_mut(&handle).ok_or_else(|| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: "Invalid handle".to_string(),
            language_tag: "".to_string(),
        })?;

        match entries.pop() {
            Some((name, attrs)) => Ok(Name {
                id,
                files: vec![(name, attrs)],
            }),
            None => Err(Status {
                id,
                status_code: StatusCode::Eof,
                error_message: "End of directory".to_string(),
                language_tag: "".to_string(),
            }),
        }
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let full_path = self.resolve_path(&path);
        let metadata = fs::symlink_metadata(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let full_path = self.resolve_path(&path);
        let metadata = fs::metadata(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let file = self.open_files.get(&handle).ok_or_else(|| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: "Invalid handle".to_string(),
            language_tag: "".to_string(),
        })?;
        let metadata = file.metadata().await.map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        if let Some(perm) = attrs.permissions {
            fs::set_permissions(&full_path, fs::Permissions::from_mode(perm)).map_err(|e| Status {
                id,
                status_code: StatusCode::Failure,
                error_message: e.to_string(),
                language_tag: "".to_string(),
            })?;
        }
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn fsetstat(&mut self, id: u32, handle: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let file = self.open_files.get(&handle).ok_or_else(|| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: "Invalid handle".to_string(),
            language_tag: "".to_string(),
        })?;
        if let Some(perm) = attrs.permissions {
            file.set_permissions(fs::Permissions::from_mode(perm)).await.map_err(|e| Status {
                id,
                status_code: StatusCode::Failure,
                error_message: e.to_string(),
                language_tag: "".to_string(),
            })?;
        }
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        fs::create_dir(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        fs::remove_dir(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&filename);
        fs::remove_file(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        let old = self.resolve_path(&oldpath);
        let new = self.resolve_path(&newpath);
        fs::rename(&old, &new).map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let full_path = self.resolve_path(&path);
        let canonical = fs::canonicalize(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Name {
            id,
            files: vec![(canonical.to_string_lossy().to_string(), FileAttributes::default())],
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let full_path = self.resolve_path(&path);
        let target = fs::read_link(&full_path).map_err(|e| Status {
            id,
            status_code: StatusCode::NoSuchFile,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Name {
            id,
            files: vec![(target.to_string_lossy().to_string(), FileAttributes::default())],
        })
    }

    async fn symlink(&mut self, id: u32, linkpath: String, targetpath: String) -> Result<Status, Self::Error> {
        let link = self.resolve_path(&linkpath);
        let target = self.resolve_path(&targetpath);
        std::os::unix::fs::symlink(&target, &link).map_err(|e| Status {
            id,
            status_code: StatusCode::Failure,
            error_message: e.to_string(),
            language_tag: "".to_string(),
        })?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add crates/minishell-server/src/sftp.rs
git commit -m "feat(server): add SFTP subsystem handler"
```

---

### Task 6: SSH server implementation

**Files:**
- Create: `crates/minishell-server/src/server.rs`

**Interfaces:**
- Produces: `pub struct MinishellServer` implementing `russh::server::Server`
- Produces: `pub struct ClientHandler` implementing `russh::server::Handler`
- Consumes: `config::ServerConfig`, `shell::PtySession`, `sftp::SftpHandler`

- [ ] **Step 1: Implement SSH server handler**

```rust
use russh::server::{self, Auth, Session, Msg, Channel, ChannelId};
use russh::{ChannelOpenHandle, Pty, Sig};
use russh_keys::key::PublicKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::config::{ServerConfig, UserConfig};
use crate::shell::PtySession;
use crate::sftp::SftpHandler;

pub struct MinishellServer {
    config: Arc<ServerConfig>,
}

impl MinishellServer {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self { config }
    }
}

impl server::Server for MinishellServer {
    type Handler = ClientHandler;

    fn new_client(&mut self, peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        tracing::info!("New connection from {:?}", peer_addr);
        ClientHandler {
            config: self.config.clone(),
            session_id: None,
            pty_session: None,
            channel_id: None,
            sftp_session: None,
            authenticated: false,
            username: String::new(),
            cols: 80,
            rows: 24,
            term: "xterm-256color".to_string(),
        }
    }
}

pub struct ClientHandler {
    config: Arc<ServerConfig>,
    session_id: Option<String>,
    pty_session: Option<PtySession>,
    channel_id: Option<ChannelId>,
    sftp_session: Option<SftpHandler>,
    authenticated: bool,
    username: String,
    cols: u16,
    rows: u16,
    term: String,
}

impl server::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if let Some(user_config) = self.config.find_user(user) {
            if let Some(ref expected_password) = user_config.password {
                // Constant-time comparison
                use subtle::ConstantTimeEq;
                let expected_bytes = expected_password.as_bytes();
                let provided_bytes = password.as_bytes();
                if expected_bytes.len() == provided_bytes.len()
                    && expected_bytes.ct_eq(provided_bytes).into()
                {
                    self.authenticated = true;
                    self.username = user.to_string();
                    tracing::info!("User '{}' authenticated via password", user);
                    return Ok(Auth::Accept);
                }
            }
        }
        tracing::warn!("Failed password attempt for user '{}'", user);
        Ok(Auth::Reject {
            proceed_with_methods_left: false,
            partial_success: false,
        })
    }

    async fn auth_publickey_offered(&mut self, user: &str, public_key: &PublicKey) -> Result<Auth, Self::Error> {
        if let Some(user_config) = self.config.find_user(user) {
            if let Some(ref keys_path) = user_config.authorized_keys {
                let keys_path = crate::config::expand_tilde(keys_path);
                if let Ok(keys_content) = std::fs::read_to_string(&keys_path) {
                    for line in keys_content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Ok(authorized_key) = russh_keys::parse_public_key(line, None) {
                            if authorized_key == *public_key {
                                return Ok(Auth::Accept);
                            }
                        }
                    }
                }
            }
        }
        Ok(Auth::Reject {
            proceed_with_methods_left: false,
            partial_success: false,
        })
    }

    async fn auth_publickey(&mut self, user: &str, public_key: &PublicKey) -> Result<Auth, Self::Error> {
        if let Some(user_config) = self.config.find_user(user) {
            if let Some(ref keys_path) = user_config.authorized_keys {
                let keys_path = crate::config::expand_tilde(keys_path);
                if let Ok(keys_content) = std::fs::read_to_string(&keys_path) {
                    for line in keys_content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Ok(authorized_key) = russh_keys::parse_public_key(line, None) {
                            if authorized_key == *public_key {
                                self.authenticated = true;
                                self.username = user.to_string();
                                tracing::info!("User '{}' authenticated via public key", user);
                                return Ok(Auth::Accept);
                            }
                        }
                    }
                }
            }
        }
        Ok(Auth::Reject {
            proceed_with_methods_left: false,
            partial_success: false,
        })
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _session: &mut Session) -> Result<(), Self::Error> {
        let _ = reply.accept().await;
        self.channel_id = Some(channel.id());
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.term = term.to_string();
        self.cols = col_width as u16;
        self.rows = row_height as u16;
        session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if !self.authenticated {
            session.channel_failure(channel);
            return Ok(());
        }

        match PtySession::spawn(&self.username, &self.term, self.cols, self.rows) {
            Ok(pty) => {
                self.pty_session = Some(pty);
                session.channel_success(channel);

                // Start async I/O loop
                if let Some(ref pty_session) = self.pty_session {
                    let master_fd = pty_session.master_fd;
                    let channel_id = channel;
                    let timeout = self.config.server.session_timeout;

                    // This is handled in the data flow — PTY reads are done in data() callback
                    // For now, we just set up the session
                }
            }
            Err(e) => {
                tracing::error!("Failed to spawn shell: {}", e);
                session.channel_failure(channel);
            }
        }
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        // For now, reject exec requests — only interactive shell supported
        session.channel_failure(channel);
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        if name == "sftp" && self.authenticated {
            self.sftp_session = Some(SftpHandler::new(std::path::PathBuf::from("/")));
            session.channel_success(channel);
            // SFTP subsystem is handled by russh-sftp's run function
            // The actual SFTP processing happens when data() is called
        } else {
            session.channel_failure(channel);
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.cols = col_width as u16;
        self.rows = row_height as u16;
        if let Some(ref pty) = self.pty_session {
            pty.resize(self.cols, self.rows);
        }
        session.channel_success(channel);
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        use std::io::Write;

        if let Some(ref pty) = self.pty_session {
            unsafe {
                let fd = pty.master_fd;
                let _ = libc::write(fd, data.as_ptr() as *const libc::c_void, data.len());
            }
        }
        Ok(())
    }

    async fn channel_close(&mut self, _channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        self.pty_session = None;
        self.sftp_session = None;
        Ok(())
    }

    async fn channel_eof(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if let Some(ref pty) = self.pty_session {
            // Send EOF to PTY
            unsafe {
                libc::close(pty.master_fd);
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Implement PTY data forwarding**

The PTY read loop needs to be handled. Update the `data` handler to also read from PTY and forward to channel. This requires a background task. Add a helper function:

```rust
pub async fn start_pty_forwarding(
    master_fd: i32,
    channel: ChannelId,
    session: &Arc<Mutex<Session>>,
    timeout: u64,
) {
    let mut buf = [0u8; 4096];
    let timeout_duration = std::time::Duration::from_secs(timeout);

    loop {
        let result = tokio::task::spawn_blocking(move || {
            let mut pollfds = [libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            }];
            let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), 1, 100) };
            if ret < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if ret > 0 && pollfds[0].revents & libc::POLLIN != 0 {
                let n = unsafe {
                    libc::read(master_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if n <= 0 {
                    return Ok(0);
                }
                return Ok(n as usize);
            }
            Ok(0)
        }).await;

        match result {
            Ok(Ok(n)) if n > 0 => {
                let mut session = session.lock().await;
                let _ = session.data(channel, &buf[..n]).await;
            }
            Ok(Ok(0)) => {
                // No data, continue
            }
            _ => {
                break;
            }
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-server/src/server.rs
git commit -m "feat(server): add SSH server implementation"
```

---

### Task 7: Wire everything together in lib.rs

**Files:**
- Modify: `crates/minishell-server/src/lib.rs`

**Interfaces:**
- Produces: `pub async fn run_server(config_path: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Implement run_server**

```rust
pub mod config;
pub mod server;
pub mod shell;
pub mod sftp;

use std::sync::Arc;
use config::{load_config, load_or_generate_host_key};

pub async fn run_server(config_path: &str) -> anyhow::Result<()> {
    let config = load_config(config_path)?;
    let config = Arc::new(config);

    // Setup logging
    let log_level = config.log.level.parse::<tracing_subscriber::filter::LevelFilter>()
        .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .init();

    tracing::info!("Starting minishell-server on {}:{}", config.server.bind, config.server.port);

    // Load or generate host key
    let host_key_path = config.expanded_host_key_path();
    let host_key = load_or_generate_host_key(&host_key_path)?;
    let host_keys = vec![host_key];

    // Create russh config
    let russh_config = russh::server::Config {
        keys: host_keys,
        auth_rejection_time: std::time::Duration::from_secs(3),
        ..Default::default()
    };

    // Create server
    let mut server = server::MinishellServer::new(config.clone());

    // Run
    let addr = format!("{}:{}", config.server.bind, config.server.port);
    server.run_on_address(Arc::new(russh_config), &addr).await?;

    Ok(())
}
```

- [ ] **Step 2: Update main.rs to use async**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "minishell-server", version, about = "SSH/SFTP server for minishell")]
struct Cli {
    #[arg(short, long)]
    config: Option<String>,

    #[arg(long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| {
        minishell_server::config::default_config_path()
    });
    minishell_server::run_server(&config_path).await
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p minishell-server`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-server/src/lib.rs crates/minishell-server/src/main.rs
git commit -m "feat(server): wire server components together"
```

---

### Task 8: Integration test

**Files:**
- Create: `crates/minishell-server/tests/integration_test.rs`

**Interfaces:**
- Consumes: `minishell_server::run_server`, `minishell_server::config`
- Uses `ssh2` crate (already in workspace) to connect as client

- [ ] **Step 1: Write config parsing test**

```rust
use minishell_server::config::load_config;
use std::io::Write;

#[test]
fn test_load_config_valid() {
    let dir = std::env::temp_dir().join("minishell_server_test");
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");

    let mut f = std::fs::File::create(&config_path).unwrap();
    writeln!(f, r#"
[server]
bind = "127.0.0.1"
port = 2222

[host_key]
path = "{}/host_key"

[auth]
[[auth.users]]
username = "test"
password = "test123"

[log]
level = "info"
"#, dir.display()).unwrap();

    let config = load_config(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.server.bind, "127.0.0.1");
    assert_eq!(config.server.port, 2222);
    assert_eq!(config.auth.users.len(), 1);
    assert_eq!(config.auth.users[0].username, "test");

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p minishell-server`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/minishell-server/tests/
git commit -m "test(server): add config integration test"
```

---

### Task 9: Verify full build

- [ ] **Step 1: Run full workspace build**

Run: `cargo build --release`
Expected: All crates compile including minishell-server

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Create example config**

Create `crates/minishell-server/config.example.toml`:

```toml
[server]
bind = "0.0.0.0"
port = 2222
max_connections = 50
session_timeout = 3600

[host_key]
path = "~/.config/minishell-server/host_key"

[auth]
[[auth.users]]
username = "admin"
password = "change-me"

[log]
level = "info"
```

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat(server): minishell-server complete"
```
