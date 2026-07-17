use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use super::types::Side;

pub struct TransferProgressState {
    pub file_name: String,
    pub bytes: u64,
    pub total: u64,
}

pub enum ActionResult {
    TransferDone(Side),
    Error(String),
    Aborted,
}

pub struct PendingTransfer {
    pub progress: Arc<Mutex<TransferProgressState>>,
    pub done_rx: mpsc::Receiver<ActionResult>,
    pub cancel: Arc<AtomicBool>,
}

impl PendingTransfer {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}
