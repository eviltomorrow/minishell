pub mod app;
pub mod table;
pub mod form;
pub mod selector;
pub mod styles;
pub mod filebrowser;
pub mod probe_state;

use std::sync::Arc;
use minishell_core::Machine;
use minishell_store::Store;

pub fn run(store: Arc<Store>) -> anyhow::Result<()> {
    app::run(store)
}

pub fn select_machine(machines: Vec<Machine>) -> anyhow::Result<Option<Machine>> {
    selector::select_machine(machines)
}
