# File Browser Tree View — Design Spec

Date: 2026-07-16

## Overview

Add a tree view mode to the existing SFTP file browser (`filebrowser.rs`). Pressing `t` toggles between the current flat list view and a 2-level deep tree expansion of the current directory. Navigation (up/down/enter/parent) works in tree mode; upload/download/delete/rename are disabled.

## Architecture

### Changes are scoped to `minishell-tui/filebrowser.rs`

No changes to `FileEntry` in `minishell-ssh/sftp.rs`. All tree logic is internal to `FileBrowserState`/`PanelState`.

## Data Structures

Add to `PanelState`:

```rust
struct TreeEntry {
    entry: FileEntry,
    depth: usize,  // 0=top level, 1=subdir level, 2=grandchild level
}

struct PanelState {
    // existing fields unchanged
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll_offset: usize,
    current_path: PathBuf,
    prev_dir_name: Option<String>,

    // new fields
    tree_mode: bool,
    tree_entries: Vec<TreeEntry>,
}
```

- `tree_mode` `false` by default — no rendering or behavior change
- `tree_entries` populated only when `tree_mode` is `true`
- `FileEntry` unchanged — no `depth` field added to the shared model

## Interaction

### Toggle (`t` key)

| Transition | Action |
|---|---|
| `tree_mode: false` → `true` | Back up current entries (used only for restoration on `t` again). Recursively list contents of current directory up to 2 levels, build `tree_entries`. Set cursor to 0. |
| `tree_mode: true` → `false` | Clear `tree_entries`, re-run `refresh_panel()` (returns to normal flat list). Cursor resets via existing panel refresh logic. |

### Navigation

All keys work **except** file-transfer/rename/delete:

| Key | Behavior in Tree Mode |
|---|---|
| `↑` / `↓` | Move cursor in `tree_entries` list normally |
| `PageUp` / `PageDown` | Jump to first/last (works on `tree_entries`) |
| `Enter` / `Right` on directory | Enter selected directory, **exit tree mode**, refresh flat list of new path |
| `Enter` / `Right` on file | Show file info in status bar (same as current behavior) |
| `Left` / `Esc` | Go to parent directory, **exit tree mode**, refresh flat list |
| `Tab` | Toggle active side (turns tree mode off for the newly active panel) |
| `t` | Toggle back to flat view (stay in same directory) |
| `u` / `d` / `x` / `r` | Disabled — silently ignored, or show "Tree mode" hint in status bar |
| `q` | Exit file browser (works normally) |

Rationale for exiting tree mode on Enter/Left: the user enters a new directory (or goes up), so the tree for the _previous_ directory is stale. The new directory should display fresh in flat mode — user can press `t` again if they want its tree.

### Disabled Operations

In tree mode, pressing `u`, `d`, `x`, or `r`:
- Operation is silently ignored (no crash, no state change)
- Status bar may briefly show a hint like "Not available in tree mode" (optional, lower priority)

## Rendering

### Visual Format

```
📂 project/
  📂 src/
    📂 components/
    📄 main.rs
  📂 tests/
📂 docs/
📄 Cargo.toml
```

Changes to `render_panel`:

1. If `tree_mode` is true, iterate `tree_entries` instead of `entries` for the item list
2. Prepend `"  ".repeat(tree_entry.depth)` to the display name at render time
3. Column widths (perm, size, modified) remain unchanged — computed from `TreeEntry.entry` fields
4. All other rendering (header, column layout, separator, scroll, marker, selected style) unchanged

Example mapping:
- `depth=0`: name displays as `"📂 project/"`
- `depth=1`: name displays as `"  📂 src/"`
- `depth=2`: name displays as `"    📄 main.rs"`

### Scroll & Cursor in Tree Mode

Same scroll logic as flat mode — `scroll_offset` and `visible_rows` work identically. `cursor` indexes into `tree_entries`. Only the source of the entry list changes.

## Tree Building

### Local Side

```
fn build_tree_local(path: &Path, max_depth: usize) -> Vec<TreeEntry>
```

- Calls `std::fs::read_dir(path)`
- Filters out entries starting with `.`
- Sorts: directories first, then alphabetical
- For each directory with `depth < max_depth`, recurse with `depth + 1`
- For each file, emit as `TreeEntry { entry, depth }`
- Returns flat `Vec<TreeEntry>` in display order (parent, then children, then siblings)

### Remote Side

```
fn build_tree_remote(sftp: &Sftp, path: &str, max_depth: usize) -> Vec<TreeEntry>
```

- Same structure as local, but uses `sftp::list_dir` for each directory
- 2 levels means at most `1 + N + M` SFTP `readdir` calls (current dir + each subdir + each grandchild dir)

### Depth Limit

Hard-coded at 2 levels. Not configurable in this version. The `max_depth` parameter is passed through in case it's useful later, but the toggle always uses 2.

## Error Handling

- If tree building fails (e.g., SFTP connection lost mid-build), clear `tree_entries`, set `tree_mode = false`, show error in status bar, revert to flat view
- Partial results: if a subdirectory fails to list, skip that subtree and continue with siblings. Accumulate errors in a `Vec<String>` and show first error in status bar.

## Testing

Coverage for tree mode:

| Test | Scope |
|---|---|
| `build_tree_local` produces correct depth/order | Unit test (mock fs) |
| `build_tree_remote` produces correct depth/order | Unit test (mock sftp) |
| Toggle `t` enters and exits tree mode | Unit test on `FileBrowserState` |
| Navigation in tree entries (up/down, wrap) | Unit test on `FileBrowserState` |
| Enter on directory exits tree mode | Unit test |
| Upload/download/delete/rename disabled in tree mode | Unit test — verify no-op/status change |
| Partial failure (one subdir fails) doesn't crash | Error handling unit test |

No integration testing beyond existing SFTP test infrastructure.

## Out of Scope

- More than 2 levels of depth
- Configurable depth
- Tree connectors (├── └──) — indentation only
- Collapse/expand individual nodes (toggle is all-or-nothing per panel)
- Tree view on the CLI `show` command
- Multi-selection in tree mode
