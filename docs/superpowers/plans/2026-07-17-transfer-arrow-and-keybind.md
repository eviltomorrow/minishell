# Transfer Arrow Indicator & Key Remapping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a visual arrow column between Local/Remote panels showing transfer direction, remap keys (`x`=transfer, `d`=delete), and add status bar Yes/No confirmation for transfers.

**Architecture:** Single-file change in `filebrowser.rs`. Add `TransferConfirm` state variant, 3-column layout with arrow gutter, and remap key bindings. No new dependencies.

**Tech Stack:** Rust, ratatui, crossterm

## Global Constraints

- All changes in `crates/minishell-tui/src/filebrowser.rs` only
- Follow existing code patterns (state via `Option` fields, no new enums unless needed)
- Arrow gutter: 3 chars wide, fixed `Length(3)` constraint
- `u` key becomes dead (no binding)

---

### Task 1: Add `transfer_confirm` state field

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser.rs:83-100` (FileBrowserState struct)
- Modify: `crates/minishell-tui/src/filebrowser.rs:100-160` (new() constructor)

**Interfaces:**
- Produces: `self.transfer_confirm: Option<Side>` — `Some(Side::Local)` means upload pending, `Some(Side::Remote)` means download pending

- [ ] **Step 1: Add field to struct**

In `FileBrowserState` (line ~94), add after `confirm_delete`:

```rust
    transfer_confirm: Option<Side>,
```

- [ ] **Step 2: Initialize in constructor**

In `FileBrowserState::new()`, add after `confirm_delete: None,`:

```rust
            transfer_confirm: None,
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 4: Commit**

```bash
git add crates/minishell-tui/src/filebrowser.rs
git commit -m "feat(tui): add transfer_confirm state field"
```

---

### Task 2: Remap keys in `handle_key()`

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser.rs:1033-1100` (handle_key)

**Interfaces:**
- Consumes: `self.transfer_confirm: Option<Side>` from Task 1
- Produces: `x` enters `TransferConfirm`, `d` enters `DeleteConfirm`, `u` removed

- [ ] **Step 1: Add TransferConfirm handler block**

In `handle_key()`, after the `confirm_delete` block (line ~1068) and before the `pending` check (line ~1070), add:

```rust
        if self.transfer_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_transfer(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.transfer_confirm = None;
                    self.status = "Transfer cancelled".to_string();
                }
                _ => {}
            }
            return;
        }
```

- [ ] **Step 2: Update tree mode blocklist**

Change line 1076 from:
```rust
                KeyCode::Char('u') | KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Char('r') => {
```
to:
```rust
                KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Char('r') => {
```

- [ ] **Step 3: Remap key bindings**

In the `match key.code` block (lines 1083-1099), replace:
```rust
            KeyCode::Char('u') => self.upload_selected(),
            KeyCode::Char('d') => self.download_selected(),
            KeyCode::Char('x') => self.start_delete(),
```
with:
```rust
            KeyCode::Char('x') => self.start_transfer_confirm(),
            KeyCode::Char('d') => self.start_delete(),
```

- [ ] **Step 4: Add `start_transfer_confirm()` method**

Add this method after `download_selected()` (line ~869):

```rust
    fn start_transfer_confirm(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let side = self.active_side;
        let panel = self.active_panel();
        let cursor = panel.cursor;
        let entry = match panel.entries.get(cursor).cloned() {
            Some(e) => e,
            None => return,
        };
        // Block on parent dir entry
        if entry.name == ".." {
            return;
        }
        let filename = entry.name.rsplit('/').next().unwrap_or(&entry.name).to_string();
        let type_label = if entry.is_dir { "[DIR]" } else { "[FILE]" };
        let direction = match side {
            Side::Local => "→ remote",
            Side::Remote => "← local",
        };
        self.status = format!("Transfer {} {} {}?", type_label, filename, direction);
        self.transfer_confirm = Some(side);
    }
```

- [ ] **Step 5: Add `confirm_transfer()` method**

Add this method after `start_transfer_confirm()`:

```rust
    fn confirm_transfer(&mut self) {
        let side = self.transfer_confirm.take().unwrap_or(self.active_side);
        match side {
            Side::Local => self.upload_selected(),
            Side::Remote => self.download_selected(),
        }
    }
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 7: Commit**

```bash
git add crates/minishell-tui/src/filebrowser.rs
git commit -m "feat(tui): remap x=transfer, d=delete, add transfer confirm state"
```

---

### Task 3: Change layout to 3-column with arrow gutter

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser.rs:1139-1147` (panel layout in render)

**Interfaces:**
- Produces: `panels[0]` = local, `panels[1]` = arrow gutter (3 chars), `panels[2]` = remote

- [ ] **Step 1: Replace 2-column layout with 3-column**

Replace lines 1139-1143:
```rust
        // Split panels
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(chunks[1]);
```
with:
```rust
        // Split panels: local | arrow gutter | remote
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(10),
                Constraint::Length(3),
                Constraint::Min(10),
            ])
            .split(chunks[1]);
```

- [ ] **Step 2: Update panel render calls**

Replace lines 1145-1147:
```rust
        self.visible_rows = (chunks[1].height as usize).saturating_sub(4).max(1);
        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[1], Side::Remote);
```
with:
```rust
        self.visible_rows = (chunks[1].height as usize).saturating_sub(4).max(1);
        self.render_panel(f, panels[0], Side::Local);
        self.render_panel(f, panels[2], Side::Remote);
        self.render_arrow(f, panels[1]);
```

- [ ] **Step 3: Add `render_arrow()` method**

Add this method after `render_panel()` (around line 1508):

```rust
    fn render_arrow(&self, f: &mut Frame, area: Rect) {
        use ratatui::widgets::Clear;

        f.render_widget(Clear, area);

        if area.height < 1 || area.width < 3 {
            return;
        }

        let (symbol, color) = if self.pending.is_some() {
            // Transfer in progress
            match self.active_side {
                Side::Local => ("\u{2192}", Color::Yellow),   // →
                Side::Remote => ("\u{2190}", Color::Yellow),  // ←
            }
        } else if self.transfer_confirm.is_some() {
            // Awaiting confirmation
            match self.transfer_confirm.unwrap() {
                Side::Local => ("\u{2192}", Color::Green),    // → upload
                Side::Remote => ("\u{2190}", Color::Green),   // ← download
            }
        } else {
            ("\u{00B7}", Color::DarkGray)  // · idle
        };

        let y = area.y + area.height / 2;
        let x = area.x + 1; // center in 3-char width
        f.set_cursor_position((x, y));
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                symbol,
                Style::default().fg(color),
            ))),
            Rect { x, y, width: 1, height: 1 },
        );
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 5: Commit**

```bash
git add crates/minishell-tui/src/filebrowser.rs
git commit -m "feat(tui): 3-column layout with arrow gutter between panels"
```

---

### Task 4: Update status bar for transfer confirm

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser.rs:1149-1301` (status bar rendering)

**Interfaces:**
- Consumes: `self.transfer_confirm: Option<Side>` from Task 1

- [ ] **Step 1: Add TransferConfirm to status text rendering**

In the `status_text` block (lines 1169-1184), after the `rename_input` check and before the `pending` check, add:

```rust
            } else if self.transfer_confirm.is_some() {
                self.status.clone()
```

The full block should become:
```rust
            let status_text = if self.rename_input.is_some() {
                if let Some(ref input) = self.rename_input {
                    format!("{} {}", self.status, input)
                } else {
                    self.status.clone()
                }
            } else if self.transfer_confirm.is_some() {
                self.status.clone()
            } else if self.pending.is_some() && self.progress_total > 0 {
                // ... existing progress code ...
```

- [ ] **Step 2: Add TransferConfirm to left-side spans**

In the spans block (lines 1186-1220), after the `confirm_delete` rendering and before the `else` fallback, add a new branch. Replace the existing `} else {` at line 1218 with:

```rust
            } else if self.transfer_confirm.is_some() {
                // Parse status text for styled rendering
                if let Some(type_end) = self.status.find(']') {
                    let type_part = &self.status[..=type_end]; // "[DIR]" or "[FILE]"
                    let rest = &self.status[type_end + 1..]; // " filename → remote?"
                    spans.push(Span::styled(type_part, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
                    if let Some(dir_pos) = rest.find('\u{2192}').or_else(|| rest.find('\u{2190}')) {
                        let name = &rest[..dir_pos];
                        let direction = &rest[dir_pos..];
                        spans.push(Span::styled(name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
                        spans.push(Span::styled(direction, Style::default().fg(Color::Green)));
                    } else {
                        spans.push(Span::styled(rest, Style::default().fg(Color::White)));
                    }
                } else {
                    spans.push(Span::styled(&self.status, Style::default().fg(Color::Cyan)));
                }
            } else {
```

- [ ] **Step 3: Add TransferConfirm to right-side help hints**

In the `right_spans` block (lines 1224-1278), add a new branch after the `confirm_delete` check (line 1230). Replace `} else if self.rename_input.is_some() {` with:

```rust
            } else if self.transfer_confirm.is_some() {
                vec![
                    Span::styled("[Y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled("es  ", styles::help_style()),
                    Span::styled("[N]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled("o", styles::help_style()),
                ]
            } else if self.rename_input.is_some() {
```

- [ ] **Step 4: Update help text for normal mode**

In the normal mode help hints (lines 1255-1277), replace:
```rust
                    Span::styled("u", styles::key_style()),
                    Span::styled(":upload  ", styles::help_style()),
                    Span::styled("d", styles::key_style()),
                    Span::styled(":download  ", styles::help_style()),
                    Span::styled("x", styles::key_style()),
                    Span::styled(":del  ", styles::help_style()),
```
with:
```rust
                    Span::styled("x", styles::key_style()),
                    Span::styled(":transfer  ", styles::help_style()),
                    Span::styled("d", styles::key_style()),
                    Span::styled(":del  ", styles::help_style()),
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check`
Expected: compiles without errors

- [ ] **Step 6: Commit**

```bash
git add crates/minishell-tui/src/filebrowser.rs
git commit -m "feat(tui): status bar transfer confirm with Yes/No and styled direction"
```

---

### Task 5: Cleanup and final verification

**Files:**
- Modify: `crates/minishell-tui/src/filebrowser.rs` (any remaining issues)

- [ ] **Step 1: Run full build**

Run: `cargo build`
Expected: compiles without errors or warnings

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 3: Manual verification checklist**

Verify in the running TUI:
- Arrow shows `·` (gray) when idle
- Select file in Local panel, press `x` → arrow shows `→` (green), status bar shows "Transfer [FILE] name → remote?  [Y]es [N]o"
- Press `y` → transfer starts, arrow turns yellow
- Press `n` → cancelled, arrow returns to `·`
- Select file in Remote panel, press `x` → arrow shows `←` (green)
- Press `d` in Local panel → delete confirmation (same as old `x`)
- `u` key does nothing
- Tree mode blocks `x` and `d`

- [ ] **Step 4: Commit any fixes**

```bash
git add crates/minishell-tui/src/filebrowser.rs
git commit -m "fix(tui): cleanup transfer arrow implementation"
```
