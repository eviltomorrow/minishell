# 剪贴板多选传输实施计划

## 目标

在文件浏览器中新增 `y`(标记) / `v`(选择面板) / `p`(批量粘贴) 剪贴板传输功能。

## 文件改动

仅 `crates/minishell-tui/src/filebrowser/mod.rs`

## 步骤

### 1. 新增数据结构

```rust
// 在 types.rs 或 mod.rs 中
pub struct ClipboardEntry {
    pub source_path: PathBuf,
    pub source_side: Side,
    pub name: String,
    pub is_dir: bool,
}
```

在 `FileBrowserState` 中新增字段：
```rust
clipboard: Vec<ClipboardEntry>,
clipboard_side: Option<Side>,
clipboard_panel_open: bool,
clipboard_panel_cursor: usize,
```

初始化为 `vec![]`, `None`, `false`, `0`。

### 2. 实现 `yank_toggle()`

- 获取当前条目（`current_entry()` + `current_entry_full_path()`）
- 如果是 `..` → 忽略
- 如果条目已在 `clipboard` 中 → 移除（取消标记）
- 如果条目不在 `clipboard` 中：
  - 如果 `clipboard_side` 是另一侧 → 弹出确认（用状态标记 `switch_confirm: Option<Side>`）
  - 如果同侧或空 → 添加到 `clipboard`，设置 `clipboard_side`

确认逻辑：
- 在 key handler 中，如果 `switch_confirm` 有值：
  - `y` / `Y` → 清空 `clipboard`，设置新 `clipboard_side`，添加条目
  - `n` / `N` / `Esc` → 取消

### 3. 实现 `open_clipboard_panel()` / `close_clipboard_panel()`

- `open_clipboard_panel()`: 设置 `clipboard_panel_open = true`，`clipboard_panel_cursor = 0`
- `close_clipboard_panel()`: 设置 `clipboard_panel_open = false`

### 4. 实现选择面板内的 key handler

在 `handle_key()` 中，如果 `clipboard_panel_open`：
- `↑` → `clipboard_panel_cursor -= 1`
- `↓` → `clipboard_panel_cursor += 1`
- `K` → 从 `clipboard` 移除当前条目，如果 `clipboard` 为空则设置 `clipboard_side = None`，关闭面板
- `v` / `Esc` → 关闭面板

### 5. 实现 `paste_from_clipboard()`

- 如果 `clipboard` 为空 → 显示 "No files selected"，返回
- 如果 `clipboard_side == active_side` → 忽略
- 如果远端未连接且需要 → 显示 "Not connected"，返回
- 遍历 `clipboard`，对每个条目：
  - 计算目标路径：`current_panel().current_path.join(entry.name)`
  - 根据 `clipboard_side` 决定方向（Local→Remote=upload, Remote→Local=download）
  - 启动传输（复用 `start_transfer` 基础设施）
- 清空 `clipboard` 和 `clipboard_side`

注意：批量传输需要串行执行（一次一个），因为 `self.pending` 是单个值。
方案：只传第一个文件，传输完成后自动传下一个（用队列 `transfer_queue: Vec<ClipboardEntry>`）。

### 6. 更新 `handle_key()`

```rust
// 新增状态
if self.switch_confirm.is_some() {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_switch(),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => self.switch_confirm = None,
        _ => {}
    }
    return;
}

if self.clipboard_panel_open {
    match key.code {
        KeyCode::Up => self.clipboard_panel_up(),
        KeyCode::Down => self.clipboard_panel_down(),
        KeyCode::Char('K') => self.clipboard_panel_remove(),
        KeyCode::Char('v') | KeyCode::Esc => self.close_clipboard_panel(),
        _ => {}
    }
    return;
}

// 在正常 key handler 中新增
KeyCode::Char('y') => self.yank_toggle(),
KeyCode::Char('v') => self.open_clipboard_panel(),
KeyCode::Char('p') => self.paste_from_clipboard(),
```

### 7. 更新渲染

**主列表标记：** 在 `render_panel()` 中，渲染每行时检查该条目是否在 `clipboard` 中，如果是则显示 `✓` 替代默认图标。

**选择面板：** 在 `render()` 中，如果 `clipboard_panel_open`，在面板区域上方渲染浮动面板。

浮动面板渲染：
- 计算居中位置
- 渲染边框 Block
- 渲染文件列表（名称、大小、来源侧 L/R）
- 当前行高亮
- 底部帮助文字

**状态栏：**
- `switch_confirm` 时：显示 "Switch to {side}? Clear selection [Y]es [N]o"
- `yank` 后：显示 "✓ Copied: {name} ({side})"

### 8. 更新 help bar

所有 help bar 模式新增 `y:copy v:select p:paste`。

### 9. 传输队列

为支持批量传输，新增：
```rust
transfer_queue: Vec<ClipboardEntry>,
```

`paste_from_clipboard()` 设置 `transfer_queue` 并调用 `process_next_transfer()`。
`process_next_transfer()` 从队列取出第一个文件并启动传输。
传输完成回调中（`handle_action_result`）调用 `process_next_transfer()` 直到队列为空。

### 10. cargo check + cargo test

## 边界情况处理

| 场景 | 代码位置 | 处理 |
|---|---|---|
| 空选择按 `p` | `paste_from_clipboard` | 状态栏提示 |
| 跨侧选择 | `yank_toggle` | `switch_confirm` 状态 |
| 传输进行中 | `paste_from_clipboard` | 检查 `self.pending` |
| `..` 条目 | `yank_toggle` | 忽略 |
| 远程未连接 | `yank_toggle` / `paste_from_clipboard` | 状态栏提示 |
| 面板为空 | `clipboard_panel_open` 渲染 | 显示 "(empty)" |
