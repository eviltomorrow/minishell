# SFTP File Browser — Design Spec

Date: 2026-07-16

## Overview

Add a TUI-based SFTP file browser to the minishell project, allowing users to browse remote directories, upload/download files, delete, and rename files/directories via a split-pane interface.

## Architecture

### Modules

```
minishell-ssh/sftp.rs    — SFTP operations (list, upload, download, delete, rename)
minishell-tui/filebrowser.rs — TUI file browser component
```

### Integration

- `app.rs` imports `filebrowser` module
- `AppState` gains `filebrowser: Option<FileBrowserState>`
- Main loop: when `filebrowser.is_some()`, dispatch `update`/`view` to filebrowser instead of main table
- `KeyCode::Char('b')` in normal mode → initialize file browser for selected machine

## SFTP Module (`minishell-ssh/sftp.rs`)

### Session Management

Extract `create_session(config: &ConnectConfig) -> Result<ssh2::Session>` from existing `try_session()`:
- TCP connect, SSH handshake, auth (key/password/agent)
- No PTY, no shell, no raw mode
- Returns authenticated `ssh2::Session`

### Functions

```rust
pub fn list_dir(session: &ssh2::Session, path: &str) -> Result<Vec<FileEntry>>
pub fn download(session: &ssh2::Session, remote: &str, local: &str) -> Result<()>
pub fn upload(session: &ssh2::Session, local: &str, remote: &str) -> Result<()>
pub fn delete_file(session: &ssh2::Session, path: &str) -> Result<()>
pub fn delete_dir(session: &ssh2::Session, path: &str) -> Result<()>
pub fn rename(session: &ssh2::Session, old: &str, new: &str) -> Result<()>
```

- `FileEntry` re-exported to TUI: `{ name, is_dir, size, modified }`
- Uses `ssh2::Sftp` for all operations
- File I/O uses 64KB buffer chunks

## File Browser TUI (`minishell-tui/filebrowser.rs`)

### Data Structures

```rust
enum Side { Local, Remote }

pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
}

struct PanelState {
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
}

pub struct FileBrowserState {
    session: ssh2::Session,
    local: PanelState,
    remote: PanelState,
    active_side: Side,
    machine: Machine,
    status: String,
    pending: Option<PendingAction>,
    confirm_delete: Option<(Side, String)>,
    rename_input: Option<String>,
}
```

### Threading

Blocking SFTP operations run in `std::thread::spawn`. Results communicated via `std::sync::mpsc`:

```rust
enum PendingAction {
    Listing { rx: Receiver<Result<Vec<FileEntry>>> },
    Transferring { rx: Receiver<Result<()>> },
}
```

### UI Layout (ratatui)

```
┌──────────────────────────────────────────────────────────┐  ← header bar
│  user@host:port                    /remote/path           │     (machine info + remote path)
├───────────────────────────┬──────────────────────────────┤
│  LOCAL: /home/user        │  REMOTE: /var/www            │  ← panel headers
│                           │                              │
│  [DIR]  ..                │  [DIR]  ..                   │
│  [DIR]  Downloads         │  [DIR]  html                 │
│  [DIR]  Documents         │  [FILE] index.html      ◄    │  ← active marker
│  [FILE] test.txt     ◄    │  [FILE] config.php           │
│  [FILE] script.sh         │  [FILE] style.css            │
├───────────────────────────┴──────────────────────────────┤  ← status bar
│  Tab:切栏  ↑↓:移动  u:上传  d:下载  x:删除  r:重命名    │     (context-sensitive)
│  q:退出  Enter:进入  Backspace:上级                      │
└──────────────────────────────────────────────────────────┘
```

- **Two equal-width columns** with `Constraint::Ratio(1, 1)`
- Each column is a `Table` widget with columns: type icon, name, size, modified
- Active side highlighted with different background color
- Bottom status bar shows available keybindings (changes based on state)

### Keybindings

| Key | Normal | Confirm Delete | Rename |
|-----|--------|----------------|--------|
| `↑`/`↓` | Move cursor | - | - |
| `Enter` | Enter directory | - | Confirm rename |
| `Backspace` / `h` | Parent directory | - | - |
| `Tab` | Toggle active side | - | - |
| `u` | Upload (local→remote) | - | - |
| `d` | Download (remote→local) | - | - |
| `x` | Delete prompt | - | - |
| `y`/`n` | - | Confirm / Cancel | - |
| `r` | Rename prompt | - | - |
| `Esc` | - | Cancel delete | Cancel rename |
| `q` | Exit file browser | - | - |

### Upload/Download Flow

1. User selects a file on one side, presses `u` (upload) or `d` (download)
2. If target side has a file with same name → confirm overwrite dialog
3. Spawn transfer thread, update status to `Uploading filename... (45%)`
4. On completion, refresh the target panel's directory listing
5. On error, show error in status bar

### Error Handling

- Connection error during browser session → show error in status bar, disable remote panel navigation
- File I/O error (permission denied, disk full, etc.) → status bar message, no UI crash
- Path traversal: reject paths containing `..` segments in user input (rename, upload path)

## CLI Future Extension (not in scope for TUI phase)

```bash
minishell push <query> <local> [remote]
minishell pull <query> <remote> [local]
```

Will reuse `create_session()` from the SFTP module. Not part of this implementation.

## Testing

- `minishell-ssh`: unit tests for SFTP operations against a local SSH server (or mocked)
- `minishell-tui`: unit tests for `FileBrowserState` logic (cursor, navigation, panel switching)
- Manual testing: connect to real SSH target, verify all operations

## Out of Scope (for this version)

- Progress bar rendering (status text only)
- Multiple file selection
- File editing/viewing inside TUI
- Tab completion for local file paths
- Sorting columns (always alphabetical, dirs first)
