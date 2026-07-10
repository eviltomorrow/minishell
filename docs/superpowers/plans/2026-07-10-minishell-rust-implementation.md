# minishell-rust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 1:1 Rust replication of the Go minishell SSH machine management TUI tool.

**Architecture:** Cargo workspace with 6 crates (core, store, ssh, xlsx, tui, cli). Manual crossterm event loop with ratatui rendering. ssh2 for SSH PTY sessions. rusqlite for SQLite persistence. calamine/rust_xlsxwriter for Excel. clap for CLI.

**Tech Stack:** ratatui, crossterm, rusqlite, ssh2, calamine, rust_xlsxwriter, clap, dirs, unicode-width

## Global Constraints

- DB path: `/tmp/minishell` (directory auto-created)
- NOT_EXIST sentinel: `"-"` for empty fields
- Single SQLite connection (no concurrent access)
- SSH ciphers: aes128-ctr, aes192-ctr, aes256-ctr, aes128-gcm@openssh.com, arcfour256, arcfour128, aes128-cbc
- SSH kex: diffie-hellman-group-exchange-sha1, diffie-hellman-group1-sha1, diffie-hellman-group-exchange-sha256, diffie-hellman-group16-sha512, diffie-hellman-group18-sha512, diffie-hellman-group14-sha256, diffie-hellman-group14-sha1, curve25519-sha256, kex-strict-s-v00@openssh.com
- TUI styles: purple header (63), cyan search (42), pink field (212), red delete, faint help (243)
- Build variables via `env!()` macro

---

## Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/minishell-core/Cargo.toml`
- Create: `crates/minishell-core/src/lib.rs`
- Create: `crates/minishell-store/Cargo.toml`
- Create: `crates/minishell-store/src/lib.rs`
- Create: `crates/minishell-ssh/Cargo.toml`
- Create: `crates/minishell-ssh/src/lib.rs`
- Create: `crates/minishell-ssh/src/card.rs`
- Create: `crates/minishell-tui/Cargo.toml`
- Create: `crates/minishell-tui/src/lib.rs`
- Create: `crates/minishell-tui/src/app.rs`
- Create: `crates/minishell-tui/src/table.rs`
- Create: `crates/minishell-tui/src/form.rs`
- Create: `crates/minishell-tui/src/selector.rs`
- Create: `crates/minishell-tui/src/styles.rs`
- Create: `crates/minishell-xlsx/Cargo.toml`
- Create: `crates/minishell-xlsx/src/lib.rs`
- Create: `crates/minishell-cli/Cargo.toml`
- Create: `crates/minishell-cli/src/main.rs`

**Interfaces:**
- Produces: Workspace that compiles with `cargo check`

- [ ] **Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/minishell-core",
    "crates/minishell-store",
    "crates/minishell-ssh",
    "crates/minishell-tui",
    "crates/minishell-xlsx",
    "crates/minishell-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
minishell-core = { path = "crates/minishell-core" }
minishell-store = { path = "crates/minishell-store" }
minishell-ssh = { path = "crates/minishell-ssh" }
minishell-tui = { path = "crates/minishell-tui" }
minishell-xlsx = { path = "crates/minishell-xlsx" }
```

- [ ] **Step 2: Create minishell-core crate**

```toml
# crates/minishell-core/Cargo.toml
[package]
name = "minishell-core"
version.workspace = true
edition.workspace = true
```

```rust
// crates/minishell-core/src/lib.rs
pub const NOT_EXIST: &str = "-";

#[derive(Debug, Clone)]
pub struct Machine {
    pub id: i64,
    pub num: i32,
    pub nat_ip: String,
    pub ip: String,
    pub username: String,
    pub password: String,
    pub port: i32,
    pub private_key_path: String,
    pub device: String,
    pub remark: String,
}

impl Machine {
    pub fn effective_host(&self) -> &str {
        if !self.nat_ip.is_empty() && self.nat_ip != NOT_EXIST {
            &self.nat_ip
        } else {
            &self.ip
        }
    }

    pub fn is_empty_field(s: &str) -> bool {
        s.is_empty() || s == NOT_EXIST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_host_with_nat() {
        let m = Machine { id: 1, num: 0, nat_ip: "10.0.0.2".into(), ip: "192.168.1.1".into(), username: "root".into(), password: "".into(), port: 22, private_key_path: "".into(), device: "".into(), remark: "".into() };
        assert_eq!(m.effective_host(), "10.0.0.2");
    }

    #[test]
    fn test_effective_host_without_nat() {
        let m = Machine { id: 1, num: 0, nat_ip: "".into(), ip: "192.168.1.1".into(), username: "root".into(), password: "".into(), port: 22, private_key_path: "".into(), device: "".into(), remark: "".into() };
        assert_eq!(m.effective_host(), "192.168.1.1");
    }

    #[test]
    fn test_effective_host_nat_dash() {
        let m = Machine { id: 1, num: 0, nat_ip: NOT_EXIST.into(), ip: "192.168.1.1".into(), username: "root".into(), password: "".into(), port: 22, private_key_path: "".into(), device: "".into(), remark: "".into() };
        assert_eq!(m.effective_host(), "192.168.1.1");
    }

    #[test]
    fn test_is_empty_field() {
        assert!(Machine::is_empty_field(""));
        assert!(Machine::is_empty_field(NOT_EXIST));
        assert!(!Machine::is_empty_field("hello"));
    }
}
```

- [ ] **Step 3: Create minishell-store crate**

```toml
# crates/minishell-store/Cargo.toml
[package]
name = "minishell-store"
version.workspace = true
edition.workspace = true

[dependencies]
minishell-core = { workspace = true }
rusqlite = { version = "0.31", features = ["bundled"] }
anyhow = "1"
```

```rust
// crates/minishell-store/src/lib.rs
use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;
use rusqlite::{Connection, params};

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path).context("Failed to create store directory")?;
        let db_path = path.join("db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at {}", db_path.display()))?;
        conn.execute("PRAGMA journal_mode=WAL", [])?;
        Ok(Store { conn })
    }

    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS machines (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                num         INTEGER,
                ip          TEXT NOT NULL,
                nat_ip      TEXT DEFAULT '',
                port        INTEGER DEFAULT 22,
                username    TEXT NOT NULL,
                password    TEXT DEFAULT '',
                private_key TEXT DEFAULT '',
                device      TEXT DEFAULT '',
                remark      TEXT DEFAULT '',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_machines_ip_port ON machines(ip, port);"
        ).context("Failed to initialize database schema")?;
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<Machine>> {
        let mut stmt = if query.is_empty() {
            self.conn.prepare("SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines ORDER BY id")?
        } else {
            self.conn.prepare("SELECT id, num, ip, nat_ip, port, username, password, private_key, device, remark FROM machines WHERE ip LIKE ?1 OR remark LIKE ?1 ORDER BY id")?
        };

        let rows = if query.is_empty() {
            stmt.query_map([], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    num: row.get(1)?,
                    ip: row.get(2)?,
                    nat_ip: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    password: row.get(6)?,
                    private_key_path: row.get(7)?,
                    device: row.get(8)?,
                    remark: row.get(9)?,
                })
            })?
        } else {
            let pattern = format!("%{}%", query);
            stmt.query_map(params![pattern], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    num: row.get(1)?,
                    ip: row.get(2)?,
                    nat_ip: row.get(3)?,
                    port: row.get(4)?,
                    username: row.get(5)?,
                    password: row.get(6)?,
                    private_key_path: row.get(7)?,
                    device: row.get(8)?,
                    remark: row.get(9)?,
                })
            })?
        };

        let machines = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(machines)
    }

    pub fn count_all(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row("SELECT COUNT(*) FROM machines", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    pub fn import_machines(&self, machines: &[Machine]) -> Result<usize> {
        let mut inserted = 0;
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO machines (num, ip, nat_ip, port, username, password, private_key, device, remark)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            )?;

            for m in machines {
                let changes = stmt.execute(params![
                    m.num, m.ip, m.nat_ip, m.port, m.username, m.password,
                    m.private_key_path, m.device, m.remark
                ]?;
                inserted += changes;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn update_machine(&self, m: &Machine) -> Result<()> {
        self.conn.execute(
            "UPDATE machines SET num=?1, ip=?2, nat_ip=?3, port=?4, username=?5, password=?6, private_key=?7, device=?8, remark=?9, updated_at=datetime('now') WHERE id=?10",
            params![m.num, m.ip, m.nat_ip, m.port, m.username, m.password, m.private_key_path, m.device, m.remark, m.id],
        )?;
        Ok(())
    }

    pub fn delete_machine(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM machines WHERE id=?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_store() -> (Store, PathBuf) {
        let dir = PathBuf::from(format!("/tmp/minishell_test_{}", std::process::id()));
        let store = Store::open(&dir).unwrap();
        store.init().unwrap();
        (store, dir)
    }

    fn test_machine(ip: &str) -> Machine {
        Machine {
            id: 0, num: 0, nat_ip: "".into(), ip: ip.into(), username: "root".into(),
            password: "pass".into(), port: 22, private_key_path: "".into(),
            device: "Linux".into(), remark: "test".into(),
        }
    }

    #[test]
    fn test_import_and_search() {
        let (store, dir) = temp_store();
        let machines = vec![test_machine("10.0.0.1"), test_machine("10.0.0.2")];
        let inserted = store.import_machines(&machines).unwrap();
        assert_eq!(inserted, 2);

        let all = store.search("").unwrap();
        assert_eq!(all.len(), 2);

        let found = store.search("10.0.0.1").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ip, "10.0.0.1");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_update_and_delete() {
        let (store, dir) = temp_store();
        store.import_machines(&vec![test_machine("10.0.0.1")]).unwrap();
        let mut m = store.search("10.0.0.1").unwrap().remove(0);
        m.remark = "updated".into();
        store.update_machine(&m).unwrap();
        let updated = store.search("10.0.0.1").unwrap();
        assert_eq!(updated[0].remark, "updated");

        store.delete_machine(m.id).unwrap();
        assert_eq!(store.search("").unwrap().len(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_count_all() {
        let (store, dir) = temp_store();
        assert_eq!(store.count_all().unwrap(), 0);
        store.import_machines(&vec![test_machine("10.0.0.1")]).unwrap();
        assert_eq!(store.count_all().unwrap(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 4: Create minishell-ssh crate**

```toml
# crates/minishell-ssh/Cargo.toml
[package]
name = "minishell-ssh"
version.workspace = true
edition.workspace = true

[dependencies]
minishell-core = { workspace = true }
ssh2 = "0.9"
anyhow = "1"
dirs = "5"
```

```rust
// crates/minishell-ssh/src/lib.rs
pub mod card;

use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use anyhow::{Result, Context};
use minishell_core::Machine;

pub struct ConnectConfig {
    pub username: String,
    pub password: String,
    pub private_key_path: String,
    pub host: String,
    pub port: i32,
    pub timeout: Duration,
}

pub fn connect(config: &ConnectConfig) -> Result<()> {
    let addr = format!("{}:{}", config.host, config.port);
    let tcp = TcpStream::connect_timeout(
        &addr.parse().context("Invalid address")?,
        config.timeout,
    ).with_context(|| format!("Failed to connect to {}", addr))?;

    let mut session = ssh2::Session::new().context("Failed to create SSH session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("SSH handshake failed")?;

    // Authenticate
    if !config.private_key_path.is_empty() {
        session.userauth_pubkey_file(&config.username, None, std::path::Path::new(&config.private_key_path), None)
            .context("Public key auth failed")?;
    } else if !config.password.is_empty() {
        session.userauth_password(&config.username, &config.password)
            .context("Password auth failed")?;
    } else {
        session.userauth_agent(&config.username)
            .context("Agent auth failed")?;
    }

    if !session.authenticated() {
        anyhow::bail!("Authentication failed");
    }

    let mut channel = session.channel_session().context("Failed to open channel")?;
    channel.request_pty("xterm-256color", None, None)?;

    // Set raw terminal
    let _ = crossterm::terminal::enable_raw_mode();

    let mut stdout = std::io::stdout();
    let mut stdin = std::io::stdin();

    // Copy remote stdout to local stdout
    let mut chan_stdout = channel.stdout();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match chan_stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => { let _ = stdout.write_all(&buf[..n]); let _ = stdout.flush(); }
                Err(_) => break,
            }
        }
    });

    // Copy remote stderr to local stderr
    let mut chan_stderr = channel.stderr();
    let mut stderr = std::io::stderr();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match chan_stderr.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => { let _ = stderr.write_all(&buf[..n]); let _ = stderr.flush(); }
                Err(_) => break,
            }
        }
    });

    // Copy local stdin to remote stdin
    let mut chan_stdin = channel.stdin().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => { let _ = chan_stdin.write_all(&buf[..n]); let _ = chan_stdin.flush(); }
                Err(_) => break,
            }
        }
    });

    // Wait for channel to close
    let _ = channel.wait_close();

    let _ = crossterm::terminal::disable_raw_mode();
    Ok(())
}

pub fn login_to_machine(machine: &Machine) -> Result<Duration> {
    let host = machine.effective_host();
    let auth_method = if !machine.private_key_path.is_empty() {
        machine.private_key_path.split('/').last().unwrap_or("key")
    } else if !machine.password.is_empty() {
        "password"
    } else {
        "none"
    };

    // Set terminal title
    print!("\x1b]0;{}\x07", host);

    let width = card::terminal_width();
    println!("{}", card::connect_card_top(&machine.ip, host, machine.port, &machine.username, auth_method, width));
    println!("{}", card::connect_card_status_line("Connecting...", width));

    let config = ConnectConfig {
        username: machine.username.clone(),
        password: machine.password.clone(),
        private_key_path: machine.private_key_path.clone(),
        host: host.to_string(),
        port: machine.port,
        timeout: Duration::from_secs(10),
    };

    let start = Instant::now();
    let result = connect(&config);
    let duration = start.elapsed();

    // Move up and replace status line
    print!("\x1b[A\r\x1b[K");
    match &result {
        Ok(()) => println!("{}", card::connect_success_line(duration)),
        Err(e) => println!("{}", card::connect_fail_line(&e.to_string())),
    }

    println!("{}", card::disconnect_card(host, duration, result.err().map(|e| e.to_string()).as_deref(), width));

    // Reset terminal title
    print!("\x1b]0;minishell\x07");

    Ok(duration)
}
```

```rust
// crates/minishell-ssh/src/card.rs
use std::io::Write;

const CARD_LINE_COLOR: &str = "\x1b[38;5;39m";
const CARD_LABEL: &str = "\x1b[38;5;87m";
const CARD_GREEN: &str = "\x1b[38;5;42m";
const CARD_RED: &str = "\x1b[38;5;196m";
const CARD_FAINT: &str = "\x1b[38;5;243m";
const CARD_BOLD: &str = "\x1b[1m";
const CARD_BOLD_OFF: &str = "\x1b[22m";
const CARD_RESET: &str = "\x1b[0m";

pub fn terminal_width() -> usize {
    crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80).max(40)
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            while let Some(&next) = chars.clone().next().as_ref() {
                chars.next();
                if next == 'm' { break; }
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn unicode_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}

pub fn card_line(width: usize, content: &str) -> String {
    let visible = unicode_width(content);
    let padding = if width > 2 && visible < width - 2 { width - 2 - visible } else { 0 };
    format!("{}│{}{}{}│{}", CARD_LINE_COLOR, CARD_RESET, content, " ".repeat(padding), CARD_LINE_COLOR)
}

pub fn card_border(width: usize, corner_left: &str, corner_right: &str) -> String {
    format!("{}{}{}{}{}", CARD_LINE_COLOR, corner_left, "─".repeat(width.saturating_sub(2)), corner_right, CARD_RESET)
}

pub fn connect_card_top(ip: &str, host: &str, port: i32, username: &str, auth_method: &str, width: usize) -> String {
    let mut lines = Vec::new();
    lines.push(card_border(width, "┌", "┐"));
    lines.push(card_line(width, &format!("{}SSH CONNECT{}", CARD_BOLD, CARD_BOLD_OFF)));
    lines.push(card_line(width, ""));
    lines.push(card_line(width, &format!("{}User: {}{}{}", CARD_LABEL, CARD_RESET, username, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Host: {}{}{}", CARD_LABEL, CARD_RESET, host, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}IP:   {}{}{}", CARD_LABEL, CARD_RESET, ip, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Auth: {}{}{}", CARD_LABEL, CARD_RESET, auth_method, CARD_FAINT)));
    lines.push(card_line(width, ""));
    lines.push(card_border(width, "├", "┤"));
    lines.join("\n")
}

pub fn connect_card_status_line(content: &str, width: usize) -> String {
    card_line(width, content)
}

pub fn connect_card_bottom(width: usize) -> String {
    card_border(width, "└", "┘")
}

pub fn connect_success_line(duration: std::time::Duration) -> String {
    let ms = duration.as_millis();
    format!("{}  ✓ Connected in {}ms{}", CARD_GREEN, ms, CARD_RESET)
}

pub fn connect_fail_line(err: &str) -> String {
    format!("{}  ✗ Failed: {}{}", CARD_RED, err, CARD_RESET)
}

pub fn disconnect_card(host: &str, duration: std::time::Duration, ssh_err: Option<&str>, width: usize) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    let duration_str = format!("{}h {}m {}s", hours, mins, secs);

    let status = match ssh_err {
        Some(err) => format!("{}Error: {}{}", CARD_RED, err, CARD_RESET),
        None => format!("{}OK{}", CARD_GREEN, CARD_RESET),
    };

    let mut lines = Vec::new();
    lines.push(card_border(width, "┌", "┐"));
    lines.push(card_line(width, &format!("{}DISCONNECTED{}", CARD_BOLD, CARD_BOLD_OFF)));
    lines.push(card_line(width, &format!("{}Host:   {}{}{}", CARD_LABEL, CARD_RESET, host, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Duration: {}{}{}", CARD_LABEL, CARD_RESET, duration_str, CARD_FAINT)));
    lines.push(card_line(width, &format!("{}Status: {}", CARD_LABEL, status)));
    lines.push(card_border(width, "└", "┘"));
    lines.join("\n")
}
```

- [ ] **Step 5: Create minishell-tui crate (skeleton)**

```toml
# crates/minishell-tui/Cargo.toml
[package]
name = "minishell-tui"
version.workspace = true
edition.workspace = true

[dependencies]
minishell-core = { workspace = true }
minishell-store = { workspace = true }
ratatui = "0.28"
crossterm = { version = "0.28", features = ["event-stream"] }
unicode-width = "0.1"
anyhow = "1"
```

```rust
// crates/minishell-tui/src/lib.rs
pub mod app;
pub mod table;
pub mod form;
pub mod selector;
pub mod styles;

use std::sync::Arc;
use minishell_core::Machine;
use minishell_store::Store;

pub fn run(store: Arc<Store>) -> anyhow::Result<Option<Machine>> {
    app::run(store)
}

pub fn select_machine(machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    selector::select_machine(machines)
}
```

```rust
// crates/minishell-tui/src/styles.rs
use ratatui::style::{Color, Modifier, Style};

pub fn header_style() -> Style {
    Style::default().fg(Color::Indexed(63)).add_modifier(Modifier::BOLD)
}

pub fn main_border_style() -> Style {
    Style::default().fg(Color::Indexed(240))
}

pub fn help_style() -> Style {
    Style::default().fg(Color::Indexed(243))
}

pub fn search_style() -> Style {
    Style::default().fg(Color::Indexed(42)).add_modifier(Modifier::BOLD)
}

pub fn form_box_style() -> Style {
    Style::default().fg(Color::Indexed(63))
}

pub fn delete_box_style() -> Style {
    Style::default().fg(Color::Indexed(196))
}

pub fn form_field_style() -> Style {
    Style::default().fg(Color::Indexed(212)).add_modifier(Modifier::BOLD)
}

pub fn separator_style() -> Style {
    Style::default().fg(Color::Indexed(243))
}

pub fn key_style() -> Style {
    Style::default().fg(Color::Indexed(42)).add_modifier(Modifier::BOLD)
}

pub fn status_style() -> Style {
    Style::default().fg(Color::Indexed(243))
}

pub fn selected_style() -> Style {
    Style::default().fg(Color::Indexed(212)).bg(Color::Indexed(238))
}
```

```rust
// crates/minishell-tui/src/table.rs
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
pub struct Column {
    pub title: String,
    pub width: usize,
}

pub struct MachineTable {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
    pub cursor: usize,
    pub width: u16,
    pub height: u16,
}

impl MachineTable {
    pub fn new(columns: Vec<Column>) -> Self {
        MachineTable {
            columns,
            rows: Vec::new(),
            cursor: 0,
            width: 0,
            height: 0,
        }
    }

    pub fn set_size(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
    }

    pub fn set_rows(&mut self, rows: Vec<Vec<String>>) {
        self.rows = rows;
        if self.cursor >= self.rows.len() && !self.rows.is_empty() {
            self.cursor = self.rows.len() - 1;
        }
    }

    pub fn cursor(&self) -> usize { self.cursor }

    pub fn set_cursor(&mut self, n: usize) {
        self.cursor = n.min(self.rows.len().saturating_sub(1));
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 { self.cursor -= 1; }
    }

    pub fn move_down(&mut self) {
        if self.cursor < self.rows.len().saturating_sub(1) { self.cursor += 1; }
    }

    pub fn goto_top(&mut self) { self.cursor = 0; }

    pub fn goto_bottom(&mut self) {
        if !self.rows.is_empty() { self.cursor = self.rows.len() - 1; }
    }

    pub fn rows_count(&self) -> usize { self.rows.len() }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, selected_style: Style, normal_style: Style) {
        let header_style = super::styles::header_style();

        // Header
        let mut header_line = Vec::new();
        for col in &self.columns {
            header_line.push(Span::styled(pad_right(&col.title, col.width), header_style));
        }
        let header = Line::from(header_line);
        buf.set_line(area.x, area.y, &header, area.width);

        // Rows
        let visible_height = self.height.saturating_sub(1) as usize;
        let total_rows = self.rows.len();

        let top = if total_rows <= visible_height {
            0
        } else {
            (self.cursor as isize - visible_height as isize / 2).max(0) as usize
        };
        let top = top.min(total_rows.saturating_sub(visible_height));

        for i in 0..visible_height {
            let row_idx = top + i;
            if row_idx >= total_rows { break; }

            let y = area.y + 1 + i as u16;
            let style = if row_idx == self.cursor { selected_style } else { normal_style };

            let mut spans = Vec::new();
            for (j, col) in self.columns.iter().enumerate() {
                let text = self.rows[row_idx].get(j).map(|s| s.as_str()).unwrap_or("");
                spans.push(Span::styled(pad_right(text, col.width), style));
            }
            let line = Line::from(spans);
            buf.set_line(area.x, y, &line, area.width);
        }
    }
}

fn pad_right(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        truncate_to_width(s, width)
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width {
            break;
        }
        result.push(c);
        current_width += cw;
    }
    result
}

pub fn format_machine_row(m: &minishell_core::Machine, show_secrets: bool) -> Vec<String> {
    let empty = "-".to_string();
    if show_secrets {
        vec![
            format!("{}", m.num),
            m.ip.clone(),
            "".to_string(),
            if m.nat_ip.is_empty() { empty.clone() } else { m.nat_ip.clone() },
            m.username.clone(),
            if m.password.is_empty() { empty.clone() } else { m.password.clone() },
            "".to_string(),
            if m.private_key_path.is_empty() { empty.clone() } else { m.private_key_path.clone() },
            if m.device.is_empty() { empty.clone() } else { m.device.clone() },
            if m.remark.is_empty() { empty.clone() } else { m.remark.clone() },
        ]
    } else {
        vec![
            format!("{}", m.num),
            m.ip.clone(),
            "".to_string(),
            if m.nat_ip.is_empty() { empty.clone() } else { m.nat_ip.clone() },
            m.username.clone(),
            if m.device.is_empty() { empty.clone() } else { m.device.clone() },
            if m.remark.is_empty() { empty.clone() } else { m.remark.clone() },
        ]
    }
}

pub fn default_columns() -> Vec<Column> {
    vec![
        Column { title: "#".into(), width: 4 },
        Column { title: "IP".into(), width: 15 },
        Column { title: "".into(), width: 2 },
        Column { title: "NAT".into(), width: 12 },
        Column { title: "User".into(), width: 10 },
        Column { title: "Device".into(), width: 10 },
        Column { title: "Remark".into(), width: 20 },
    ]
}

pub fn secrets_columns() -> Vec<Column> {
    vec![
        Column { title: "#".into(), width: 4 },
        Column { title: "IP".into(), width: 15 },
        Column { title: "".into(), width: 2 },
        Column { title: "NAT".into(), width: 12 },
        Column { title: "User".into(), width: 10 },
        Column { title: "Password".into(), width: 15 },
        Column { title: "".into(), width: 2 },
        Column { title: "Key".into(), width: 20 },
        Column { title: "Device".into(), width: 10 },
        Column { title: "Remark".into(), width: 20 },
    ]
}
```

```rust
// crates/minishell-tui/src/form.rs
use minishell_core::Machine;

pub const FORM_FIELDS: &[(&str, usize, usize)] = &[
    ("IP:", 64, 40),
    ("NAT-IP:", 64, 40),
    ("Port:", 5, 10),
    ("Username:", 64, 40),
    ("Password:", 64, 40),
    ("PrivateKey:", 64, 40),
    ("Device:", 64, 40),
    ("Remark:", 64, 40),
];

pub struct FormField {
    pub label: String,
    pub value: String,
    pub max_length: usize,
    pub width: usize,
    pub cursor_pos: usize,
}

impl FormField {
    pub fn new(label: &str, max_length: usize, width: usize) -> Self {
        FormField {
            label: label.to_string(),
            value: String::new(),
            max_length,
            width,
            cursor_pos: 0,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.value.len() < self.max_length {
            self.value.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
        }
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.value.remove(self.cursor_pos);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 { self.cursor_pos -= 1; }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.value.len() { self.cursor_pos += 1; }
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor_pos = 0;
    }
}

pub struct FormState {
    pub fields: Vec<FormField>,
    pub step: usize,
    pub is_edit: bool,
    pub target_id: Option<i64>,
}

impl FormState {
    pub fn new_add() -> Self {
        let fields = FORM_FIELDS.iter()
            .map(|(label, max_len, width)| FormField::new(label, *max_len, *width))
            .collect();
        FormState { fields, step: 0, is_edit: false, target_id: None }
    }

    pub fn new_edit(machine: &Machine) -> Self {
        let values = vec![
            machine.ip.clone(),
            machine.nat_ip.clone(),
            if machine.port == 22 { "".into() } else { machine.port.to_string() },
            machine.username.clone(),
            machine.password.clone(),
            machine.private_key_path.clone(),
            machine.device.clone(),
            machine.remark.clone(),
        ];

        let fields: Vec<FormField> = FORM_FIELDS.iter().enumerate()
            .map(|(i, (label, max_len, width))| {
                let mut f = FormField::new(label, *max_len, *width);
                let val = &values[i];
                f.value = if val == "-" || val.is_empty() { String::new() } else { val.clone() };
                f.cursor_pos = f.value.len();
                f
            })
            .collect();

        FormState { fields, step: 0, is_edit: true, target_id: Some(machine.id) }
    }

    pub fn navigate_next(&mut self) {
        self.step = (self.step + 1) % self.fields.len();
    }

    pub fn navigate_prev(&mut self) {
        self.step = if self.step == 0 { self.fields.len() - 1 } else { self.step - 1 };
    }

    pub fn to_machine(&self) -> Machine {
        let port: i32 = self.fields[2].value.parse().unwrap_or(22);
        let or_dash = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };

        Machine {
            id: self.target_id.unwrap_or(0),
            num: 0,
            ip: or_dash(&self.fields[0].value),
            nat_ip: or_dash(&self.fields[1].value),
            port,
            username: or_dash(&self.fields[3].value),
            password: or_dash(&self.fields[4].value),
            private_key_path: or_dash(&self.fields[5].value),
            device: or_dash(&self.fields[6].value),
            remark: or_dash(&self.fields[7].value),
        }
    }
}

pub struct DeleteState {
    pub target: Machine,
}

impl DeleteState {
    pub fn new(target: Machine) -> Self {
        DeleteState { target }
    }
}
```

```rust
// crates/minishell-tui/src/selector.rs
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Row, Table};
use ratatui::Terminal;
use minishell_core::Machine;

pub struct SelectorState {
    machines: Vec<Machine>,
    cursor: usize,
    selected: Option<Machine>,
    quitting: bool,
}

pub fn select_machine(machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    if machines.is_empty() { return Ok(None); }
    if machines.len() == 1 { return Ok(Some(machines.into_iter().next().unwrap())); }

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = SelectorState {
        machines,
        cursor: 0,
        selected: None,
        quitting: false,
    };

    loop {
        terminal.draw(|f| draw_selector(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            handle_selector_key(&mut state, key);
        }

        if state.quitting || state.selected.is_some() {
            break;
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen)?;
    Ok(state.selected)
}

fn draw_selector(f: &mut ratatui::Frame, state: &mut SelectorState) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    // Title
    let title = Line::from(vec![
        Span::styled(" Select Machine ", Style::default().fg(Color::Indexed(63)).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(title, chunks[0]);

    // Table header
    let header = Row::new(vec![
        Cell::from(">"),
        Cell::from("#"),
        Cell::from("IP"),
        Cell::from("NAT-IP"),
        Cell::from("Port"),
        Cell::from("User"),
        Cell::from("Remark"),
    ]).style(Style::default().fg(Color::Indexed(63)).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = state.machines.iter().enumerate().map(|(i, m)| {
        let indicator = if i == state.cursor { "▸" } else { " " };
        Row::new(vec![
            Cell::from(indicator),
            Cell::from(format!("{}", i + 1)),
            Cell::from(m.ip.clone()),
            Cell::from(m.nat_ip.clone()),
            Cell::from(format!("{}", m.port)),
            Cell::from(m.username.clone()),
            Cell::from(m.remark.clone()),
        ])
    }).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(20),
        ],
    ).header(header);

    f.render_widget(table, chunks[0]);

    // Help
    let help = Line::from(vec![
        Span::styled("↑↓", Style::default().fg(Color::Indexed(42)).add_modifier(Modifier::BOLD)),
        Span::styled(" navigate  ", Style::default().fg(Color::Indexed(243))),
        Span::styled("↵", Style::default().fg(Color::Indexed(42)).add_modifier(Modifier::BOLD)),
        Span::styled(" select  ", Style::default().fg(Color::Indexed(243))),
        Span::styled("q", Style::default().fg(Color::Indexed(42)).add_modifier(Modifier::BOLD)),
        Span::styled(" quit", Style::default().fg(Color::Indexed(243))),
    ]);
    f.render_widget(help, chunks[1]);
}

use ratatui::widgets::Cell;

fn handle_selector_key(state: &mut SelectorState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => state.quitting = true,
        KeyCode::Up | KeyCode::Char('k') => {
            if state.cursor > 0 { state.cursor -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if state.cursor < state.machines.len() - 1 { state.cursor += 1; }
        }
        KeyCode::Enter => {
            state.selected = Some(state.machines[state.cursor].clone());
        }
        _ => {}
    }
}
```

```rust
// crates/minishell-tui/src/app.rs
use std::sync::Arc;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use minishell_core::Machine;
use minishell_store::Store;

use super::form::{FormState, DeleteState};
use super::table::{MachineTable, default_columns, secrets_columns, format_machine_row};
use super::styles;

pub struct AppState {
    pub store: Arc<Store>,
    pub machines: Vec<Machine>,
    pub table: MachineTable,
    pub search_input: String,
    pub search_focused: bool,
    pub show_secrets: bool,
    pub form: Option<FormState>,
    pub delete_confirm: Option<DeleteState>,
    pub should_quit: bool,
    pub login_target: Option<Machine>,
    pub terminal_size: (u16, u16),
}

pub fn run(store: Arc<Store>) -> anyhow::Result<Option<Machine>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let machines = store.search("")?;
    let total = store.count_all()?;
    let columns = default_columns();
    let table = MachineTable::new(columns);

    let mut state = AppState {
        store,
        machines,
        table,
        search_input: String::new(),
        search_focused: false,
        show_secrets: false,
        form: None,
        delete_confirm: None,
        should_quit: false,
        login_target: None,
        terminal_size: (0, 0),
    };

    // Initial table data
    rebuild_table(&mut state);

    loop {
        terminal.draw(|f| view(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            update(&mut state, key);
        }

        if state.should_quit {
            break;
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), crossterm::terminal::LeaveAlternateScreen)?;

    Ok(state.login_target)
}

fn rebuild_table(state: &mut AppState) {
    let rows: Vec<Vec<String>> = state.machines.iter()
        .map(|m| format_machine_row(m, state.show_secrets))
        .collect();

    if state.show_secrets {
        state.table.columns = secrets_columns();
    } else {
        state.table.columns = default_columns();
    }
    state.table.set_rows(rows);
}

fn view(f: &mut ratatui::Frame, state: &mut AppState) {
    let area = f.area();

    let dialog_lines = if state.form.is_some() { 12 } else if state.delete_confirm.is_some() { 6 } else { 0 };

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),   // title
            Constraint::Length(1),   // separator
            Constraint::Min(5),     // table
            Constraint::Length(1),   // separator
            Constraint::Length(1),   // status
            Constraint::Length(1),   // help
        ])
        .split(area);

    // Title
    let title = Line::from(vec![
        Span::styled(" minishell ", Style::default().fg(Color::Indexed(63)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} machines", state.machines.len()), Style::default().fg(Color::Indexed(243))),
    ]);
    f.render_widget(title, main_chunks[0]);

    // Separator
    let sep = Line::from(vec![Span::styled("─".repeat(area.width as usize), styles::separator_style())]);
    f.render_widget(sep, main_chunks[1]);

    // Table
    let table_height = main_chunks[2].height;
    state.table.set_size(area.width, table_height);
    let selected_style = styles::selected_style();
    let normal_style = Style::default();
    state.table.render(main_chunks[2], f.buffer_mut(), selected_style, normal_style);

    // Separator
    let sep2 = Line::from(vec![Span::styled("─".repeat(area.width as usize), styles::separator_style())]);
    f.render_widget(sep2, main_chunks[3]);

    // Status bar
    let status = if let Some(m) = state.machines.get(state.table.cursor()) {
        format!("[{}/{}] [{}@{}:{}] [secrets {}]",
            state.table.cursor() + 1,
            state.machines.len(),
            m.username,
            m.effective_host(),
            m.port,
            if state.show_secrets { "show" } else { "hide" })
    } else {
        format!("[0/0] [secrets {}]", if state.show_secrets { "show" } else { "hide" })
    };
    let status_line = Line::from(vec![Span::styled(status, styles::status_style())]);
    f.render_widget(status_line, main_chunks[4]);

    // Help bar
    let help = Line::from(vec![
        Span::styled("↑↓", styles::key_style()),
        Span::styled(" sel ", styles::help_style()),
        Span::styled("↵", styles::key_style()),
        Span::styled(" login ", styles::help_style()),
        Span::styled("e", styles::key_style()),
        Span::styled(" edit ", styles::help_style()),
        Span::styled("a", styles::key_style()),
        Span::styled(" add ", styles::help_style()),
        Span::styled("d", styles::key_style()),
        Span::styled(" del ", styles::help_style()),
        Span::styled("s", styles::key_style()),
        Span::styled(" secrets ", styles::help_style()),
        Span::styled("/", styles::key_style()),
        Span::styled(" search ", styles::help_style()),
        Span::styled("q", styles::key_style()),
        Span::styled(" quit", styles::help_style()),
    ]);
    f.render_widget(help, main_chunks[5]);

    // Dialog overlays
    if let Some(ref form_state) = state.form {
        let dialog_area = centered_rect(50, 12, area);
        render_form(f, dialog_area, form_state);
    } else if let Some(ref del_state) = state.delete_confirm {
        let dialog_area = centered_rect(40, 6, area);
        render_delete_confirm(f, dialog_area, &del_state.target);
    }

    // Search bar overlay at top
    if state.search_focused {
        let search_area = Rect { x: 0, y: 0, width: area.width, height: 1 };
        let search_text = format!("/ {}", state.search_input);
        let search_para = Paragraph::new(Line::from(vec![
            Span::styled(search_text, styles::search_style()),
        ]));
        f.render_widget(search_para, search_area);
    }
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((r.width.saturating_sub(r.width * percent_x / 100)) / 2),
            Constraint::Length(r.width * percent_x / 100),
            Constraint::Min(0),
        ])
        .split(popup_layout[1])[1]
}

fn render_form(f: &mut ratatui::Frame, area: Rect, form_state: &FormState) {
    let title = if form_state.is_edit { "Edit Machine" } else { "Add Machine" };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(styles::form_box_style())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(area);
    f.render_widget(block, area);

    for (i, field) in form_state.fields.iter().enumerate() {
        if i as u16 >= inner.height { break; }
        let y = inner.y + i as u16;
        let style = if i == form_state.step { styles::form_field_style() } else { Style::default() };

        let label = Span::styled(format!("{:>12} ", field.label), style);
        let value = Span::styled(&field.value, style);
        let line = Line::from(vec![label, value]);
        f.render_widget(line, Rect { x: inner.x, y, width: inner.width, height: 1 });
    }

    // Navigation hints
    let hint_y = inner.y + inner.height.saturating_sub(1);
    let hints = Line::from(vec![
        Span::styled("↑↓", styles::key_style()),
        Span::styled(" next  ", styles::help_style()),
        Span::styled("↵", styles::key_style()),
        Span::styled(" save  ", styles::help_style()),
        Span::styled("Esc", styles::key_style()),
        Span::styled(" back", styles::help_style()),
    ]);
    f.render_widget(hints, Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 });
}

fn render_delete_confirm(f: &mut ratatui::Frame, area: Rect, target: &Machine) {
    let block = Block::default()
        .title("Delete Machine")
        .borders(Borders::ALL)
        .border_style(styles::delete_box_style())
        .border_type(ratatui::widgets::BorderType::Rounded);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let msg = format!("Remove {} ({})?", target.ip, target.username);
    let line = Line::from(Span::styled(msg, Style::default()));
    f.render_widget(line, Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });

    let hints = Line::from(vec![
        Span::styled("y", styles::key_style()),
        Span::styled(" yes  ", styles::help_style()),
        Span::styled("n", styles::key_style()),
        Span::styled(" no  ", styles::help_style()),
        Span::styled("Esc", styles::key_style()),
        Span::styled(" cancel", styles::help_style()),
    ]);
    f.render_widget(hints, Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 });
}

fn update(state: &mut AppState, key: KeyEvent) {
    // Handle form/dialog first
    if let Some(ref mut form) = state.form {
        handle_form_key(state, form, key);
        return;
    }
    if let Some(ref del) = state.delete_confirm {
        handle_delete_key(state, key);
        return;
    }

    // Search focused
    if state.search_focused {
        match key.code {
            KeyCode::Esc => {
                state.search_focused = false;
                state.search_input.clear();
                reload_machines(state);
            }
            KeyCode::Enter => {
                state.search_focused = false;
                reload_machines(state);
            }
            KeyCode::Backspace => {
                state.search_input.pop();
                reload_machines(state);
            }
            KeyCode::Char(c) => {
                state.search_input.push(c);
                reload_machines(state);
            }
            _ => {}
        }
        return;
    }

    // Normal mode
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Up | KeyCode::Char('k') => state.table.move_up(),
        KeyCode::Down | KeyCode::Char('j') => state.table.move_down(),
        KeyCode::PageUp => {
            for _ in 0..10 { state.table.move_up(); }
        }
        KeyCode::PageDown => {
            for _ in 0..10 { state.table.move_down(); }
        }
        KeyCode::Char('g') => state.table.goto_top(),
        KeyCode::Char('G') => state.table.goto_bottom(),
        KeyCode::Enter => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.login_target = Some(m);
                state.should_quit = true;
            }
        }
        KeyCode::Char('/') => {
            state.search_focused = true;
            state.search_input.clear();
        }
        KeyCode::Char('a') => {
            state.form = Some(FormState::new_add());
        }
        KeyCode::Char('e') => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.form = Some(FormState::new_edit(&m));
            }
        }
        KeyCode::Char('d') => {
            if let Some(m) = state.machines.get(state.table.cursor()).cloned() {
                state.delete_confirm = Some(DeleteState::new(m));
            }
        }
        KeyCode::Char('s') => {
            state.show_secrets = !state.show_secrets;
            rebuild_table(state);
        }
        _ => {}
    }
}

fn handle_form_key(state: &mut AppState, form: &mut FormState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            state.form = None;
        }
        KeyCode::Up | KeyCode::Char('k') => form.navigate_prev(),
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => form.navigate_next(),
        KeyCode::Enter => {
            if form.step == form.fields.len() - 1 {
                // Last field - save
                let machine = form.to_machine();
                if form.is_edit {
                    let _ = state.store.update_machine(&machine);
                } else {
                    let _ = state.store.import_machines(&[machine]);
                }
                state.form = None;
                reload_machines(state);
            } else {
                form.navigate_next();
            }
        }
        KeyCode::Char(c) => {
            form.fields[form.step].insert_char(c);
        }
        KeyCode::Backspace => {
            form.fields[form.step].delete_char();
        }
        KeyCode::Left => {
            form.fields[form.step].move_cursor_left();
        }
        KeyCode::Right => {
            form.fields[form.step].move_cursor_right();
        }
        _ => {}
    }
}

fn handle_delete_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(del) = state.delete_confirm.take() {
                let _ = state.store.delete_machine(del.target.id);
                reload_machines(state);
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.delete_confirm = None;
        }
        _ => {}
    }
}

fn reload_machines(state: &mut AppState) {
    state.machines = state.store.search(&state.search_input).unwrap_or_default();
    rebuild_table(state);
}
```

- [ ] **Step 6: Create minishell-xlsx crate**

```toml
# crates/minishell-xlsx/Cargo.toml
[package]
name = "minishell-xlsx"
version.workspace = true
edition.workspace = true

[dependencies]
minishell-core = { workspace = true }
calamine = "0.26"
rust_xlsxwriter = "0.67"
anyhow = "1"
```

```rust
// crates/minishell-xlsx/src/lib.rs
use std::path::Path;
use anyhow::{Result, Context};
use minishell_core::Machine;

pub fn generate_template(path: &Path) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet().context("Failed to add worksheet")?;

    let headers = ["IP", "NAT-IP", "Port", "Username", "Password", "PrivateKey-Path", "Device", "Remark"];
    for (i, h) in headers.iter().enumerate() {
        sheet.write_string(0, i as u16, h, None)?;
    }

    let example = ["10.0.0.1", "-", "22", "root", "your-password", "-", "Linux", "example"];
    for (i, v) in example.iter().enumerate() {
        sheet.write_string(1, i as u16, v, None)?;
    }

    workbook.save(path).context("Failed to save template")?;
    Ok(())
}

pub fn import_from(path: &Path) -> Result<Vec<Machine>> {
    let mut workbook = calamine::open_workbook_auto(path).context("Failed to open Excel file")?;
    let sheet_names = workbook.sheet_names().to_owned();
    let sheet_name = sheet_names.first().context("No sheets found")?;

    let range = workbook.worksheet_range(sheet_name).context("Failed to read sheet")?;
    let rows: Vec<Vec<calamine::Data>> = range.rows().map(|r| r.to_vec()).collect();

    if rows.len() < 2 {
        anyhow::bail!("Excel file must have at least 2 rows (header + data)");
    }

    let mut machines = Vec::new();
    for row in &rows[1..] {
        if row.is_empty() { continue; }

        let ip = cell_to_string(row.get(0));
        if ip.is_empty() || looks_like_description(&ip) { continue; }

        let nat_ip = cell_to_string(row.get(1));
        let port: i32 = cell_to_string(row.get(2)).parse().unwrap_or(22);
        let username = cell_to_string(row.get(3));
        let password = cell_to_string(row.get(4));
        let private_key_path = cell_to_string(row.get(5));
        let device = cell_to_string(row.get(6));
        let remark = cell_to_string(row.get(7));

        machines.push(Machine {
            id: 0,
            num: 0,
            nat_ip: if nat_ip.is_empty() { "-".into() } else { nat_ip },
            ip,
            username: if username.is_empty() { "root".into() } else { username },
            password: if password.is_empty() { "-".into() } else { password },
            port,
            private_key_path: if private_key_path.is_empty() { "-".into() } else { private_key_path },
            device: if device.is_empty() { "-".into() } else { device },
            remark: if remark.is_empty() { "-".into() } else { remark },
        });
    }

    Ok(machines)
}

pub fn export_to(path: &Path, machines: &[Machine]) -> Result<()> {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet().context("Failed to add worksheet")?;

    let headers = ["#", "IP", "NAT-IP", "Port", "Username", "Password", "PrivateKey", "Device", "Remark"];
    let header_format = rust_xlsxwriter::Format::new()
        .set_bold()
        .set_font_color(rust_xlsxwriter::Color::RGB(0xFFFFFF))
        .set_bg_color(rust_xlsxwriter::Color::RGB(0x4472C4))
        .set_align(rust_xlsxwriter::FormatAlignment::Center);

    for (i, h) in headers.iter().enumerate() {
        sheet.write_string_with_format(0, i as u16, h, &header_format)?;
    }

    let stripe_format = rust_xlsxwriter::Format::new()
        .set_bg_color(rust_xlsxwriter::Color::RGB(0xD9E2F3));

    for (idx, m) in machines.iter().enumerate() {
        let row = (idx + 1) as u32;
        let format = if idx % 2 == 1 { Some(&stripe_format) } else { None };

        let values = [
            format!("{}", idx + 1),
            m.ip.clone(),
            m.nat_ip.clone(),
            format!("{}", m.port),
            m.username.clone(),
            m.password.clone(),
            m.private_key_path.clone(),
            m.device.clone(),
            m.remark.clone(),
        ];

        for (col, val) in values.iter().enumerate() {
            match format {
                Some(f) => sheet.write_string_with_format(row, col as u16, val, f)?,
                None => sheet.write_string(row, col as u16, val, None)?,
            }
        }
    }

    // Auto-width columns
    for i in 0..headers.len() {
        sheet.set_column_width(i as u16, 15.0)?;
    }

    sheet.set_freeze_panes(1, 0)?;

    workbook.save(path).context("Failed to save export")?;
    Ok(())
}

fn cell_to_string(cell: Option<&calamine::Data>) -> String {
    match cell {
        Some(calamine::Data::String(s)) => s.trim().to_string(),
        Some(calamine::Data::Float(f)) => {
            if *f == (*f as i64) as f64 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Some(calamine::Data::Int(i)) => format!("{}", i),
        _ => String::new(),
    }
}

fn looks_like_description(s: &str) -> bool {
    if s.is_empty() { return true; }

    // Check for Chinese characters (Unicode Han range)
    if s.chars().any(|c| {
        let cp = c as u32;
        (0x4E00..=0x9FFF).contains(&cp) || (0x3400..=0x4DBF).contains(&cp)
    }) {
        return true;
    }

    // Check for keywords
    let upper = s.to_uppercase();
    if upper.contains("IP") || upper.contains("SSH") || upper.contains("LOCAL") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_description() {
        assert!(looks_like_description(""));
        assert!(looks_like_description("这是一台服务器"));
        assert!(looks_like_description("SSH Connection"));
        assert!(looks_like_description("LOCAL MACHINE"));
        assert!(!looks_like_description("10.0.0.1"));
        assert!(!looks_like_description("web-server"));
    }
}
```

- [ ] **Step 7: Create minishell-cli crate**

```toml
# crates/minishell-cli/Cargo.toml
[package]
name = "minishell-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "minishell"
path = "src/main.rs"

[dependencies]
minishell-core = { workspace = true }
minishell-store = { workspace = true }
minishell-ssh = { workspace = true }
minishell-tui = { workspace = true }
minishell-xlsx = { workspace = true }
clap = { version = "4", features = ["derive"] }
dirs = "5"
anyhow = "1"
```

```rust
// crates/minishell-cli/src/main.rs
use std::io::Write;
use std::path::PathBuf;
use anyhow::{Result, Context};
use clap::{Parser, Subcommand};
use minishell_core::Machine;
use minishell_store::Store;

#[derive(Parser)]
#[command(name = "minishell", version, about = "SSH Machine Management TUI Tool")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Query for quick login
    query: Option<String>,

    /// Skip TUI, use CLI mode
    #[arg(long)]
    no_tui: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Print version info
    Version,

    /// Generate import template
    Tpl {
        /// Output path
        path: Option<String>,
    },

    /// Import machines from Excel
    Import {
        /// Excel file path
        path: String,
    },

    /// Export machines to Excel
    Export {
        /// Output path
        path: Option<String>,
    },

    /// Show all machines
    Show,
}

fn db_path() -> PathBuf {
    PathBuf::from("/tmp/minishell")
}

fn open_db() -> Result<Store> {
    let path = db_path();
    let store = Store::open(&path)?;
    store.init()?;
    Ok(store)
}

fn can_use_tui() -> bool {
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
}

fn print_machines(machines: &[Machine]) {
    let header = format!("{:>4}  {:<15}  {:<15}  {:>5}  {:<10}  {:<15}  {:<20}  {:<10}  {:<20}",
        "#", "IP", "NAT-IP", "Port", "User", "Password", "Key", "Device", "Remark");
    println!("{}", header);
    println!("{}", "-".repeat(120));

    for (i, m) in machines.iter().enumerate() {
        let or_dash = |s: &str| if s.is_empty() || s == "-" { "-".to_string() } else { s.to_string() };
        println!("{:>4}  {:<15}  {:<15}  {:>5}  {:<10}  {:<15}  {:<20}  {:<10}  {:<20}",
            i + 1,
            m.ip,
            or_dash(&m.nat_ip),
            m.port,
            m.username,
            or_dash(&m.password),
            or_dash(&m.private_key_path),
            or_dash(&m.device),
            or_dash(&m.remark),
        );
    }
}

fn quick_login(query: &str) -> Result<()> {
    let store = open_db()?;

    // Try exact match by ID
    if let Ok(id) = query.parse::<i64>() {
        let machines = store.search("")?;
        if let Some(m) = machines.iter().find(|m| m.id == id) {
            minishell_ssh::login_to_machine(m)?;
            return Ok(());
        }
    }

    // Try exact match by IP
    let machines = store.search(query)?;
    if machines.len() == 1 {
        minishell_ssh::login_to_machine(&machines[0])?;
        return Ok(());
    }

    if machines.is_empty() {
        anyhow::bail!("No machines found matching '{}'", query);
    }

    // Multiple matches - try selector if TUI available
    if can_use_tui() {
        if let Some(selected) = minishell_tui::select_machine(machines)? {
            minishell_ssh::login_to_machine(&selected)?;
        }
    } else {
        println!("Multiple matches for '{}':", query);
        print_machines(&machines);
        anyhow::bail!("Please refine your search or use TUI mode");
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Version) => {
            println!("minishell {}", env!("CARGO_PKG_VERSION"));
            println!("git: {}", option_env!("GIT_SHA").unwrap_or("unknown"));
            println!("built: {}", option_env!("BUILD_TIME").unwrap_or("unknown"));
        }
        Some(Commands::Tpl { path }) => {
            let path = path.map(PathBuf::from).unwrap_or_else(|| {
                let bin_dir = std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."));
                bin_dir.join("machines-template.xlsx")
            });
            minishell_xlsx::generate_template(&path)?;
            println!("Template generated: {}", path.display());
        }
        Some(Commands::Import { path }) => {
            let store = open_db()?;
            let machines = minishell_xlsx::import_from(PathBuf::from(&path).as_path())?;
            let count = store.import_machines(&machines)?;
            println!("Imported {} machines ({} skipped)", count, machines.len() - count);
        }
        Some(Commands::Export { path }) => {
            let store = open_db()?;
            let machines = store.search("")?;
            let path = path.map(PathBuf::from).unwrap_or_else(|| {
                let bin_dir = std::env::current_exe().ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."));
                bin_dir.join("machines-export.xlsx")
            });
            minishell_xlsx::export_to(&path, &machines)?;
            println!("Exported {} machines to {}", machines.len(), path.display());
        }
        Some(Commands::Show) => {
            let store = open_db()?;
            let machines = store.search("")?;
            print_machines(&machines);
        }
        None => {
            if let Some(query) = cli.query {
                quick_login(&query)?;
            } else if cli.no_tui || !can_use_tui() {
                let store = open_db()?;
                let machines = store.search("")?;
                print_machines(&machines);
            } else {
                let store = open_db()?;
                let store = std::sync::Arc::new(store);
                if let Some(machine) = minishell_tui::run(store)? {
                    minishell_ssh::login_to_machine(&machine)?;
                }
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 8: Verify compilation**

Run: `cargo check --workspace`
Expected: All crates compile successfully

- [ ] **Step 9: Run tests**

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: implement minishell-rust - full 1:1 replication of Go minishell"
```
