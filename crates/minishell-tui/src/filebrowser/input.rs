use crossterm::event::{KeyCode, KeyEvent};
use super::FileBrowserState;

impl FileBrowserState {
    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.rename_input.is_some() {
            match key.code {
                KeyCode::Enter => self.confirm_rename(),
                KeyCode::Esc => {
                    self.rename_input = None;
                    self.status = "重命名已取消".to_string();
                }
                KeyCode::Backspace => {
                    if let Some(ref mut s) = self.rename_input {
                        s.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut s) = self.rename_input {
                        s.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        if self.confirm_delete.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_delete_action(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm_delete = None;
                    self.status = "删除已取消".to_string();
                }
                _ => {}
            }
            return;
        }

        if self.transfer_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_transfer(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.transfer_confirm = None;
                    self.status = "传输已取消".to_string();
                }
                _ => {}
            }
            return;
        }

        if self.switch_confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_switch(),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.switch_confirm = None;
                    self.status = "选择未变".to_string();
                }
                _ => {}
            }
            return;
        }

        if self.clipboard_panel_open {
            match key.code {
                KeyCode::Char('n') => {
                    self.clipboard_panel_next();
                    return;
                }
                KeyCode::Char('k') => {
                    self.clipboard_panel_remove();
                    return;
                }
                KeyCode::Char('v') | KeyCode::Esc => {
                    self.close_clipboard_panel();
                    return;
                }
                KeyCode::Char('p') => {
                    self.close_clipboard_panel();
                    self.paste_from_clipboard();
                    return;
                }
                KeyCode::Char('y') => {
                    return;
                }
                _ => {}
            }
        }

        if self.pending.is_some() {
            if key.code == KeyCode::Esc {
                self.cancel_transfer();
            }
            return;
        }

        match key.code {
            KeyCode::Up => self.move_cursor(-1, self.visible_rows),
            KeyCode::Down => self.move_cursor(1, self.visible_rows),
            KeyCode::PageUp => self.cursor_first(),
            KeyCode::PageDown => self.cursor_last(self.visible_rows),
            KeyCode::Right | KeyCode::Enter => self.enter_dir(),
            KeyCode::Left => self.collapse_or_navigate_tree(),
            KeyCode::Esc => self.parent_dir(),
            KeyCode::Tab => self.toggle_side(),
            KeyCode::Char('/') => self.goto_root(),
            KeyCode::Char('~') => self.goto_home(),
            KeyCode::Char('x') => {
                if self.clipboard.is_empty() {
                    self.start_transfer_confirm();
                }
            }
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('t') => self.toggle_tree(),
            KeyCode::Char('y') => self.yank_toggle(),
            KeyCode::Char('v') => self.open_clipboard_panel(),
            KeyCode::Char('p') => self.paste_from_clipboard(),
            _ => {}
        }
    }
}
