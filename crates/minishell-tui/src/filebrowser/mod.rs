mod panel;
mod tree;
mod transfer;
mod delete;
mod rename;
mod render;

pub use panel::{Side, PanelState};
pub use tree::TreeEntry;

use minishell_core::Machine;
use crossterm::event::KeyEvent;
use ratatui::Frame;

pub struct FileBrowserState {
    // Will be populated in next tasks
}

impl FileBrowserState {
    pub fn new(machine: Machine) -> Self {
        todo!()
    }
    
    pub fn check_pending(&mut self) {
        todo!()
    }
    
    pub fn handle_key(&mut self, key: KeyEvent) {
        todo!()
    }
    
    pub fn render(&mut self, f: &mut Frame) {
        todo!()
    }
}
