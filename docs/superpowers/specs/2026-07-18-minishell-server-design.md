# minishell-server Design Spec

## Problem

When target machines don't have SSH/SFTP services enabled, minishell cannot connect to them. We need a lightweight SSH/SFTP server that can be deployed on target machines to provide these services, allowing minishell clients to connect using the standard SSH protocol without any client-side changes.

## Overview

`minishell-server` is a new crate that implements a standalone SSH/SFTP server using the `russh` library (pure Rust, async). It is deployed on target machines and provides:

- Full SSH shell (PTY) sessions — users get an interactive shell just like a regular SSH server
- SFTP subsystem — file browsing, upload, download, delete, rename, mkdir, permissions
- Password and public key authentication
- TOML-based configuration file
- Auto-generated Ed25519 host key

The existing minishell client (`minishell-ssh`) connects to this server via the standard SSH protocol using `ssh2` — no client changes required.

## Crate Structure

```
crates/minishell-server/
├── Cargo.toml
└── src/
    ├── lib.rs      # Public API: run_server(config_path)
    ├── main.rs     # clap CLI entry point
    ├── config.rs   # TOML config deserialization
    ├── server.rs   # russh Server trait implementation
    ├── shell.rs    # PTY process management
    └── sftp.rs     # SFTP subsystem implementation
```

## Dependencies

| Crate | Purpose |
|---|---|
| `russh` + `russh-keys` | SSH server protocol implementation |
| `tokio` | Async runtime (required by russh) |
| `serde` + `toml` | Config file parsing |
| `nix` | forkpty(), PTY management, signal handling |
| `libc` | Low-level PTY fd operations |
| `clap` | CLI argument parsing |
| `anyhow` | Error handling |
| `tracing` + `tracing-subscriber` | Structured logging |
| `dirs` | Home directory resolution |

## Configuration File

### Location Priority

1. `--config <path>` CLI argument
2. `~/.config/minishell-server/config.toml`
3. `/etc/minishell-server/config.toml`

### Format

```toml
[server]
bind = "0.0.0.0"
port = 2222
max_connections = 50
session_timeout = 3600  # seconds

[host_key]
path = "~/.config/minishell-server/host_key"

[auth]
# Password-only user
[[auth.users]]
username = "admin"
password = "secret123"

# Public key user
[[auth.users]]
username = "deploy"
authorized_keys = "~/.ssh/authorized_keys"

# Both password and public key
[[auth.users]]
username = "root"
password = "rootpass"
authorized_keys = "/root/.ssh/authorized_keys"

[log]
level = "info"   # trace | debug | info | warn | error
```

### Defaults

- `server.bind`: `"0.0.0.0"`
- `server.port`: `2222`
- `server.max_connections`: `50`
- `server.session_timeout`: `3600`
- `host_key.path`: `~/.config/minishell-server/host_key`
- `log.level`: `"info"`

## Architecture

### Runtime Model

- Single `tokio` async runtime
- `russh::server::Server` trait handles incoming connections
- Each connection spawns a tokio task
- Shell PTY I/O uses `tokio::io::unix::AsyncFd` to bridge sync fd to async

### Connection Flow

```
TCP accept
  → russh handshake
  → authentication (password or public key against config)
  → channel open
  → if shell request:
      → forkpty() → exec("bash" / "sh")
      → async bidirectional copy: SSH channel ↔ PTY fd
      → handle window-change events
  → if subsystem "sftp":
      → create SftpSession
      → handle filesystem operations
```

### Shell (shell.rs)

- `forkpty()` creates a pseudo-terminal
- Child process: check `$SHELL` env var first, then try `/bin/bash`, fallback to `/bin/sh`
- Sets environment: `TERM`, `HOME`, `USER`, `SHELL`, `PATH`
- Parent: async read/write loop between PTY master fd and SSH channel
- Handles `window-change` requests to resize PTY via `ioctl(TIOCSWINSZ)`
- Session timeout: close channel after `session_timeout` seconds of inactivity
- Cleans up child process on channel close (sends SIGHUP, waitpid)

### SFTP (sftp.rs)

Implements `russh::server::SftpSession` trait, mapping operations to local filesystem:

| SFTP Operation | Local Operation |
|---|---|
| `opendir` | `std::fs::read_dir` |
| `readdir` | Iterate dir entries, return `FileAttr` with perm/size/mtime |
| `stat` / `lstat` | `std::fs::metadata` / `std::fs::symlink_metadata` |
| `open` (read) | `std::fs::File::open` |
| `open` (write/create) | `std::fs::File::create` / `OpenOptions` |
| `read` / `write` | Standard file I/O with offsets |
| `close` | Drop file handle |
| `mkdir` | `std::fs::create_dir` |
| `rmdir` | `std::fs::remove_dir` |
| `remove` | `std::fs::remove_file` |
| `rename` | `std::fs::rename` |
| `setstat` (perm) | `std::fs::set_permissions` with `PermissionsExt` |
| `symlink` | `std::os::unix::fs::symlink` |
| `readlink` | `std::fs::read_link` |
| `realpath` | `std::fs::canonicalize` |

File attributes include: size, permissions (unix mode), mtime, atime, uid, gid.

### Authentication (server.rs)

For each auth attempt, check against `config.auth.users`:

- **Password**: compare username + password (constant-time comparison)
- **Public key**: load `authorized_keys` file, compare presented key against listed keys
- If a user has both password and authorized_keys configured, either method is accepted
- Unknown users are rejected

### Host Key

- On first startup, if host key file doesn't exist, generate Ed25519 key pair
- Create parent directory if needed (`std::fs::create_dir_all`)
- Save to configured `host_key.path`
- Load existing key on subsequent starts
- Use `russh_keys::key::KeyPair::generate_ed25519()` for generation

## CLI Interface

```
minishell-server [--config <path>] [--foreground] [--log-level <level>] [--version]
```

- `--config`: Override config file path
- `--foreground`: Run in foreground (default: also foreground, but flag is explicit)
- `--log-level`: Override log level from config
- `--version`: Print version info

## Error Handling

- Config parse errors: print to stderr, exit 1
- Bind failures (port in use, permission denied): print to stderr, exit 1
- Authentication failures: log warning, reject connection (no exit)
- PTY fork failures: log error, close channel
- SFTP errors: return appropriate SFTP status codes to client

## Security Considerations

- Password comparison uses constant-time comparison to prevent timing attacks
- Host key is auto-generated with Ed25519 (strong, fast)
- No plaintext password logging
- Session timeout prevents idle connections
- `max_connections` prevents resource exhaustion

## Workspace Integration

Add `crates/minishell-server` to workspace `Cargo.toml`:

```toml
[workspace]
members = [
    ...
    "crates/minishell-server",
]

[workspace.dependencies]
minishell-server = { path = "crates/minishell-server" }
```

This crate has no dependency on other minishell crates — it is a standalone server binary.

## Testing Strategy

- Unit tests for config parsing (valid/invalid TOML)
- Unit tests for SFTP operations (using temp directories)
- Integration test: start server, connect with ssh2 client, verify shell and SFTP
- Test authentication: valid password, invalid password, valid key, invalid key
