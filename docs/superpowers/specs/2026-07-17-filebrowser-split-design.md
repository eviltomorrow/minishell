# Filebrowser Module Split Design

Date: 2026-07-17

## Overview

Split the monolithic `filebrowser.rs` (1676 lines) into 6 focused modules, each with a single responsibility. This improves code organization, maintainability, and testability.

## Current State

`crates/minishell-tui/src/filebrowser.rs` contains:
- `Side` enum
- `PanelState` struct
- `FileBrowserState` struct with 40+ methods
- Helper functions for rendering

## Target Structure

```
crates/minishell-tui/src/
  filebrowser/
    mod.rs           — Core state + public interface
    panel.rs         — Panel state management
    tree.rs          — Tree view logic
    transfer.rs      — File transfer operations
    operations.rs    — Delete and rename operations
    render.rs        — All rendering code
```

## Module Specifications

### Module 1: `mod.rs` — Core State + Public Interface

**Responsibilities:**
- Define `Side` enum
- Define `FileBrowserState` struct
- Provide public API: `new()`, `check_pending()`, `handle_key()`, `render()`
- Delegate to other modules

**Exports:**
- `Side` (enum)
- `FileBrowserState` (struct)

**Dependencies:**
- All other modules

**Estimated size:** ~150 lines

### Module 2: `panel.rs` — Panel State Management

**Responsibilities:**
- Define `PanelState` struct
- Cursor movement: `move_cursor()`, `cursor_first()`, `cursor_last()`
- Directory navigation: `enter_dir()`, `parent_dir()`, `goto_root()`, `goto_home()`
- Side switching: `toggle_side()`

**Exports:**
- `PanelState` (struct)

**Dependencies:**
- `tree.rs` (for `TreeEntry`)

**Estimated size:** ~200 lines

### Module 3: `tree.rs` — Tree View Logic

**Responsibilities:**
- Define `TreeEntry` struct
- Tree operations: `toggle_tree()`, `rebuild_panel_tree()`, `append_tree_entries()`
- Tree utilities: `sync_tree()`, `tree_entry_full_path()`

**Exports:**
- `TreeEntry` (struct)

**Dependencies:**
- None

**Estimated size:** ~200 lines

### Module 4: `transfer.rs` — File Transfer Operations

**Responsibilities:**
- Define `TransferProgressState`, `ActionResult`, `PendingTransfer` structs
- Transfer initiation: `start_transfer()`, `init_transfer()`
- Upload/download: `upload_selected()`, `download_selected()`
- Transfer confirmation: `start_transfer_confirm()`, `confirm_transfer()`

**Exports:**
- `TransferProgressState` (struct)
- `ActionResult` (enum)
- `PendingTransfer` (struct)

**Dependencies:**
- None

**Estimated size:** ~250 lines

### Module 5: `operations.rs` — Delete and Rename Operations

**Responsibilities:**
- Delete operations: `start_delete()`, `confirm_delete_action()`
- Rename operations: `start_rename()`, `confirm_rename()`

**Exports:**
- None (methods on `FileBrowserState`)

**Dependencies:**
- None

**Estimated size:** ~150 lines

### Module 6: `render.rs` — Rendering Code

**Responsibilities:**
- Main render: `render()` method
- Panel rendering: `render_panel()`
- Helper functions: `render_name_and_path()`, `format_size()`, `pad_left()`, `pad_right()`, `truncate_to_width()`

**Exports:**
- None (methods on `FileBrowserState`)

**Dependencies:**
- `minishell-utils` (for `format_size`, `pad_left`, `pad_right`, `truncate_to_width`)

**Estimated size:** ~350 lines

## Implementation Approach

### Step 1: Create module structure
- Create `filebrowser/` directory
- Create stub files for each module

### Step 2: Move types and structs
- Move `Side` to `mod.rs`
- Move `PanelState` to `panel.rs`
- Move `TreeEntry` to `tree.rs`
- Move transfer structs to `transfer.rs`

### Step 3: Move methods
- Move panel methods to `panel.rs`
- Move tree methods to `tree.rs`
- Move transfer methods to `transfer.rs`
- Move delete/rename methods to `operations.rs`
- Move render methods to `render.rs`

### Step 4: Update imports
- Add `use` statements in `mod.rs` to re-export types
- Update `app.rs` to use new module structure

### Step 5: Verify
- Run `cargo check`
- Run `cargo test`
- Verify no functionality changes

## Success Criteria

- [ ] `filebrowser.rs` split into 6 modules
- [ ] Each module < 350 lines
- [ ] All tests pass
- [ ] No functionality changes
- [ ] Clear module boundaries

## Out of Scope

- Changing functionality
- Adding new features
- Refactoring other parts of the codebase
