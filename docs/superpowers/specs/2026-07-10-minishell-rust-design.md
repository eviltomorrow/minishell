# minishell-rust Design Spec

## Overview

1:1 Rust replication of the Go `minishell` SSH machine management TUI tool. Uses ratatui for TUI, ssh2 for SSH connections, rusqlite for persistence, calamine for Excel reading, rust_xlsxwriter for Excel writing, and clap for CLI.

## Project Structure

```
minishell-rust/
  Cargo.toml                 (workspace root)
  crates/
    minishell-core/          (Machine model, constants)
    minishell-store/         (SQLite CRUD via rusqlite)
    minishell-ssh/           (SSH connection via ssh2, terminal cards)
    minishell-tui/           (ratatui TUI: table, forms, search, selector)
    minishell-xlsx/          (Excel import via calamine, export via rust_xlsxwriter)
    minishell-cli/           (clap CLI entry point, binary)
```

### Dependencies

| Crate | Purpose |
|-------|---------|
| ratatui | Terminal UI framework |
| crossterm | Terminal backend, raw mode, events |
| rusqlite | SQLite database |
| ssh2 | SSH connections (wraps libssh2) |
| calamine | Excel read (import) |
| rust_xlsxwriter | Excel write (export, template) |
| clap | CLI argument parsing |
| dirs | Home directory resolution |

### Build Variables

Set via `env!()` macro at compile time:

- `APP_NAME` = "minishell"
- `MAIN_VERSION` = from Cargo.toml
- `GIT_SHA` = git rev-parse HEAD
- `BUILD_TIME` = chrono Utc::now()

## Data Model (minishell-core)

```rust
pub const NOT_EXIST: &str = "-";

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
    /// Returns nat_ip if non-empty and not "-", else ip
    pub fn effective_host(&self) -> &str;

    /// Returns true if field is empty or equals NOT_EXIST
    pub fn is_empty_field(s: &str) -> bool;
}
```

## Store (minishell-store)

### Database

- Path: `/tmp/minishell` (directory auto-created)
- SQLite with single connection (`SetMaxOpenConns(1)` equivalent)
- Schema:

```sql
CREATE TABLE IF NOT EXISTS machines (
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_machines_ip_port ON machines(ip, port);
```

### API

```rust
pub struct Store { conn: Connection }

impl Store {
    pub fn open(path: &Path) -> Result<Store>;
    pub fn init(&self) -> Result<()>;
    pub fn search(&self, query: &str) -> Result<Vec<Machine>>;  // LIKE %query% on ip/remark, empty = all
    pub fn count_all(&self) -> Result<usize>;
    pub fn import_machines(&self, machines: &[Machine]) -> Result<usize>;  // INSERT OR IGNORE
    pub fn update_machine(&self, m: &Machine) -> Result<()>;
    pub fn delete_machine(&self, id: i64) -> Result<()>;
}
```

## SSH (minishell-ssh)

### Connection

```rust
pub struct ConnectConfig {
    pub username: String,
    pub password: String,
    pub private_key_path: String,
    pub host: String,
    pub port: i32,
    pub timeout: Duration,
}

/// Interactive SSH session with PTY. Blocks until session ends.
pub fn connect(config: &ConnectConfig) -> Result<()>;
```

### Connection Flow

1. Build `ssh2::Session` with TCP connect
2. Authenticate: try private key first, then keyboard-interactive, then password
3. Set raw terminal mode via crossterm
4. Open PTY session with terminal size
5. Set terminal modes: ECHOCTL=1, ECHO=1, speed=14400
6. Use `$TERM` env var (default `xterm-256color`)
7. Request PTY on remote
8. Spawn threads: remote stdout->local stdout, remote stderr->local stderr, local stdin->remote stdin
9. After 1s delay, inject PS1 prefix: `export PS1="[host] $PS1"`
10. Wait for session to end

### Host Key Verification

- Load `~/.ssh/known_hosts`
- If unknown: display fingerprint, prompt "Are you sure you want to continue connecting (yes/no)?"
- Save key on "yes"

### Supported Ciphers

aes128-ctr, aes192-ctr, aes256-ctr, aes128-gcm@openssh.com, arcfour256, arcfour128, aes128-cbc

### Supported Key Exchanges

diffie-hellman-group-exchange-sha1, diffie-hellman-group1-sha1, diffie-hellman-group-exchange-sha256, diffie-hellman-group16-sha512, diffie-hellman-group18-sha512, diffie-hellman-group14-sha256, diffie-hellman-group14-sha1, curve25519-sha256, kex-strict-s-v00@openssh.com

### Login Orchestration

```rust
/// Connects to machine, shows connect/disconnect cards, returns session duration
pub fn login_to_machine(machine: &Machine) -> Result<Duration>;
```

Flow:
1. Select host: nat_ip if set, else ip
2. Resolve `~/` in key path to `$HOME`
3. Set terminal title to host IP via ANSI escape
4. Display ConnectCardTop with connection details
5. Show "Connecting..." status
6. Call `connect()` with 10s timeout
7. Update status to success/failure
8. Display DisconnectCard with duration
9. Reset terminal title to "minishell"

### Terminal Cards (ASCII Art)

ANSI color variables:
- `cardLineColor`: blue (38;5;39)
- `cardLabel`: light blue (38;5;87)
- `cardGreen`: green (38;5;42)
- `cardRed`: red (38;5;196)
- `cardFaint`: faint gray (38;5;243)
- `cardBold`: bold
- `cardReset`: reset

Card functions:
- `card_line(width, content)` -> `│ content padded │`
- `card_border(width, corner_left, corner_right)` -> `┌─────┐`
- `connect_card_top(ip, host, port, username, auth_method, width)` -> full card with "SSH CONNECT" title
- `connect_card_status_line(content, width)` -> single status line
- `connect_card_bottom(width)` -> `└─────┘`
- `connect_success_line(duration)` -> green checkmark + "Connected in 123ms"
- `connect_fail_line(err)` -> red X + error message
- `disconnect_card(host, duration, ssh_err, width)` -> "DISCONNECTED" card with formatted duration

## TUI (minishell-tui)

### Architecture

Manual event loop with crossterm `EventStream`. Elm-like pattern:

```rust
pub struct AppState {
    store: Arc<Store>,
    machines: Vec<Machine>,
    table: MachineTable,
    search_input: String,
    search_focused: bool,
    show_secrets: bool,
    form: Option<FormState>,      // Add or Edit
    delete_confirm: Option<DeleteState>,
    should_quit: bool,
    login_target: Option<Machine>,
    terminal_size: (u16, u16),
}

pub fn run(store: Arc<Store>) -> Result<Option<Machine>>;
```

### Event Loop

```rust
loop {
    terminal.draw(|f| view(f, &state))?;
    if let Event::Key(key) = event::read().await? {
        update(&mut state, key);
    }
    if state.should_quit {
        break;
    }
}
// If login_target is set, return it for ssh::login_to_machine()
```

### Components

#### MachineTable

Custom table widget with cursor-centered scrolling.

```rust
struct MachineTable {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
    cursor: usize,
    width: u16,
    height: u16,  // visible rows (excluding header)
}
```

Columns (default, 7):
| Title | Width |
|-------|-------|
| `#` | 4 |
| `IP` | 15 |
| `""` | 2 (spacer) |
| `NAT` | 12 |
| `User` | 10 |
| `Device` | 10 |
| `Remark` | 20 |

Columns (with secrets, 10):
| Title | Width |
|-------|-------|
| `#` | 4 |
| `IP` | 15 |
| `""` | 2 (spacer) |
| `NAT` | 12 |
| `User` | 10 |
| `Password` | 15 |
| `""` | 2 (spacer) |
| `Key` | 20 |
| `Device` | 10 |
| `Remark` | 20 |

Methods:
- `set_size(w, h)`
- `set_rows(rows)`
- `cursor()` / `set_cursor(n)`
- `move_up()` / `move_down()` / `goto_top()` / `goto_bottom()`
- `rows_count()`

Rendering:
- Header row with `headerStyle` (purple, bold)
- Cursor-centered scrolling: `top = cursor - h/2`
- Selected row: light gray background, pink text
- Wide-character-aware padding via `unicode_width`

#### SearchBar

- Focus on `/` key
- Blur on `Esc` (clears input) or `Enter` (commits search)
- Updates trigger `store.search(query)` and rebuild table

#### AddForm

8 text input fields in rounded purple border (width 50):

1. IP: (limit 64, width 40)
2. NAT-IP: (limit 64, width 40)
3. Port: (placeholder "22", limit 5, width 10)
4. Username: (limit 64, width 40)
5. Password: (limit 64, width 40)
6. PrivateKey: (limit 64, width 40)
7. Device: (limit 64, width 40)
8. Remark: (limit 64, width 40)

Navigation: `↑↓`/`Tab` next field, `Enter` on last field saves, `Esc` cancels.

#### EditForm

Same as AddForm but pre-filled from selected machine. Saves via `store.update_machine()`.

#### DeleteConfirm

Red bordered box (width 40):
- Title: "Delete Machine"
- Subtitle: "Remove IP (user)?"
- Keys: `y` yes, `n` no, `Esc` cancel

#### StatusBar

Left-to-right: `[selected/total]` | `[username@ip:port]` or `[search: query]` | `[secrets show/hide]`

#### HelpBar

Bottom bar: `↑↓` sel | `↵` login | `e` edit | `a` add | `d` del | `s` secrets | `/` search | `q` quit

#### Selector

Mini TUI for quick login multi-match. Table with columns: `>` (cursor `▸`), `#`, `IP`, `NAT-IP`, `Port`, `User`, `Remark`. Keys: `↑↓`/`j`/`k` navigate, `Enter` select, `q`/`Esc`/`Ctrl+C` quit.

### Styles

| Style | Color | Usage |
|-------|-------|-------|
| header | Purple (63), Bold | Table headers, title |
| main_border | Dark gray (240) | Main container |
| help | Faint gray (243) | Help text |
| search | Cyan (42), Bold | Search prompt |
| form_box | Purple rounded border | Add/Edit form |
| delete_box | Red rounded border | Delete confirmation |
| form_field | Pink (212), Bold | Active form field |
| separator | Faint gray | Separator line |
| key | Cyan (42), Bold | Keyboard shortcuts |
| status | Faint gray | Status bar |
| selected | Light gray bg, pink text | Selected table row |

### Key Bindings (Normal Mode)

| Key | Action |
|-----|--------|
| `↑`/`k` | Move cursor up |
| `↓`/`j` | Move cursor down |
| `PgUp` | Move cursor up 10 rows |
| `PgDn` | Move cursor down 10 rows |
| `g` | Go to top |
| `G` | Go to bottom |
| `Enter` | Login to selected machine |
| `/` | Focus search |
| `a` | Open add form |
| `e` | Open edit form |
| `d` | Open delete confirmation |
| `s` | Toggle secrets visibility |
| `q`/`Ctrl+C` | Quit |

## Excel (minishell-xlsx)

Uses `calamine` for reading and `rust_xlsxwriter` for writing.

### Template Generation (rust_xlsxwriter)

```rust
pub fn generate_template(path: &Path) -> Result<()>;
```

Creates xlsx with headers: `IP`, `NAT-IP`, `Port`, `Username`, `Password`, `PrivateKey-Path`, `Device`, `Remark`
One example row: `10.0.0.1`, `-`, 22, `root`, `your-password`, `-`, `Linux`, `example`

### Import (calamine)

```rust
pub fn import_from(path: &Path) -> Result<Vec<Machine>>;
```

1. Open xlsx, read "Sheet1"
2. Require at least 2 rows (header + data)
3. Skip header row
4. Skip empty rows and rows that look like descriptions:
   - Empty strings skipped
   - Strings containing Chinese characters (Unicode Han range) = description
   - Strings containing "IP", "SSH", "LOCAL" (case-insensitive) = description
5. Map columns: 0=IP, 1=NAT-IP, 2=Port, 3=Username, 4=Password, 5=PrivateKeyPath, 6=Device, 7=Remark

### Export (rust_xlsxwriter)

```rust
pub fn export_to(path: &Path, machines: &[Machine]) -> Result<()>;
```

Styled xlsx:
- Header: Bold white on blue (#4472C4), centered, borders
- Data: Alternating white/#D9E2F3 stripes with thin borders
- Auto-calculated column widths (clamped 10-40)
- Frozen header row

## CLI (minishell-cli)

### Commands

| Command | Description |
|---------|-------------|
| (default) | Launch TUI, or quick login if query arg provided |
| `version` | Print version info |
| `tpl [path]` | Generate import template (default: `<bindir>/machines-template.xlsx`) |
| `import <path>` | Import machines from xlsx |
| `export [path]` | Export machines to xlsx (default: `<bindir>/machines-export.xlsx`) |
| `show` | Print machines table to stdout |

### Flags

| Flag | Description |
|------|-------------|
| `--no-tui` | Force CLI mode, skip TUI |

### Quick Login Flow

1. Open DB, search all machines
2. Try exact match: by ID (if numeric), then by IP, then by Remark
3. If no exact match: fuzzy search via `store.search(query)`
4. If 1 match: directly `ssh::login_to_machine()`
5. If multiple matches + TUI available: launch `selector` mini TUI
6. If multiple matches + no TUI: print table + error

### TUI Check

Returns false if `TERM=dumb` or stdout is not a terminal (check via `atty` or `std::io::IsTerminal`).

### Table Output (show, no-TUI)

Formatted table using similar styling to Go version:
Columns: `#`, `IP`, `NAT-IP`, `Port`, `User`, `Password`, `Key`, `Device`, `Remark`
Empty/"-" values displayed as `-`

## File Layout

```
crates/minishell-core/
  src/lib.rs              -- Machine struct, NOT_EXIST constant
  Cargo.toml

crates/minishell-store/
  src/lib.rs              -- Store struct, all CRUD methods
  Cargo.toml

crates/minishell-ssh/
  src/lib.rs              -- connect(), login_to_machine()
  src/card.rs             -- ASCII card rendering
  Cargo.toml

crates/minishell-tui/
  src/lib.rs              -- run() entry point
  src/app.rs              -- AppState, update(), view()
  src/table.rs            -- MachineTable widget
  src/form.rs             -- AddForm, EditForm, DeleteConfirm
  src/selector.rs         -- SelectMachine mini TUI
  src/styles.rs           -- All ratatui Style definitions
  Cargo.toml

crates/minishell-xlsx/
  src/lib.rs              -- generate_template, import_from, export_to (calamine + rust_xlsxwriter)
  Cargo.toml

crates/minishell-cli/
  src/main.rs             -- clap CLI, defaultAction, quickLogin, printMachines
  Cargo.toml
```
