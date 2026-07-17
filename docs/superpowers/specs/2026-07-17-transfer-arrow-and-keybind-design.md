# Transfer Arrow Indicator & Key Remapping Design

Date: 2026-07-17

## Overview

Enhance the file transfer TUI (`filebrowser.rs`) with:
1. A visual arrow column between Local/Remote panels showing transfer direction
2. Key binding remapping: `x`=transfer, `d`=delete
3. Status bar Yes/No confirmation for transfers

## Current State

- **Layout**: 2-column 50/50 split (`Ratio(1,2)` × 2)
- **Keys**: `u`=upload (local only), `d`=download (remote only), `x`=delete (y/n confirm)
- **Status bar**: Left=status icon+text, Right=context-sensitive help hints
- **No visual direction indicator** — direction is implicit from key + active panel

## Layout Change: Arrow Column

### New 3-Column Layout

```
[   Local panel (~48%)   ][ ← ][   Remote panel (~48%)   ]
```

Implementation in `render()`:
```rust
let panels = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([
        Constraint::Min(10),    // local panel
        Constraint::Length(3),  // arrow gutter
        Constraint::Min(10),    // remote panel
    ])
    .split(chunks[1]);
```

The `Min(10)` constraints ensure panels share remaining space equally after the fixed 3-char arrow gutter.

### Arrow Gutter Rendering

A new `render_arrow()` method renders the 3-char wide column:
- Vertically centered at `inner.height / 2`
- Horizontally centered (1 char arrow + 1 char padding each side)

### Arrow States

| State | Symbol | Color |
|-------|--------|-------|
| Idle | `·` | DarkGray |
| Upload confirm (local active + x pressed) | `→` | Green |
| Download confirm (remote active + x pressed) | `←` | Green |
| Transfer in progress | `→` / `←` | Yellow |

## Key Binding Changes

| Action | Before | After |
|--------|--------|-------|
| Upload | `u` (local active only) | `x` (local active) |
| Download | `d` (remote active only) | `x` (remote active) |
| Delete | `x` → y/n confirm | `d` → y/n confirm |
| `u` key | Upload | No function |

- `x` blocked in tree mode (same as current `u`/`d` behavior)
- `d` (delete) blocked in tree mode
- Status bar help text updates: `x:transfer  d:delete` replaces `u:upload  d:download  x:del`

## Transfer Confirmation Flow

### New State: `TransferConfirm`

Added to the existing `Mode` enum alongside `DeleteConfirm` and `RenameInput`.

**Upload flow:**
```
1. User selects file in Local panel, presses x
2. Arrow column shows green →
3. Status bar: "Upload main.rs → remote?  [Y]es  [N]o"
4. y → start transfer (arrow turns yellow, progress shown)
5. n / Esc → cancel (arrow returns to gray ·)
```

**Download flow:**
```
1. User selects file in Remote panel, presses x
2. Arrow column shows green ←
3. Status bar: "Download config.txt ← remote?  [Y]es  [N]o"
4. y → start transfer
5. n / Esc → cancel
```

### State Machine Changes

In `handle_key()`:
- `x` key in `Mode::Normal`: if not tree mode, enter `Mode::TransferConfirm` with direction based on `active_side`
- `y`/`Y` in `Mode::TransferConfirm`: start transfer (call existing `upload_selected()`/`download_selected()`)
- `n`/`N`/`Esc` in `Mode::TransferConfirm`: return to `Mode::Normal`
- `d` key in `Mode::Normal`: if not tree mode, enter `Mode::DeleteConfirm` (existing logic, just key changed)

### Status Bar Rendering

In the status bar's right-side help section:
- `Mode::TransferConfirm`: `[Y]es  [N]o` (Y green bold, N red bold)
- Transfer direction text: `Upload {name} → remote?` or `Download {name} ← remote?`

In the status bar's left-side status section:
- `Mode::TransferConfirm`: show file name in Yellow bold + direction arrow in Green

### Mutual Exclusivity

`TransferConfirm` and `DeleteConfirm` are mutually exclusive — only one can be active at a time (enforced by the single `mode` field).

## Files Modified

| File | Changes |
|------|---------|
| `crates/minishell-tui/src/filebrowser.rs` | Layout, arrow rendering, key remapping, TransferConfirm state, status bar |

## Edge Cases

- Empty directory: `x` does nothing (no file to transfer) — same as current `u`/`d`
- Tree mode: `x` and `d` blocked — status bar shows tree mode help
- Transfer already in progress: `x` blocked (`self.pending.is_some()` check)
- Parent directory (`..`): `x` does nothing — same as current behavior
