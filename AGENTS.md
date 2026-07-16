# minishell — agent guide

## Build & test

```bash
cargo build --release          # binary: target/release/minishell
cargo test                     # tests in core, store, xlsx crates
cargo test -p minishell-store  # single crate
cargo check                    # fast compile check
```

No CI, no linter/formatter config, no pre-commit hooks, no Makefile.

## Workspace

6 crates under `crates/`:

| Crate | Responsibility | Deps |
|---|---|---|
| `minishell-core` | `Machine` model, `NOT_EXIST` constant | none |
| `minishell-store` | SQLite CRUD (`$HOME/.minishell/db`, WAL mode) | rusqlite (bundled) |
| `minishell-ssh` | SSH PTY session via `libc::poll` (not threads), connect cards | ssh2, crossterm, libc |
| `minishell-tui` | ratatui app, custom table, form, selector | ratatui, crossterm (event-stream) |
| `minishell-xlsx` | Excel import (calamine) / export (rust_xlsxwriter) | calamine, rust_xlsxwriter |
| `minishell-cli` | Binary entrypoint (`[[bin]] name = "minishell"`), clap CLI | all of the above |

All crates depend on `minishell-core`. `minishell-cli` depends on all 5 others.

## Key architecture facts

- **No async runtime** — blocking crossterm `event::read()` in the TUI loop
- **SSH I/O** uses raw `libc::poll()` on stdin_fd + session_fd (not threads as the design doc spec says)
- **DB path**: `~/.minishell/db` (from `main.rs` `db_path()` — NOT `/tmp/minishell` as older docs claim)
- **Store** uses `unchecked_transaction()` for batch imports, `INSERT OR IGNORE` (unique on ip+port)
- **Selector** (multi-match quick login) uses raw ANSI escape sequences inline, not ratatui
- **Password/key visibility** toggleable with `s` key — column layout changes dynamically
- **Edit form** sets empty fields to `"-"` sentinel; edit pre-fills from existing values
- **`format_machine_row`** uses `m.num` for the `#` column
- **Session timeout**: 1 hour hard limit in SSH connect loop

## CLI

```bash
minishell                    # TUI mode
minishell <ip|remark|id>     # quick login (single match → direct, multi → selector)
minishell show               # table to stdout
minishell import <file.xlsx>
minishell export [path]
minishell tpl [path]
minishell version
```

## Version info

Built with `env!("CARGO_PKG_VERSION")`, compile-time `GIT_SHA` and `BUILD_TIME` via `option_env!()`.

## TUI quirks

- Panic hook disables raw mode and leaves alternate screen
- Form field cursor is byte-based (not char-count), uses `char_indices()` for navigation
- Search filters in real-time (no debounce); `Enter` commits, `Esc` clears and unfocuses
- Dialog overlays are positioned above the status bar with a gap, not centered vertically
- `login_target` set + `should_quit = true` on Enter (TUI exits before SSH connects)
