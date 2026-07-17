# Filebrowser Module Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `filebrowser.rs` (1676 lines) into 6 focused modules with clear responsibilities.

**Architecture:** Incremental split — create module structure, move types, move methods, verify compilation at each step.

**Tech Stack:** Rust, ratatui, crossterm, ssh2

## Global Constraints

- Rust edition 2021
- All changes must pass `cargo check` and `cargo test`
- No functionality changes — pure refactoring
- Follow existing code conventions

---

### Task 1: Create module structure

**Files:**
- Create: `crates/minishell-tui/src/filebrowser/mod.rs`
- Create: `crates/minishell-tui/src/filebrowser/panel.rs`
- Create: `crates/minishell-tui/src/filebrowser/tree.rs`
- Create: `crates/minishell-tui/src/filebrowser/transfer.rs`
- Create: `crates/minishell-tui/src/filebrowser/operations.rs`
- Create: `crates/minishell-tui/src/filebrowser/render.rs`

**Interfaces:**
- Produces: Empty module files with correct `mod` declarations

- [ ] **Step 1: Create directory**

```bash
mkdir -p crates/minishell-tui/src/filebrowser
```

- [ ] **Step 2: Create mod.rs stub**

```rust
// mod.rs will be populated in later tasks
```

- [ ] **Step 3: Create panel.rs stub**

```rust
// panel.rs will be populated in later tasks
```

- [ ] **Step 4: Create tree.rs stub**

```rust
// tree.rs will be populated in later tasks
```

- [ ] **Step 5: Create transfer.rs stub**

```rust
// transfer.rs will be populated in later tasks
```

- [ ] **Step 6: Create operations.rs stub**

```rust
// operations.rs will be populated in later tasks
```

- [ ] **Step 7: Create render.rs stub**

```rust
// render.rs will be populated in later tasks
```

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS (warnings about unused files)

- [ ] **Step 9: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/
git commit -m "refactor(tui): create filebrowser module structure"
```

---

### Task 2: Move Side enum to mod.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/mod.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: `Side` enum from `filebrowser.rs`
- Produces: `Side` enum in `filebrowser/mod.rs`

- [ ] **Step 1: Copy Side enum to mod.rs**

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Local,
    Remote,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Local => Side::Remote,
            Side::Remote => Side::Local,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Local => "LOCAL",
            Side::Remote => "REMOTE",
        }
    }
}
```

- [ ] **Step 2: Remove Side from filebrowser.rs**

Delete lines 16-36 from `filebrowser.rs`

- [ ] **Step 3: Add use statement in filebrowser.rs**

Add at top of `filebrowser.rs`:
```rust
use super::filebrowser::Side;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/mod.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move Side enum to filebrowser/mod.rs"
```

---

### Task 3: Move PanelState to panel.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/panel.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: `PanelState` struct from `filebrowser.rs`
- Produces: `PanelState` struct in `filebrowser/panel.rs`

- [ ] **Step 1: Copy PanelState struct to panel.rs**

```rust
use std::path::PathBuf;
use minishell_ssh::sftp::FileEntry;

pub struct PanelState {
    pub entries: Vec<FileEntry>,
    pub cursor: usize,
    pub scroll_offset: usize,
    pub current_path: PathBuf,
    pub prev_dir_name: Option<String>,
    pub tree_entries: Vec<TreeEntry>,
    pub expanded_dirs: Vec<PathBuf>,
}

impl PanelState {
    pub fn new(path: PathBuf) -> Self {
        PanelState {
            entries: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            current_path: path,
            prev_dir_name: None,
            tree_entries: Vec::new(),
            expanded_dirs: Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Remove PanelState from filebrowser.rs**

Delete lines 43-65 from `filebrowser.rs`

- [ ] **Step 3: Add use statement in filebrowser.rs**

Add at top of `filebrowser.rs`:
```rust
use super::filebrowser::panel::PanelState;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/panel.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move PanelState to filebrowser/panel.rs"
```

---

### Task 4: Move TreeEntry to tree.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/tree.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: `TreeEntry` struct from `filebrowser.rs`
- Produces: `TreeEntry` struct in `filebrowser/tree.rs`

- [ ] **Step 1: Copy TreeEntry struct to tree.rs**

```rust
use minishell_ssh::sftp::FileEntry;

pub struct TreeEntry {
    pub entry: FileEntry,
    pub depth: usize,
}
```

- [ ] **Step 2: Remove TreeEntry from filebrowser.rs**

Delete lines 38-41 from `filebrowser.rs`

- [ ] **Step 3: Add use statement in filebrowser.rs**

Add at top of `filebrowser.rs`:
```rust
use super::filebrowser::tree::TreeEntry;
```

- [ ] **Step 4: Update panel.rs imports**

Update `panel.rs` to import `TreeEntry`:
```rust
use super::tree::TreeEntry;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/tree.rs crates/minishell-tui/src/filebrowser/panel.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move TreeEntry to filebrowser/tree.rs"
```

---

### Task 5: Move transfer structs to transfer.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/transfer.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: `TransferProgressState`, `ActionResult`, `PendingTransfer` from `filebrowser.rs`
- Produces: Structs in `filebrowser/transfer.rs`

- [ ] **Step 1: Copy structs to transfer.rs**

```rust
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use super::Side;

pub struct TransferProgressState {
    pub file_name: String,
    pub bytes: u64,
    pub total: u64,
}

pub enum ActionResult {
    TransferDone(Side),
    Error(String),
}

pub struct PendingTransfer {
    pub progress: Arc<Mutex<TransferProgressState>>,
    pub done_rx: Receiver<ActionResult>,
}
```

- [ ] **Step 2: Remove structs from filebrowser.rs**

Delete lines 67-81 from `filebrowser.rs`

- [ ] **Step 3: Add use statement in filebrowser.rs**

Add at top of `filebrowser.rs`:
```rust
use super::filebrowser::transfer::{TransferProgressState, ActionResult, PendingTransfer};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/transfer.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move transfer structs to filebrowser/transfer.rs"
```

---

### Task 6: Move panel methods to panel.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/panel.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: Panel methods from `filebrowser.rs`
- Produces: Methods on `PanelState` in `panel.rs`

- [ ] **Step 1: Copy methods to panel.rs**

Move these methods from `filebrowser.rs` to `panel.rs`:
- `move_cursor()`
- `cursor_first()`
- `cursor_last()`
- `enter_dir()`
- `parent_dir()`
- `goto_root()`
- `goto_home()`
- `toggle_side()`

- [ ] **Step 2: Remove methods from filebrowser.rs**

Delete the method implementations from `filebrowser.rs`

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/panel.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move panel methods to filebrowser/panel.rs"
```

---

### Task 7: Move tree methods to tree.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/tree.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: Tree methods from `filebrowser.rs`
- Produces: Methods on `FileBrowserState` in `tree.rs`

- [ ] **Step 1: Copy methods to tree.rs**

Move these methods from `filebrowser.rs` to `tree.rs`:
- `toggle_tree()`
- `rebuild_panel_tree()`
- `append_tree_entries()`
- `sync_tree()`
- `tree_entry_full_path()`

- [ ] **Step 2: Remove methods from filebrowser.rs**

Delete the method implementations from `filebrowser.rs`

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/tree.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move tree methods to filebrowser/tree.rs"
```

---

### Task 8: Move transfer methods to transfer.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/transfer.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: Transfer methods from `filebrowser.rs`
- Produces: Methods on `FileBrowserState` in `transfer.rs`

- [ ] **Step 1: Copy methods to transfer.rs**

Move these methods from `filebrowser.rs` to `transfer.rs`:
- `start_transfer()`
- `init_transfer()`
- `upload_selected()`
- `download_selected()`
- `start_transfer_confirm()`
- `confirm_transfer()`

- [ ] **Step 2: Remove methods from filebrowser.rs**

Delete the method implementations from `filebrowser.rs`

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/transfer.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move transfer methods to filebrowser/transfer.rs"
```

---

### Task 9: Move operations methods to operations.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/operations.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: Operations methods from `filebrowser.rs`
- Produces: Methods on `FileBrowserState` in `operations.rs`

- [ ] **Step 1: Copy methods to operations.rs**

Move these methods from `filebrowser.rs` to `operations.rs`:
- `start_delete()`
- `confirm_delete_action()`
- `start_rename()`
- `confirm_rename()`

- [ ] **Step 2: Remove methods from filebrowser.rs**

Delete the method implementations from `filebrowser.rs`

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/operations.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move operations methods to filebrowser/operations.rs"
```

---

### Task 10: Move render methods to render.rs

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/render.rs`
- Modify: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: Render methods from `filebrowser.rs`
- Produces: Methods on `FileBrowserState` in `render.rs`

- [ ] **Step 1: Copy methods to render.rs**

Move these methods from `filebrowser.rs` to `render.rs`:
- `render()`
- `render_panel()`
- `render_name_and_path()`
- `format_size()`
- `pad_left()`
- `pad_right()`
- `truncate_to_width()`

- [ ] **Step 2: Remove methods from filebrowser.rs**

Delete the method implementations from `filebrowser.rs`

- [ ] **Step 3: Update imports in render.rs**

```rust
use minishell_utils::{format_size, pad_left, pad_right, truncate_to_width};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/render.rs crates/minishell-tui/src/filebrowser.rs
git commit -m "refactor(tui): move render methods to filebrowser/render.rs"
```

---

### Task 11: Final cleanup and verification

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser/mod.rs`
- Delete: `crates/minishell-tui/src/filebrowser.rs`

**Interfaces:**
- Consumes: All modules
- Produces: Clean module structure

- [ ] **Step 1: Update mod.rs with re-exports**

```rust
pub mod panel;
pub mod tree;
pub mod transfer;
pub mod operations;
pub mod render;

pub use panel::PanelState;
pub use tree::TreeEntry;
pub use transfer::{TransferProgressState, ActionResult, PendingTransfer};
pub use Side;
```

- [ ] **Step 2: Delete old filebrowser.rs**

```bash
rm crates/minishell-tui/src/filebrowser.rs
```

- [ ] **Step 3: Update app.rs imports**

Update `crates/minishell-tui/src/app.rs` to use new module structure:
```rust
use super::filebrowser::FileBrowserState;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minishell-tui`
Expected: PASS

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/minishell-tui/src/filebrowser/mod.rs crates/minishell-tui/src/filebrowser.rs crates/minishell-tui/src/app.rs
git commit -m "refactor(tui): complete filebrowser module split"
```

---

## Plan Complete

Plan saved to `docs/superpowers/plans/2026-07-17-filebrowser-split.md`.

**Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
