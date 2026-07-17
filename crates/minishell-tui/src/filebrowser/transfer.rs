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
